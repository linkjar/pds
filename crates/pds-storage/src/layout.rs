//! Where the files of a data directory are (`docs/SPEC.md` §8.1).

use std::path::{Path, PathBuf};

use std::sync::Arc;

use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};

use crate::account::AccountDb;
use crate::audit::AuditDb;
use crate::did_cache::DidCacheDb;
use crate::error::StorageError;
use crate::pg::PgTarget;
use crate::sequencer::SequencerDb;
use crate::sql::PgPool;

/// One of the databases of the shared tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedDb {
    /// Accounts, sessions and OAuth state.
    Account,
    /// The event log.
    Sequencer,
    /// DID documents.
    DidCache,
    /// The audit log.
    Audit,
}

impl SharedDb {
    /// The four, in the order a server creates them.
    pub const ALL: [Self; 4] = [Self::Account, Self::Sequencer, Self::DidCache, Self::Audit];

    /// The name of the database, which is the end of its schema name on
    /// Postgres.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Sequencer => "sequencer",
            Self::DidCache => "did_cache",
            Self::Audit => "audit",
        }
    }
}

/// The shared tier on Postgres (SPEC §21.2): one schema for each database
/// that the data directory holds as a file.
#[derive(Debug, Clone)]
pub struct SharedTier {
    target: PgTarget,
    prefix: String,
    pools: [Arc<PgPool>; 4],
}

impl PartialEq for SharedTier {
    fn eq(&self, other: &Self) -> bool {
        self.target == other.target && self.prefix == other.prefix
    }
}

impl Eq for SharedTier {}

/// The session lock of the one process that appends to the event log. It
/// is held until this value is dropped.
#[derive(Debug)]
pub struct SequencerLease(#[allow(dead_code)] crate::pg::PgClient);

impl SharedTier {
    /// The tier in the database at `target`. Each schema is named
    /// `<prefix>_<database>`.
    pub fn new(target: PgTarget, prefix: &str) -> Self {
        let pools =
            SharedDb::ALL.map(|db| PgPool::new(target.clone(), &format!("{prefix}_{}", db.name())));
        Self {
            target,
            prefix: prefix.to_owned(),
            pools,
        }
    }

    /// The pool of one database.
    pub fn pool(&self, db: SharedDb) -> &Arc<PgPool> {
        &self.pools[db as usize]
    }

    /// Takes the lease of the event log, or `None` when another process
    /// holds it.
    pub fn sequencer_lease(&self) -> Result<Option<SequencerLease>, StorageError> {
        let mut client =
            crate::pg::PgClient::connect(&self.target, std::time::Duration::from_secs(30))?;
        let key = format!("{}:sequencer", self.prefix);
        let (_, rows) = client.query(
            "select pg_try_advisory_lock(hashtext($1))",
            &[crate::sql::Value::Text(key)],
        )?;
        let held = rows.first().and_then(|row| row.first()) == Some(&crate::sql::Value::Integer(1));
        Ok(held.then_some(SequencerLease(client)))
    }

    /// Removes the four schemas with everything in them.
    pub fn drop_schemas(&self) -> Result<(), StorageError> {
        let mut client =
            crate::pg::PgClient::connect(&self.target, std::time::Duration::from_secs(30))?;
        for pool in &self.pools {
            client.batch(&format!(
                r#"drop schema if exists "{}" cascade"#,
                pool.schema()
            ))?;
        }
        Ok(())
    }
}

/// The paths of one data directory, and the place of its shared tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLayout {
    /// The shared tier on Postgres. `None`: the four databases are the
    /// SQLite files below.
    pub shared: Option<SharedTier>,
    /// `account.sqlite`.
    pub account_db: PathBuf,
    /// `sequencer.sqlite`.
    pub sequencer_db: PathBuf,
    /// `did_cache.sqlite`.
    pub did_cache_db: PathBuf,
    /// `ops.sqlite`, which only the Candidate keeps.
    pub ops_db: PathBuf,
    /// `audit.sqlite`, which only the Candidate keeps (SPEC §20).
    pub audit_db: PathBuf,
    /// The directory of the actor stores.
    pub actor_dir: PathBuf,
}

impl DataLayout {
    /// The Reference's default layout under `data_dir`.
    pub fn new(data_dir: &Path) -> Self {
        Self {
            shared: None,
            account_db: data_dir.join("account.sqlite"),
            sequencer_db: data_dir.join("sequencer.sqlite"),
            did_cache_db: data_dir.join("did_cache.sqlite"),
            ops_db: data_dir.join("ops.sqlite"),
            audit_db: data_dir.join("audit.sqlite"),
            actor_dir: data_dir.join("actors"),
        }
    }

