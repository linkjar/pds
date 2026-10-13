//! `ops.sqlite`: state that only the Candidate keeps (`docs/SPEC.md` §8.1).
//!
//! This unit keeps one thing in it: the commits that are stored and not yet
//! sequenced. A commit and its event are in two files, so no transaction
//! covers both. The server notes the commit here before it stores it and
//! removes the note after the event is in the log. A note that is still
//! there at startup names a repository whose head may be ahead of the log
//! (§7.4).
//!
//! Unit 9 adds the operator's state (§22.1): the roles of named accounts
//! and the email domains that may not sign up.

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

/// An account that holds a role on the ops listener (SPEC §12.11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorRole {
    /// The account.
    pub did: String,
    /// `operator` or `moderator`.
    pub role: String,
    /// Who granted it.
    pub granted_by: String,
    /// When (ISO 8601).
    pub granted_at: String,
}

/// An email domain that may not sign up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedEmailDomain {
    /// The domain, in lower case.
    pub domain: String,
    /// Why the operator blocked it.
    pub reason: String,
    /// Who blocked it.
    pub created_by: String,
    /// When (ISO 8601).
    pub created_at: String,
}

const OPERATOR_TABLES: &str = r#"
create table if not exists "operator_role" ("did" varchar primary key, "role" varchar not null, "grantedBy" varchar not null, "grantedAt" varchar not null);
create table if not exists "blocked_email_domain" ("domain" varchar primary key, "reason" varchar not null, "createdBy" varchar not null, "createdAt" varchar not null);
"#;

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
        conn.execute_batch(OPERATOR_TABLES)?;
        Ok(Self { conn })
    }

    /// The role of `did`, when it holds one.
    pub fn role(&self, did: &str) -> Result<Option<String>, StorageError> {
        use rusqlite::OptionalExtension as _;
        Ok(self
            .conn
            .query_row(
                r#"select "role" from "operator_role" where "did" = ?1"#,
                [did],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Every role, by account.
    pub fn roles(&self) -> Result<Vec<OperatorRole>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select "did", "role", "grantedBy", "grantedAt" from "operator_role" order by "did""#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok(OperatorRole {
                did: row.get(0)?,
                role: row.get(1)?,
                granted_by: row.get(2)?,
                granted_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Gives `did` a role, in place of the one it holds.
    pub fn grant_role(&self, role: &OperatorRole) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "operator_role" ("did", "role", "grantedBy", "grantedAt") values (?1, ?2, ?3, ?4) on conflict ("did") do update set "role" = ?2, "grantedBy" = ?3, "grantedAt" = ?4"#,
            [&role.did, &role.role, &role.granted_by, &role.granted_at],
        )?;
        Ok(())
    }

    /// Takes the role of `did` away. `false` when it held none.
    pub fn revoke_role(&self, did: &str) -> Result<bool, StorageError> {
        Ok(self
            .conn
            .execute(r#"delete from "operator_role" where "did" = ?1"#, [did])?
            > 0)
    }

    /// Every blocked email domain, by name.
    pub fn blocked_email_domains(&self) -> Result<Vec<BlockedEmailDomain>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select "domain", "reason", "createdBy", "createdAt" from "blocked_email_domain" order by "domain""#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok(BlockedEmailDomain {
                domain: row.get(0)?,
                reason: row.get(1)?,
                created_by: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Blocks a domain. A domain that is blocked keeps its first row.
    pub fn block_email_domain(&self, block: &BlockedEmailDomain) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "blocked_email_domain" ("domain", "reason", "createdBy", "createdAt") values (?1, ?2, ?3, ?4) on conflict ("domain") do nothing"#,
            [
                &block.domain,
                &block.reason,
                &block.created_by,
                &block.created_at,
            ],
        )?;
        Ok(())
    }

    /// Lifts the block of a domain. `false` when it was not blocked.
    pub fn unblock_email_domain(&self, domain: &str) -> Result<bool, StorageError> {
        Ok(self.conn.execute(
            r#"delete from "blocked_email_domain" where "domain" = ?1"#,
            [domain],
        )? > 0)
    }

    /// Whether the domain of `email`, or a domain above it, is blocked.
    pub fn is_email_blocked(&self, email: &str) -> Result<bool, StorageError> {
        let Some((_, domain)) = email.rsplit_once('@') else {
            return Ok(false);
        };
        let domain = domain.to_ascii_lowercase();
        let mut statement = self
            .conn
            .prepare_cached(r#"select 1 from "blocked_email_domain" where "domain" = ?1"#)?;
        let mut rest = domain.as_str();
        loop {
            if statement.exists([rest])? {
                return Ok(true);
            }
            match rest.split_once('.') {
                Some((_, above)) if above.contains('.') => rest = above,
                _ => return Ok(false),
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_and_blocked_domains_are_kept() {
        let dir = std::env::temp_dir().join(format!("pds-ops-db-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ops = OpsDb::open(&dir.join("ops.sqlite")).unwrap();
        let role = |role: &str| OperatorRole {
            did: "did:plc:a".to_owned(),
            role: role.to_owned(),
            granted_by: "admin".to_owned(),
            granted_at: "2026-10-13T00:00:00.000Z".to_owned(),
        };
        ops.grant_role(&role("moderator")).unwrap();
        ops.grant_role(&role("operator")).unwrap();
        assert_eq!(ops.role("did:plc:a").unwrap().as_deref(), Some("operator"));
        assert_eq!(ops.roles().unwrap().len(), 1);
        assert!(ops.revoke_role("did:plc:a").unwrap());
        assert!(!ops.revoke_role("did:plc:a").unwrap());
        assert_eq!(ops.role("did:plc:a").unwrap(), None);

        ops.block_email_domain(&BlockedEmailDomain {
            domain: "spam.example".to_owned(),
            reason: "a wave of sign-ups".to_owned(),
            created_by: "admin".to_owned(),
            created_at: "2026-10-13T00:00:00.000Z".to_owned(),
        })
        .unwrap();
        assert!(ops.is_email_blocked("a@spam.example").unwrap());
        assert!(ops.is_email_blocked("a@Mail.Spam.Example").unwrap());
        assert!(!ops.is_email_blocked("a@notspam.example").unwrap());
        assert!(!ops.is_email_blocked("a@example").unwrap());
        assert!(!ops.is_email_blocked("no-at-sign").unwrap());
        assert!(ops.unblock_email_domain("spam.example").unwrap());
        assert!(!ops.is_email_blocked("a@spam.example").unwrap());
    }
}
