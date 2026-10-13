//! Where the files of a data directory are (`docs/SPEC.md` §8.1).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};

use crate::error::StorageError;

/// The paths of one data directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLayout {
    /// `account.sqlite`.
    pub account_db: PathBuf,
    /// `sequencer.sqlite`.
    pub sequencer_db: PathBuf,
    /// `did_cache.sqlite`.
    pub did_cache_db: PathBuf,
    /// `ops.sqlite`, which only the Candidate keeps.
    pub ops_db: PathBuf,
    /// The directory of the actor stores.
    pub actor_dir: PathBuf,
}

impl DataLayout {
    /// The Reference's default layout under `data_dir`.
    pub fn new(data_dir: &Path) -> Self {
        Self {
            account_db: data_dir.join("account.sqlite"),
            sequencer_db: data_dir.join("sequencer.sqlite"),
            did_cache_db: data_dir.join("did_cache.sqlite"),
            ops_db: data_dir.join("ops.sqlite"),
            actor_dir: data_dir.join("actors"),
        }
    }

    /// The directory of one actor: `<first two hex of sha256(did)>/<did>`.
    pub fn actor_store_dir(&self, did: &str) -> PathBuf {
        let digest = Sha256::digest(did.as_bytes());
        let shard = data_encoding::HEXLOWER.encode(&digest[..1]);
        self.actor_dir.join(shard).join(did)
    }

    /// The actor's `store.sqlite`.
    pub fn actor_db(&self, did: &str) -> PathBuf {
        self.actor_store_dir(did).join("store.sqlite")
    }

    /// The actor's signing key file.
    pub fn actor_key(&self, did: &str) -> PathBuf {
        self.actor_store_dir(did).join("key")
    }

    /// The file of a signing key that is reserved for an account that
    /// will come: `reserved_keys/<name>`, where the name is the DID of the
    /// account, or the `did:key` of the key when no DID was given.
    pub fn reserved_key(&self, name: &str) -> PathBuf {
        self.actor_dir.join("reserved_keys").join(name)
    }

    /// The 32 bytes of the actor's secp256k1 signing key, as the Reference
    /// wrote them.
    pub fn read_actor_key(&self, did: &str) -> Result<Vec<u8>, StorageError> {
        let path = self.actor_key(did);
        std::fs::read(&path).map_err(|source| StorageError::io(path, source))
    }
}

/// Opens a database for reading. A missing file is an error: reading never
/// creates a database.
///
/// The connection changes no row. It can still write beside the database:
/// when the database is in WAL journal mode, SQLite creates the `-shm` and
/// `-wal` files that are absent and leaves them when the connection closes.
/// The user that runs this process owns the new files, and needs write
/// access to the directory of the database: without it the read fails.
/// `tests/wal.rs` shows this, and shows that the connection reads what a
/// `-wal` file holds.
///
/// The `immutable` URI parameter would avoid the files, but SQLite then
/// ignores a `-wal` file, and a copy of a live database can carry one.
pub(crate) fn open_read_only(path: &Path) -> Result<Connection, StorageError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = crate::io::open(path, flags)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

/// Opens a database that exists for reading and writing.
///
/// The Reference sets no pragma besides the journal mode, which the file
/// keeps: `synchronous` stays at SQLite's default, so a commit is synced
/// (SPEC §8.3). It handles a busy database itself; here SQLite waits.
pub(crate) fn open_read_write(path: &Path) -> Result<Connection, StorageError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = crate::io::open(path, flags)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_directory_is_sharded_by_the_hash_of_the_did() {
        let layout = DataLayout::new(Path::new("/data"));
        // sha256("did:plc:dndoklorqricibwctfekw4zx") starts with 06, as a
        // Reference data directory shows.
        assert_eq!(
            layout.actor_db("did:plc:dndoklorqricibwctfekw4zx"),
            Path::new("/data/actors/06/did:plc:dndoklorqricibwctfekw4zx/store.sqlite")
        );
    }
}
