//! A SQLite virtual file system that loses power (`docs/SPEC.md` §19.2).
//!
//! The file system passes every operation to the files of the operating
//! system and remembers what a write replaced until the file is synced.
//! When the power is lost, each file keeps a prefix of its writes since
//! the last sync and loses the rest; the last write that is kept can be
//! torn at a sector boundary. Every operation after the loss fails, as for
//! a process that no longer runs.
//!
//! The write-ahead log's shared memory and all locks are in this process,
//! so the files must not be opened by another process, or through another
//! file system, at the same time.
//!
//! What it does not model: a write that lands at another offset, a bit
//! that flips, a full disk, and the loss of a deleted file's deletion. A
//! delete is durable at once.

// An operation holds the state for all of its work on the file, so that
// what the state says of a file is what the file holds.
#![allow(clippy::significant_drop_tightening)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::FileExt as _;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use sqlite_plugin::flags::{AccessFlags, CreateMode, LockLevel, OpenMode, OpenOpts, ShmLockMode};
use sqlite_plugin::vars;
use sqlite_plugin::vfs::{RegisterOpts, Vfs, VfsHandle, VfsResult, register_static};

/// The name the file system is registered under.
pub const NAME: &str = "pds-sim";

/// The sector size a torn write is cut at.
const SECTOR: usize = 512;
const SECTOR_I32: i32 = 512;

/// The slots of the write-ahead log's lock table.
const SHM_SLOTS: usize = 8;

/// What a write or a truncation replaced.
#[derive(Debug)]
struct Undo {
    offset: u64,
    old: Vec<u8>,
    old_len: u64,
}

#[derive(Debug, Default)]
struct Slot {
    shared: Vec<u64>,
    exclusive: Option<u64>,
}

/// What the handles of one file share.
#[derive(Debug, Default)]
struct Shared {
    unsynced: Vec<Undo>,
    regions: Vec<Box<[u8]>>,
    mapped: BTreeSet<u64>,
    slots: [Slot; SHM_SLOTS],
    readers: BTreeSet<u64>,
    reserved: Option<u64>,
    pending: Option<u64>,
    exclusive: Option<u64>,
}

#[derive(Debug, Default)]
struct State {
    files: BTreeMap<String, Shared>,
    /// Handles of an earlier epoch belong to a process that lost power.
    epoch: u64,
    dead: bool,
    /// Writes, truncations and syncs until the power is lost.
    countdown: Option<u64>,
    next_handle: u64,
    /// Shared memory that a handle of a dead process may still map.
    graveyard: Vec<Box<[u8]>>,
    operations: u64,
}

/// What a power loss cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lost {
    /// Files that had writes since their last sync.
    pub files: usize,
    /// Writes and truncations that were not kept.
    pub writes: usize,
    /// Whether a kept write was torn.
    pub torn: bool,
}

/// The file system.
#[derive(Debug, Clone, Default)]
pub struct SimVfs {
    state: Arc<Mutex<State>>,
}

/// An open file.
#[derive(Debug)]
pub struct Handle {
    id: u64,
    epoch: u64,
    path: Option<String>,
    file: File,
    readonly: bool,
    delete_on_close: bool,
}

impl VfsHandle for Handle {
    fn readonly(&self) -> bool {
        self.readonly
    }

    fn in_memory(&self) -> bool {
        false
    }
}

fn revert(path: &str, undo: &[Undo]) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    for step in undo.iter().rev() {
        file.set_len(step.old_len)?;
        file.write_all_at(&step.old, step.offset)?;
    }
    Ok(())
}

impl SimVfs {
    /// Registers the file system with SQLite, once for the process, and
    /// makes every database of `pds-storage` open through it.
    pub fn install() -> Result<&'static Self, String> {
        static INSTALLED: OnceLock<Result<SimVfs, String>> = OnceLock::new();
        let installed = INSTALLED.get_or_init(|| {
            let vfs = Self::default();
            let name = CString::new(NAME).map_err(|error| error.to_string())?;
            register_static(
                name,
                vfs.clone(),
                RegisterOpts {
                    make_default: false,
                },
            )
            .map_err(|code| format!("sqlite3_vfs_register answered {code}"))?;
            pds_storage::io::use_vfs(Some(NAME));
            Ok(vfs)
        });
        installed.as_ref().map_err(Clone::clone)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Loses the power at the write, truncation or sync that comes after
    /// `operations` more of them.
    pub fn arm(&self, operations: u64) {
        self.state().countdown = Some(operations);
    }

