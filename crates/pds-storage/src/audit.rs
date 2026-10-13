//! `audit.sqlite`: the entries and the checkpoints of the audit log
//! (`docs/SPEC.md` §8.1, §20).
//!
//! The store keeps bytes. What an entry says, how entries chain and what a
//! checkpoint signs are `pds-audit`'s. The tables take inserts only: a
//! trigger refuses every update and every delete, so a connection of the
//! server has no way to rewrite the log.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::error::StorageError;

const SCHEMA: &str = r#"
create table if not exists "entry" ("seq" integer primary key, "hash" blob not null, "bytes" blob not null);
create table if not exists "checkpoint" ("last" integer primary key, "first" integer not null, "root" blob not null, "head" blob not null, "time" varchar not null, "keyId" varchar not null, "signature" blob not null);
create table if not exists "key" ("keyId" varchar primary key, "publicKey" blob not null);
create trigger if not exists "key_no_update" before update on "key" begin select raise(abort, 'the audit log takes inserts only'); end;
create trigger if not exists "key_no_delete" before delete on "key" begin select raise(abort, 'the audit log takes inserts only'); end;
create trigger if not exists "entry_no_update" before update on "entry" begin select raise(abort, 'the audit log takes inserts only'); end;
create trigger if not exists "entry_no_delete" before delete on "entry" begin select raise(abort, 'the audit log takes inserts only'); end;
create trigger if not exists "checkpoint_no_update" before update on "checkpoint" begin select raise(abort, 'the audit log takes inserts only'); end;
create trigger if not exists "checkpoint_no_delete" before delete on "checkpoint" begin select raise(abort, 'the audit log takes inserts only'); end;
"#;

/// A stored checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointRow {
    /// The first entry it covers.
    pub first: u64,
    /// The last entry it covers.
    pub last: u64,
    /// The Merkle root.
    pub root: Vec<u8>,
    /// The hash of the last entry.
    pub head: Vec<u8>,
    /// When it was made (ISO 8601).
    pub time: String,
    /// The key that signed it.
    pub key_id: String,
    /// The signature.
    pub signature: Vec<u8>,
}

/// A connection to `audit.sqlite`.
#[derive(Debug)]
pub struct AuditDb {
    conn: Connection,
}