    /// A connection to one database of the shared tier. On SQLite the
    /// file must exist, and `writable` says how it is opened.
    pub fn connect(
        &self,
        db: SharedDb,
        writable: bool,
    ) -> Result<crate::sql::Connection, StorageError> {
        if let Some(tier) = &self.shared {
            return Ok(tier.pool(db).connect()?);
        }
        let path = match db {
            SharedDb::Account => &self.account_db,
            SharedDb::Sequencer => &self.sequencer_db,
            SharedDb::DidCache => &self.did_cache_db,
            SharedDb::Audit => &self.audit_db,
        };
        let conn = if writable {
            open_read_write(path)?
        } else {
            open_read_only(path)?
        };
        Ok(conn.into())
    }

    /// A read connection to the accounts.
    pub fn accounts(&self) -> Result<AccountDb, StorageError> {
        match &self.shared {
            Some(tier) => Ok(AccountDb::over(tier.pool(SharedDb::Account).connect()?)),
            None => AccountDb::open(&self.account_db),
        }
    }

    /// A connection that reads and writes the accounts.
    pub fn accounts_writable(&self) -> Result<AccountDb, StorageError> {
        match &self.shared {
            Some(tier) => Ok(AccountDb::over(tier.pool(SharedDb::Account).connect()?)),
            None => AccountDb::open_writable(&self.account_db),
        }
    }

    /// A read connection to the event log.
    pub fn sequencer(&self) -> Result<SequencerDb, StorageError> {
        match &self.shared {
            Some(tier) => Ok(SequencerDb::over(tier.pool(SharedDb::Sequencer).connect()?)),
            None => SequencerDb::open(&self.sequencer_db),
        }
    }

    /// A connection that reads and writes the event log.
    pub fn sequencer_writable(&self) -> Result<SequencerDb, StorageError> {
        match &self.shared {
            Some(tier) => Ok(SequencerDb::over(tier.pool(SharedDb::Sequencer).connect()?)),
            None => SequencerDb::open_writable(&self.sequencer_db),
        }
    }

    /// A connection to the cache of DID documents.
    pub fn did_cache(&self) -> Result<DidCacheDb, StorageError> {
        match &self.shared {
            Some(tier) => Ok(DidCacheDb::over(tier.pool(SharedDb::DidCache).connect()?)),
            None => DidCacheDb::open(&self.did_cache_db),
        }
    }

    /// A connection to the audit log. The log is created when it is not
    /// there.
    pub fn audit(&self) -> Result<AuditDb, StorageError> {
        match &self.shared {
            Some(tier) => {
                let pool = tier.pool(SharedDb::Audit);
                pool.create_schema()?;
                AuditDb::over(pool.connect()?)
            }
            None => AuditDb::open(&self.audit_db),
        }
    }

    /// Whether one database of the shared tier exists.
    pub fn exists(&self, db: SharedDb) -> Result<bool, StorageError> {
        match &self.shared {
            Some(tier) => Ok(tier.pool(db).is_created()?),
            None => Ok(match db {
                SharedDb::Account => &self.account_db,
                SharedDb::Sequencer => &self.sequencer_db,
                SharedDb::DidCache => &self.did_cache_db,
                SharedDb::Audit => &self.audit_db,
            }
            .exists()),
        }
    }

    /// Whether the accounts, the event log and the cache exist.
    pub fn has_shared_databases(&self) -> Result<bool, StorageError> {
        match &self.shared {
            Some(tier) => {
                for db in [SharedDb::Account, SharedDb::Sequencer, SharedDb::DidCache] {
                    if !tier.pool(db).is_created()? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            None => Ok(self.account_db.exists()
                && self.sequencer_db.exists()
                && self.did_cache_db.exists()),
        }
    }

    /// Creates the accounts, the event log and the cache where they are
    /// missing, as the Reference creates them at the pin, with their
    /// migrations recorded at `now`. Gives the names of what it created.
    pub fn create_missing(&self, now: &str) -> Result<Vec<&'static str>, StorageError> {
        use crate::schema_shared::{ACCOUNT, DID_CACHE, SEQUENCER};
        let mut created = Vec::new();
        for (db, path, shared) in [
            (SharedDb::Account, &self.account_db, &ACCOUNT),
            (SharedDb::Sequencer, &self.sequencer_db, &SEQUENCER),
            (SharedDb::DidCache, &self.did_cache_db, &DID_CACHE),
        ] {
            match &self.shared {
                Some(tier) => {
                    let pool = tier.pool(db);
                    pool.create_schema()?;
                    if !pool.is_created()? {
                        let mut conn = pool.connect()?;
                        let txn = conn.transaction()?;
                        crate::schema::fill_shared(&txn, shared, now)?;
                        txn.commit()?;
                        created.push(db.name());
                    }
                }
                None if !path.exists() => {
                    drop(crate::schema::create_shared(path, shared, now)?);
                    created.push(db.name());
                }
                None => {}
            }
        }
        Ok(created)
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