    /// Withdraws [`SimVfs::arm`]. Returns whether the power is lost.
    pub fn disarm(&self) -> bool {
        let mut state = self.state();
        state.countdown = None;
        state.dead
    }

    /// How many writes, truncations and syncs the file system has seen.
    pub fn operations(&self) -> u64 {
        self.state().operations
    }

    /// Forgets everything: for a new data directory. The handles of the
    /// directory before must be closed.
    pub fn reset(&self) {
        let mut state = self.state();
        let graveyard = std::mem::take(&mut state.graveyard);
        *state = State {
            epoch: state.epoch + 1,
            graveyard,
            ..State::default()
        };
    }

    /// Applies a power loss and lets a new process open the files.
    /// `choose(n)` answers a number below `n`.
    pub fn power_on(&self, mut choose: impl FnMut(u64) -> u64) -> std::io::Result<Lost> {
        let mut state = self.state();
        let mut lost = Lost::default();
        let files = std::mem::take(&mut state.files);
        for (path, mut shared) in files {
            // A handle of the process before may still map the memory.
            state.graveyard.append(&mut shared.regions);
            if shared.unsynced.is_empty() {
                continue;
            }
            lost.files += 1;
            let writes = shared.unsynced.len();
            let kept = usize::try_from(choose(writes as u64 + 1)).unwrap_or(writes);
            lost.writes += writes - kept;
            let dropped = shared.unsynced.split_off(kept);
            revert(&path, &dropped)?;
            // The last write that is kept may have reached the disk in
            // part: the sectors after the cut hold what was there before.
            if let Some(last) = shared.unsynced.last()
                && last.old.len() > SECTOR
                && choose(4) == 0
            {
                let sectors = last.old.len() / SECTOR;
                let cut = usize::try_from(choose(sectors as u64)).unwrap_or(0) * SECTOR + SECTOR;
                if let Some(tail) = last.old.get(cut..) {
                    let file = OpenOptions::new().write(true).open(&path)?;
                    file.write_all_at(tail, last.offset + cut as u64)?;
                    lost.torn = true;
                }
            }
        }
        state.epoch += 1;
        state.dead = false;
        state.countdown = None;
        Ok(lost)
    }

    /// Counts one write, truncation or sync. An error is the power loss.
    fn tick(state: &mut State) -> VfsResult<()> {
        state.operations += 1;
        match state.countdown {
            Some(0) => {
                state.dead = true;
                state.countdown = None;
                Err(vars::SQLITE_IOERR)
            }
            Some(left) => {
                state.countdown = Some(left - 1);
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// The state, when `handle` belongs to the process that runs.
    fn live(&self, handle: &Handle) -> VfsResult<MutexGuard<'_, State>> {
        let state = self.state();
        if state.dead || handle.epoch != state.epoch {
            return Err(vars::SQLITE_IOERR);
        }
        Ok(state)
    }

    fn remember(state: &mut State, handle: &Handle, offset: u64, len: u64) -> VfsResult<()> {
        let Some(path) = &handle.path else {
            return Ok(());
        };
        let old_len = handle
            .file
            .metadata()
            .map_err(|_| vars::SQLITE_IOERR_FSTAT)?
            .len();
        let end = (offset + len).min(old_len);
        let mut old = vec![0; usize::try_from(end.saturating_sub(offset)).unwrap_or(0)];
        handle
            .file
            .read_exact_at(&mut old, offset)
            .map_err(|_| vars::SQLITE_IOERR_READ)?;
        state
            .files
            .entry(path.clone())
            .or_default()
            .unsynced
            .push(Undo {
                offset,
                old,
                old_len,
            });
        Ok(())
    }

    /// Releases what `id` holds on the file at `path`.
    fn release(state: &mut State, path: Option<&String>, id: u64, shm_only: bool) {
        let Some(shared) = path.and_then(|path| state.files.get_mut(path)) else {
            return;
        };
        for slot in &mut shared.slots {
            slot.shared.retain(|holder| *holder != id);
            if slot.exclusive == Some(id) {
                slot.exclusive = None;
            }
        }
        shared.mapped.remove(&id);
        if shm_only {
            return;
        }
        shared.readers.remove(&id);
        for held in [
            &mut shared.reserved,
            &mut shared.pending,
            &mut shared.exclusive,
        ] {
            if *held == Some(id) {
                *held = None;
            }
        }
    }
}

impl Vfs for SimVfs {
    type Handle = Handle;

    fn open(&self, path: Option<&str>, opts: OpenOpts) -> VfsResult<Handle> {
        let mut state = self.state();
        if state.dead {
            return Err(vars::SQLITE_CANTOPEN);
        }
        let mut options = OpenOptions::new();
        options.read(true);
        let readonly = match opts.mode() {
            OpenMode::ReadOnly => true,
            OpenMode::ReadWrite { create } => {
                options.write(true);
                match create {
                    CreateMode::None => {}
                    CreateMode::Create => {
                        options.create(true);
                    }
                    CreateMode::MustCreate => {
                        options.create_new(true);
                    }
                }
                false
            }
        };
        state.next_handle += 1;
        let id = state.next_handle;
        // A file without a name is SQLite's own scratch space.
        let (file, path, delete_on_close) = if let Some(path) = path {
            let file = options.open(path).map_err(|_| vars::SQLITE_CANTOPEN)?;
            (file, Some(path.to_owned()), opts.delete_on_close())
        } else {
            let name =
                std::env::temp_dir().join(format!("pds-sim-scratch-{}-{id}", std::process::id()));
            let file = options
                .write(true)
                .create(true)
                .truncate(true)
                .open(&name)
                .map_err(|_| vars::SQLITE_CANTOPEN)?;
            let _ = std::fs::remove_file(&name);
            (file, None, false)
        };
        Ok(Handle {
            id,
            epoch: state.epoch,
            path,
            file,
            readonly,
            delete_on_close,
        })
    }

    fn delete(&self, path: &str) -> VfsResult<()> {
        let mut state = self.state();
        // A process without power deletes nothing.
        if state.dead {
            return Ok(());
        }
        if let Some(shared) = state.files.get_mut(path) {
            shared.unsynced.clear();
        }
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(vars::SQLITE_IOERR_DELETE_NOENT)
            }
            Err(_) => Err(vars::SQLITE_IOERR_DELETE),
        }
    }

