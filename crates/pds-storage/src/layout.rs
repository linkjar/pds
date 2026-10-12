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

    /// The 32 bytes of the actor's secp256k1 signing key, as the Reference
    /// wrote them.
    pub fn read_actor_key(&self, did: &str) -> Result<Vec<u8>, StorageError> {
        let path = self.actor_key(did);
        std::fs::read(&path).map_err(|source| StorageError::io(path, source))
    }
}

/// Opens a database for reading. A missing file is an error: reading never
/// creates one.
pub(crate) fn open_read_only(path: &Path) -> Result<Connection, StorageError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags)?;
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
