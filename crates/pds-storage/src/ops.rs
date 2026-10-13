//! `ops.sqlite`: state that only the Candidate keeps (`docs/SPEC.md` §8.1).
//!
//! This unit keeps one thing in it: the commits that are stored and not yet
//! sequenced. A commit and its event are in two files, so no transaction
//! covers both. The server notes the commit here before it stores it and
//! removes the note after the event is in the log. A note that is still
//! there at startup names a repository whose head may be ahead of the log
//! (§7.4).

use std::path::Path;

use rusqlite::{Connection, OpenFlags, named_params};

use crate::error::StorageError;

/// A commit that was about to be stored when the note was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCommit {
    /// The account.
    pub did: String,
    /// The revision of the commit.
    pub rev: String,
}

/// A connection to `ops.sqlite`.
#[derive(Debug)]
pub struct OpsDb {
    conn: Connection,
}

impl OpsDb {
    /// Opens the database and creates it when it is not there.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = crate::io::open(path, flags)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.execute_batch(
            r#"create table if not exists "pending_commit" ("did" varchar primary key, "rev" varchar not null)"#,
        )?;
        Ok(Self { conn })
    }

    /// Notes that the commit `rev` of `did` is about to be stored.
    pub fn note_commit(&self, did: &str, rev: &str) -> Result<(), StorageError> {
        self.conn.prepare_cached(
            r#"insert into "pending_commit" ("did", "rev") values (:did, :rev) on conflict ("did") do update set "rev" = :rev"#,
        )?
        .execute(named_params! { ":did": did, ":rev": rev })?;
        Ok(())
    }

    /// Removes the note of `did` when it is about `rev`.
    ///
    /// The removal is not synced. A note that outlives its commit costs one
    /// look at the log at the next start; a note that is lost before its
    /// commit is stored would cost the repair, so [`OpsDb::note_commit`]
    /// is synced.
    pub fn clear_commit(&self, did: &str, rev: &str) -> Result<(), StorageError> {
        self.conn.pragma_update(None, "synchronous", "normal")?;
        let cleared = self
            .conn
            .prepare_cached(r#"delete from "pending_commit" where "did" = :did and "rev" = :rev"#)
            .and_then(|mut statement| {
                statement.execute(named_params! { ":did": did, ":rev": rev })
            });
        self.conn.pragma_update(None, "synchronous", "full")?;
        cleared?;
        Ok(())
    }

    /// Every note, by account.
    pub fn pending_commits(&self) -> Result<Vec<PendingCommit>, StorageError> {
        let mut statement = self
            .conn
            .prepare(r#"select "did", "rev" from "pending_commit" order by "did""#)?;
        let rows = statement.query_map([], |row| {
            Ok(PendingCommit {
                did: row.get(0)?,
                rev: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}