    fn access(&self, path: &str, _flags: AccessFlags) -> VfsResult<bool> {
        Ok(std::path::Path::new(path).exists())
    }

    fn file_size(&self, handle: &mut Handle) -> VfsResult<usize> {
        let _state = self.live(handle)?;
        let len = handle
            .file
            .metadata()
            .map_err(|_| vars::SQLITE_IOERR_FSTAT)?
            .len();
        usize::try_from(len).map_err(|_| vars::SQLITE_IOERR_FSTAT)
    }

    fn truncate(&self, handle: &mut Handle, size: usize) -> VfsResult<()> {
        let mut state = self.live(handle)?;
        Self::tick(&mut state)?;
        let size = size as u64;
        let len = handle
            .file
            .metadata()
            .map_err(|_| vars::SQLITE_IOERR_FSTAT)?
            .len();
        Self::remember(&mut state, handle, size.min(len), len.saturating_sub(size))?;
        handle
            .file
            .set_len(size)
            .map_err(|_| vars::SQLITE_IOERR_TRUNCATE)
    }

    fn write(&self, handle: &mut Handle, offset: usize, data: &[u8]) -> VfsResult<usize> {
        let mut state = self.live(handle)?;
        Self::tick(&mut state)?;
        Self::remember(&mut state, handle, offset as u64, data.len() as u64)?;
        handle
            .file
            .write_all_at(data, offset as u64)
            .map_err(|_| vars::SQLITE_IOERR_WRITE)?;
        Ok(data.len())
    }

    fn read(&self, handle: &mut Handle, offset: usize, data: &mut [u8]) -> VfsResult<usize> {
        let _state = self.live(handle)?;
        let mut done = 0;
        while done < data.len() {
            let Some(rest) = data.get_mut(done..) else {
                break;
            };
            match handle.file.read_at(rest, (offset + done) as u64) {
                Ok(0) => break,
                Ok(read) => done += read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Err(vars::SQLITE_IOERR_READ),
            }
        }
        Ok(done)
    }

    fn lock(&self, handle: &mut Handle, level: LockLevel) -> VfsResult<()> {
        let mut state = self.live(handle)?;
        let id = handle.id;
        let Some(path) = &handle.path else {
            return Ok(());
        };
        let shared = state.files.entry(path.clone()).or_default();
        let other = |held: Option<u64>| held.is_some_and(|holder| holder != id);
        match level {
            LockLevel::Unlocked => {}
            LockLevel::Shared => {
                if other(shared.pending) || other(shared.exclusive) {
                    return Err(vars::SQLITE_BUSY);
                }
                shared.readers.insert(id);
            }
            LockLevel::Reserved => {
                if other(shared.reserved) || other(shared.pending) || other(shared.exclusive) {
                    return Err(vars::SQLITE_BUSY);
                }
                shared.reserved = Some(id);
            }
            LockLevel::Pending | LockLevel::Exclusive => {
                if other(shared.reserved) || other(shared.pending) || other(shared.exclusive) {
                    return Err(vars::SQLITE_BUSY);
                }
                shared.pending = Some(id);
                if matches!(level, LockLevel::Exclusive) {
                    if shared.readers.iter().any(|reader| *reader != id) {
                        return Err(vars::SQLITE_BUSY);
                    }
                    shared.exclusive = Some(id);
                }
            }
        }
        Ok(())
    }

