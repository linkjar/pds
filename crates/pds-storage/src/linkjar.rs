//! The tables that the `linkjar` profile adds to `account.sqlite`
//! (`docs/SPEC.md` §12.9, §13, EX-6).
//!
//! The patch stack of the Reference adds them with four migrations. The
//! Candidate applies the ones that a database lacks, under the same names,
//! so each server opens a database that the other migrated.

use rusqlite::{OptionalExtension, params};

use crate::account::AccountDb;
use crate::error::StorageError;

/// The migrations of the profile, in the order of their names: the name
/// that `kysely_migration` records, and the statements.
pub const MIGRATIONS: &[(&str, &[&str])] = &[
    (
        "007a-linkjar-093",
        &[
            r#"create table "linkjar_handle_policy" ("did" varchar primary key references "actor" ("did") on delete cascade, "createdAt" varchar not null, "renameHandle" varchar)"#,
        ],
    ),
    (
        "007b-linkjar-099",
        &[
            r#"create table "external_identity" ("provider" varchar not null, "subject" varchar not null, "did" varchar not null references "actor" ("did") on delete cascade, "assertedEmail" varchar, "emailVerified" integer not null, "displayName" varchar, "createdAt" varchar not null, "lastUsedAt" varchar not null, constraint "external_identity_pk" primary key ("provider", "subject"))"#,
            r#"create index "external_identity_did_idx" on "external_identity" ("did")"#,
        ],
    ),
    (
        "007c-linkjar-signup-receipt",
        &[
            r#"alter table "authorization_request" add column "signupDid" varchar"#,
            r#"alter table "token" add column "signupDid" varchar"#,
        ],
    ),
    (
        "007d-linkjar-signin-notices",
        &[
            r#"create table "sign_in_notice" ("id" varchar primary key, "did" varchar not null references "actor" ("did") on delete cascade, "recipient" varchar not null, "action" varchar not null, "provider" varchar, "createdAt" integer not null, "nextAttemptAt" integer not null, "attempts" integer default 0 not null)"#,
            r#"create index "sign_in_notice_ready" on "sign_in_notice" ("nextAttemptAt")"#,
        ],
    ),
];

/// The handle policy row of an account: when the account was created, as
/// the row says it, and the target it claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlePolicy {
    /// When the account was created (ISO 8601).
    pub created_at: String,
    /// The hosted handle that the account chose with its one rename.
    pub rename_handle: Option<String>,
}

impl AccountDb {
    /// Applies the migrations of the profile that the database lacks, each
    /// in a transaction with its record. Returns how many it applied.
    pub fn apply_linkjar_migrations(&mut self, now: &str) -> Result<usize, StorageError> {
        let mut applied = 0;
        for (name, statements) in MIGRATIONS {
            let txn = self.conn.transaction()?;
            let known: bool = txn.query_row(
                r#"select count(*) > 0 from "kysely_migration" where "name" = ?1"#,
                [name],
                |row| row.get(0),
            )?;
            if known {
                continue;
            }
            for statement in *statements {
                txn.execute_batch(statement)?;
            }
            txn.execute(
                r#"insert into "kysely_migration" ("name", "timestamp") values (?1, ?2)"#,
                [name, &now],
            )?;
            txn.commit()?;
            applied += 1;
        }
        Ok(applied)
    }

    /// The handle policy row of an account.
    pub fn handle_policy(&self, did: &str) -> Result<Option<HandlePolicy>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "createdAt", "renameHandle" from "linkjar_handle_policy" where "did" = ?1"#,
                [did],
                |row| {
                    Ok(HandlePolicy {
                        created_at: row.get(0)?,
                        rename_handle: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Gives a new account its handle policy row.
    pub fn register_handle_policy(&self, did: &str, created_at: &str) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "linkjar_handle_policy" ("did", "createdAt", "renameHandle") values (?1, ?2, null)"#,
            params![did, created_at],
        )?;
        Ok(())
    }

    /// Records the target of the account's one hosted rename. `false`
    /// means another request recorded a target first.
    pub fn claim_handle_rename(&self, did: &str, handle: &str) -> Result<bool, StorageError> {
        let changed = self.conn.execute(
            r#"update "linkjar_handle_policy" set "renameHandle" = ?2 where "did" = ?1 and "renameHandle" is null"#,
            params![did, handle],
        )?;
        Ok(changed == 1)
    }
}
