//! The seam below SQLite (`docs/SPEC.md` §19.2).
//!
//! Every database of the data directory is opened here. Production opens
//! it with the operating system's files. The simulation registers a
//! virtual file system that fails and loses writes, and names it with
//! [`use_vfs`]; from then on every database of the process goes through
//! it. An operator whose backup tool owns the checkpoints turns SQLite's
//! own off with [`disable_wal_auto_checkpoint`].

use std::path::Path;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{Connection, OpenFlags};

static VFS: RwLock<Option<String>> = RwLock::new(None);
static WAL_AUTO_CHECKPOINT: AtomicBool = AtomicBool::new(true);

/// Hands the checkpoints of every database this process opens from now on
/// to a backup tool such as Litestream.
///
/// This is what the Reference does with
/// `PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT` (SPEC §8.3): SQLite's automatic
/// checkpoint is off on each connection, and a WAL file grows until the
/// tool checkpoints it.
pub fn disable_wal_auto_checkpoint() {
    WAL_AUTO_CHECKPOINT.store(false, Ordering::SeqCst);
}

/// Applies the process-wide settings to a connection that was just opened.
fn configure(conn: &Connection, wal_auto_checkpoint: bool) -> rusqlite::Result<()> {
    if !wal_auto_checkpoint {
        conn.pragma_update(None, "wal_autocheckpoint", 0)?;
    }
    Ok(())
}

/// Opens every database of this process through the SQLite VFS `name`, or
/// through the default one with `None`. The VFS must be registered.
pub fn use_vfs(name: Option<&str>) {
    let mut vfs = VFS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *vfs = name.map(str::to_owned);
}

/// Opens the database at `path`.
pub(crate) fn open(path: &Path, flags: OpenFlags) -> rusqlite::Result<Connection> {
    let vfs = VFS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let conn = match vfs.as_deref() {
        Some(name) => Connection::open_with_flags_and_vfs(path, flags, name),
        None => Connection::open_with_flags(path, flags),
    }?;
    drop(vfs);
    configure(&conn, WAL_AUTO_CHECKPOINT.load(Ordering::SeqCst))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto_checkpoint_pages(conn: &Connection) -> i64 {
        conn.query_row("pragma wal_autocheckpoint", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_connection_keeps_sqlite_s_checkpoints_unless_they_are_handed_over() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn, true).unwrap();
        assert_eq!(auto_checkpoint_pages(&conn), 1000);
        configure(&conn, false).unwrap();
        assert_eq!(auto_checkpoint_pages(&conn), 0);
    }
}