    fn unlock(&self, handle: &mut Handle, level: LockLevel) -> VfsResult<()> {
        let mut state = self.state();
        if handle.epoch != state.epoch {
            return Ok(());
        }
        let id = handle.id;
        let Some(shared) = handle
            .path
            .as_ref()
            .and_then(|path| state.files.get_mut(path))
        else {
            return Ok(());
        };
        for held in [
            &mut shared.reserved,
            &mut shared.pending,
            &mut shared.exclusive,
        ] {
            if *held == Some(id) {
                *held = None;
            }
        }
        if matches!(level, LockLevel::Unlocked) {
            shared.readers.remove(&id);
        }
        Ok(())
    }

    fn check_reserved_lock(&self, handle: &mut Handle) -> VfsResult<bool> {
        let state = self.live(handle)?;
        Ok(handle
            .path
            .as_ref()
            .and_then(|path| state.files.get(path))
            .is_some_and(|shared| shared.reserved.is_some() || shared.exclusive.is_some()))
    }

    fn sync(&self, handle: &mut Handle) -> VfsResult<()> {
        let mut state = self.live(handle)?;
        Self::tick(&mut state).map_err(|_| vars::SQLITE_IOERR_FSYNC)?;
        // The disk of the host is not asked: what a sync makes durable is
        // decided here, by what a power loss may take back.
        if let Some(shared) = handle
            .path
            .as_ref()
            .and_then(|path| state.files.get_mut(path))
        {
            shared.unsynced.clear();
        }
        Ok(())
    }

    /// The sector a write can be torn at.
    fn sector_size(&self, _handle: &mut Handle) -> VfsResult<i32> {
        Ok(SECTOR_I32)
    }

    /// A write is not atomic and an append is not safe. As on an ordinary
    /// disk, a torn write changes no byte outside its own range.
    fn device_characteristics(&self, _handle: &mut Handle) -> VfsResult<i32> {
        Ok(vars::SQLITE_IOCAP_POWERSAFE_OVERWRITE)
    }