fn seq(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn checkpoint_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CheckpointRow> {
    Ok(CheckpointRow {
        first: u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
        last: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
        root: row.get(2)?,
        head: row.get(3)?,
        time: row.get(4)?,
        key_id: row.get(5)?,
        signature: row.get(6)?,
    })
}

const CHECKPOINT_COLUMNS: &str = r#""first", "last", "root", "head", "time", "keyId", "signature""#;

impl AuditDb {
    /// Opens the database, and creates it when the data directory has
    /// none.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = crate::io::open(path, flags)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Appends an entry. `build` gets the number and the bytes of the
    /// last entry and gives the number, the hash and the bytes of the
    /// next. The read and the insert are one transaction, so two writers
    /// cannot give two entries the same place.
    pub fn append(
        &mut self,
        build: impl FnOnce(Option<(u64, &[u8])>) -> (u64, [u8; 32], Vec<u8>),
    ) -> Result<u64, StorageError> {
        let txn = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let last: Option<(i64, Vec<u8>)> = txn
            .query_row(
                r#"select "seq", "bytes" from "entry" order by "seq" desc limit 1"#,
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (number, hash, bytes) = build(
            last.as_ref()
                .map(|(seq, bytes)| (u64::try_from(*seq).unwrap_or(0), bytes.as_slice())),
        );
        txn.execute(
            r#"insert into "entry" ("seq", "hash", "bytes") values (?1, ?2, ?3)"#,
            params![seq(number), hash.as_slice(), bytes],
        )?;
        txn.commit()?;
        Ok(number)
    }

    /// The number of the last entry, or 0 for an empty log.
    pub fn last_seq(&self) -> Result<u64, StorageError> {
        let last: Option<i64> =
            self.conn
                .query_row(r#"select max("seq") from "entry""#, [], |row| row.get(0))?;
        Ok(last.and_then(|last| u64::try_from(last).ok()).unwrap_or(0))
    }

    /// The bytes of the entries from `from` to `to`, in order.
    pub fn entries(&self, from: u64, to: u64) -> Result<Vec<(u64, Vec<u8>)>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select "seq", "bytes" from "entry" where "seq" >= ?1 and "seq" <= ?2 order by "seq""#,
        )?;
        let rows = statement.query_map(params![seq(from), seq(to)], |row| {
            Ok((
                u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
                row.get(1)?,
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The stored hash of an entry.
    pub fn entry_hash(&self, number: u64) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "hash" from "entry" where "seq" = ?1"#,
                [seq(number)],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The latest checkpoint.
    pub fn last_checkpoint(&self) -> Result<Option<CheckpointRow>, StorageError> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    r#"select {CHECKPOINT_COLUMNS} from "checkpoint" order by "last" desc limit 1"#
                ),
                [],
                checkpoint_from_row,
            )
            .optional()?)
    }

    /// Stores a checkpoint.
    pub fn insert_checkpoint(&self, row: &CheckpointRow) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "checkpoint" ("first", "last", "root", "head", "time", "keyId", "signature") values (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                seq(row.first),
                seq(row.last),
                row.root,
                row.head,
                row.time,
                row.key_id,
                row.signature
            ],
        )?;
        Ok(())
    }

    /// Keeps the public key of a checkpoint key, so that an export can
    /// carry every key that signed.
    pub fn remember_key(&self, key_id: &str, public_key: &[u8]) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "key" ("keyId", "publicKey") values (?1, ?2) on conflict do nothing"#,
            params![key_id, public_key],
        )?;
        Ok(())
    }

    /// The public keys of the checkpoint keys, by id.
    pub fn keys(&self) -> Result<Vec<(String, Vec<u8>)>, StorageError> {
        let mut statement = self
            .conn
            .prepare(r#"select "keyId", "publicKey" from "key" order by "keyId""#)?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The checkpoints that cover an entry from `from` to `to`, in order.
    pub fn checkpoints(&self, from: u64, to: u64) -> Result<Vec<CheckpointRow>, StorageError> {
        let mut statement = self.conn.prepare(&format!(
            r#"select {CHECKPOINT_COLUMNS} from "checkpoint" where "last" >= ?1 and "first" <= ?2 order by "last""#
        ))?;
        let rows = statement.query_map(params![seq(from), seq(to)], checkpoint_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_takes_inserts_only() {
        let dir = std::env::temp_dir().join(format!("pds-audit-db-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit.sqlite");
        let mut db = AuditDb::open(&path).unwrap();
        assert_eq!(db.last_seq().unwrap(), 0);
        for n in 1..=3_u8 {
            let number = db
                .append(|last| {
                    assert_eq!(last.map(|(seq, _)| seq), (n > 1).then(|| u64::from(n) - 1));
                    (u64::from(n), [n; 32], vec![n])
                })
                .unwrap();
            assert_eq!(number, u64::from(n));
        }
        assert_eq!(db.entries(2, 3).unwrap(), [(2, vec![2]), (3, vec![3])]);
        assert_eq!(db.entry_hash(2).unwrap(), Some(vec![2; 32]));
        // A place is given once.
        assert!(db.append(|_| (3, [0; 32], vec![9])).is_err());

        let row = CheckpointRow {
            first: 1,
            last: 3,
            root: vec![1; 32],
            head: vec![3; 32],
            time: "2026-10-13T00:00:00.000Z".to_owned(),
            key_id: "ed25519:00".to_owned(),
            signature: vec![7; 64],
        };
        db.insert_checkpoint(&row).unwrap();
        assert_eq!(db.last_checkpoint().unwrap(), Some(row.clone()));
        assert_eq!(db.checkpoints(2, 2).unwrap(), [row]);
        assert_eq!(db.checkpoints(4, 9).unwrap(), Vec::<CheckpointRow>::new());

        // No statement of the connection rewrites the log.
        for statement in [
            r#"update "entry" set "bytes" = x'00' where "seq" = 1"#,
            r#"delete from "entry" where "seq" = 1"#,
            r#"update "checkpoint" set "time" = 'x'"#,
            r#"delete from "checkpoint""#,
        ] {
            let refused = db.conn.execute(statement, []).unwrap_err().to_string();
            assert!(refused.contains("takes inserts only"), "{refused}");
        }
        // The log is there after a reopen.
        drop(db);
        assert_eq!(AuditDb::open(&path).unwrap().last_seq().unwrap(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