    fn close(&self, handle: Handle) -> VfsResult<()> {
        let mut state = self.state();
        if handle.epoch != state.epoch {
            return Ok(());
        }
        Self::release(&mut state, handle.path.as_ref(), handle.id, false);
        if handle.delete_on_close
            && !state.dead
            && let Some(path) = &handle.path
        {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    fn shm_map(
        &self,
        handle: &mut Handle,
        region_idx: usize,
        region_size: usize,
        extend: bool,
    ) -> VfsResult<Option<NonNull<u8>>> {
        let mut state = self.live(handle)?;
        let Some(path) = &handle.path else {
            return Err(vars::SQLITE_IOERR_SHMMAP);
        };
        let shared = state.files.entry(path.clone()).or_default();
        shared.mapped.insert(handle.id);
        while shared.regions.len() <= region_idx {
            if !extend {
                return Ok(None);
            }
            shared
                .regions
                .push(vec![0_u8; region_size].into_boxed_slice());
        }
        // A region is a box of its own, so a later region moves no earlier
        // one. It lives until no handle maps the file, or for ever when a
        // process lost power with the file mapped.
        Ok(shared
            .regions
            .get_mut(region_idx)
            .and_then(|region| NonNull::new(region.as_mut_ptr())))
    }

    fn shm_lock(
        &self,
        handle: &mut Handle,
        offset: u32,
        count: u32,
        mode: ShmLockMode,
    ) -> VfsResult<()> {
        let unlocks = matches!(
            mode,
            ShmLockMode::UnlockShared | ShmLockMode::UnlockExclusive
        );
        let mut state = self.state();
        if handle.epoch != state.epoch {
            return if unlocks {
                Ok(())
            } else {
                Err(vars::SQLITE_IOERR)
            };
        }
        if state.dead && !unlocks {
            return Err(vars::SQLITE_IOERR);
        }
        let id = handle.id;
        let Some(path) = &handle.path else {
            return Err(vars::SQLITE_IOERR_SHMLOCK);
        };
        let shared = state.files.entry(path.clone()).or_default();
        let from = offset as usize;
        let Some(slots) = shared.slots.get_mut(from..from + count as usize) else {
            return Err(vars::SQLITE_IOERR_SHMLOCK);
        };
        match mode {
            ShmLockMode::LockShared => {
                if slots.iter().any(|slot| slot.exclusive.is_some()) {
                    return Err(vars::SQLITE_BUSY);
                }
                for slot in slots {
                    slot.shared.push(id);
                }
            }
            ShmLockMode::LockExclusive => {
                if slots
                    .iter()
                    .any(|slot| slot.exclusive.is_some() || !slot.shared.is_empty())
                {
                    return Err(vars::SQLITE_BUSY);
                }
                for slot in slots {
                    slot.exclusive = Some(id);
                }
            }
            ShmLockMode::UnlockShared => {
                for slot in slots {
                    if let Some(at) = slot.shared.iter().position(|holder| *holder == id) {
                        slot.shared.swap_remove(at);
                    }
                }
            }
            ShmLockMode::UnlockExclusive => {
                for slot in slots {
                    if slot.exclusive == Some(id) {
                        slot.exclusive = None;
                    }
                }
            }
        }
        Ok(())
    }

    fn shm_unmap(&self, handle: &mut Handle, delete: bool) -> VfsResult<()> {
        let mut state = self.state();
        if handle.epoch != state.epoch {
            return Ok(());
        }
        Self::release(&mut state, handle.path.as_ref(), handle.id, true);
        if delete
            && let Some(shared) = handle
                .path
                .as_ref()
                .and_then(|path| state.files.get_mut(path))
            && shared.mapped.is_empty()
        {
            shared.regions.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, OpenFlags};

    use super::*;

    fn open(path: &std::path::Path) -> Connection {
        Connection::open_with_flags_and_vfs(path, OpenFlags::default(), NAME).unwrap()
    }

    fn rows(db: &Connection) -> i64 {
        db.query_row("select count(*) from t", [], |row| row.get(0))
            .unwrap()
    }

    /// One test, because the file system is one for the process.
    #[test]
    fn a_power_loss_keeps_what_was_committed_and_two_connections_share_a_log() {
        let vfs = SimVfs::install().unwrap();
        vfs.reset();
        let dir = std::env::temp_dir().join(format!("pds-sim-vfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite");

        let writer = open(&path);
        let mode: String = writer
            .query_row("pragma journal_mode=wal", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        writer
            .execute_batch("create table t(a integer primary key, b blob)")
            .unwrap();
        // A second connection reads through the shared memory of the log.
        let reader = open(&path);
        for n in 0..20 {
            writer
                .execute("insert into t(b) values (zeroblob(3000))", [])
                .unwrap();
            assert_eq!(rows(&reader), n + 1);
        }
        // A reader holds its snapshot while the writer commits.
        reader
            .execute_batch("begin; select count(*) from t")
            .unwrap();
        writer
            .execute("insert into t(b) values (zeroblob(10))", [])
            .unwrap();
        assert_eq!(rows(&reader), 20);
        reader.execute_batch("commit").unwrap();
        assert_eq!(rows(&reader), 21);

        // The power goes inside a transaction of many pages.
        let before = vfs.operations();
        vfs.arm(6);
        let failed = writer.execute_batch(
            "begin; insert into t(b) select zeroblob(3000) from t; insert into t(b) select zeroblob(3000) from t; commit",
        );
        assert!(failed.is_err());
        assert!(vfs.disarm());
        assert!(vfs.operations() > before);
        // The process is gone: nothing more reaches the files.
        assert!(
            writer
                .execute("insert into t(b) values (x'00')", [])
                .is_err()
        );
        assert!(Connection::open_with_flags_and_vfs(&path, OpenFlags::default(), NAME).is_err());

        let mut draws = 0_u64;
        let lost = vfs
            .power_on(|below| {
                draws += 1;
                (draws * 7) % below
            })
            .unwrap();
        assert!(lost.files >= 1, "{lost:?}");
        // The connections of the process before stay dead.
        assert!(reader.query_row("select 1", [], |_| Ok(())).is_ok());
        assert!(
            reader
                .query_row("select count(*) from t", [], |_| Ok(()))
                .is_err()
        );
        let db = open(&path);
        let check: String = db
            .query_row("pragma integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(check, "ok");
        assert_eq!(rows(&db), 21);
        db.execute("insert into t(b) values (x'01')", []).unwrap();
        assert_eq!(rows(&db), 22);
        drop((db, reader, writer));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
