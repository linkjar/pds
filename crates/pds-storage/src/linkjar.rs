//! The tables that the `linkjar` profile adds to `account.sqlite`
//! (`docs/SPEC.md` §12.9, §13, EX-6).
//!
//! The patch stack of the Reference adds them with four migrations. The
//! Candidate applies the ones that a database lacks, under the same names,
//! so each server opens a database that the other migrated.

use crate::sql::{Connection, OptionalExtension, params};

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

/// An identity to store with an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewIdentity<'a> {
    /// `apple`, `google` or `github`.
    pub provider: &'a str,
    /// The provider's identifier of the person.
    pub subject: &'a str,
    /// The email address the provider asserted.
    pub email: Option<&'a str>,
    /// Whether the provider said the address is verified.
    pub email_verified: bool,
    /// The name the provider gave.
    pub display_name: Option<&'a str>,
}

/// An identity of an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// `apple`, `google` or `github`.
    pub provider: String,
    /// The provider's identifier of the person.
    pub subject: String,
    /// The email address the provider asserted.
    pub email: Option<String>,
}

/// Stores an identity for an account. `false` means the identity has an
/// account already: an identity is one account's.
pub(crate) fn insert_identity(
    conn: &Connection,
    did: &str,
    identity: &NewIdentity<'_>,
    now: &str,
) -> Result<bool, StorageError> {
    let inserted = conn.execute(
        r#"insert into "external_identity" ("provider", "subject", "did", "assertedEmail", "emailVerified", "displayName", "createdAt", "lastUsedAt") values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) on conflict do nothing"#,
        params![
            identity.provider,
            identity.subject,
            did,
            identity.email,
            i64::from(identity.email_verified),
            identity.display_name,
            now
        ],
    )?;
    Ok(inserted == 1)
}

/// Marks the pushed request with the account that is created in it. The
/// request must be open, of this client and this device, and without an
/// account of its own yet. `false` means it is not, and the account must
/// not be created.
pub(crate) fn register_signup(
    conn: &Connection,
    did: &str,
    request: &str,
    client: &str,
    device: &str,
    now: &str,
) -> Result<bool, StorageError> {
    let marked = conn.execute(
        r#"update "authorization_request" set "signupDid" = ?1 where "id" = ?2 and "clientId" = ?3 and "deviceId" = ?4 and "expiresAt" > ?5 and "did" is null and "code" is null and "signupDid" is null"#,
        params![did, request, client, device, now],
    )?;
    Ok(marked == 1)
}

impl AccountDb {
    /// The account that was created in the authorization of a code.
    pub fn signup_of_code(&self, code: &str) -> Result<Option<String>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "signupDid" from "authorization_request" where "code" = ?1"#,
                [code],
                |row| row.get::<Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Gives a session the creation receipt of its account.
    pub fn set_token_signup(&self, token_id: &str, did: &str) -> Result<(), StorageError> {
        self.conn.execute(
            r#"update "token" set "signupDid" = ?2 where "tokenId" = ?1 and "did" = ?2"#,
            params![token_id, did],
        )?;
        Ok(())
    }

    /// Whether the session holds the creation receipt of its account.
    pub fn token_has_signup(
        &self,
        token_id: &str,
        client_id: &str,
        did: &str,
        now: &str,
    ) -> Result<bool, StorageError> {
        Ok(self.conn.query_row(
            r#"select count(*) > 0 from "token" where "tokenId" = ?1 and "clientId" = ?2 and "did" = ?3 and "signupDid" = ?3 and "expiresAt" > ?4"#,
            params![token_id, client_id, did, now],
            |row| row.get(0),
        )?)
    }

    /// Clears the creation receipt of a session.
    pub fn clear_token_signup(
        &self,
        token_id: &str,
        client_id: &str,
        did: &str,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            r#"update "token" set "signupDid" = null where "tokenId" = ?1 and "clientId" = ?2 and "did" = ?3"#,
            params![token_id, client_id, did],
        )?;
        Ok(())
    }

    /// The account of an identity. A sign-in with it is recorded at `now`.
    pub fn external_identity(
        &self,
        provider: &str,
        subject: &str,
        now: &str,
    ) -> Result<Option<String>, StorageError> {
        let did: Option<String> = self
            .conn
            .query_row(
                r#"select "did" from "external_identity" where "provider" = ?1 and "subject" = ?2"#,
                [provider, subject],
                |row| row.get(0),
            )
            .optional()?;
        if did.is_some() {
            self.conn.execute(
                r#"update "external_identity" set "lastUsedAt" = ?3 where "provider" = ?1 and "subject" = ?2"#,
                [provider, subject, now],
            )?;
        }
        Ok(did)
    }

    /// The identities of an account, by provider.
    pub fn external_identities(&self, did: &str) -> Result<Vec<Identity>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select "provider", "subject", "assertedEmail" from "external_identity" where "did" = ?1 order by "provider", "subject""#,
        )?;
        let rows = statement.query_map([did], |row| {
            Ok(Identity {
                provider: row.get(0)?,
                subject: row.get(1)?,
                email: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

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

    /// Whether the account has a password. An account that an identity
    /// created has none until it sets one.
    pub fn has_password(&self, did: &str) -> Result<bool, StorageError> {
        let hash: Option<String> = self
            .conn
            .query_row(
                r#"select "passwordScrypt" from "account" where "did" = ?1"#,
                [did],
                |row| row.get(0),
            )
            .optional()?;
        Ok(hash.is_some_and(|hash| hash.contains(':')))
    }

    /// Links an identity to an account and enqueues the notice of it, in
    /// one transaction. An identity that an account holds already is not
    /// linked again: the answer names that account.
    pub fn link_identity(
        &mut self,
        did: &str,
        identity: &NewIdentity<'_>,
        notice_id: &str,
        now: &str,
        now_ms: i64,
    ) -> Result<Linked, StorageError> {
        let txn = self.conn.transaction()?;
        let owner: Option<String> = txn
            .query_row(
                r#"select "did" from "external_identity" where "provider" = ?1 and "subject" = ?2"#,
                [identity.provider, identity.subject],
                |row| row.get(0),
            )
            .optional()?;
        match owner {
            Some(owner) if owner == did => return Ok(Linked::Already),
            Some(owner) => return Ok(Linked::Other(owner)),
            None => {}
        }
        if !insert_identity(&txn, did, identity, now)? {
            return Ok(Linked::Already);
        }
        enqueue_notice(
            &txn,
            notice_id,
            did,
            "linked",
            Some(identity.provider),
            now_ms,
        )?;
        txn.commit()?;
        Ok(Linked::Done)
    }

    /// Removes an identity from an account and enqueues the notice of it,
    /// in one transaction. `usable` names the providers that can sign a
    /// person in now. `false` means the identity is the account's last
    /// sign-in method and stays.
    pub fn unlink_identity(
        &mut self,
        did: &str,
        provider: &str,
        subject: &str,
        usable: &[&str],
        notice_id: &str,
        now_ms: i64,
    ) -> Result<bool, StorageError> {
        let has_password = self.has_password(did)?;
        let txn = self.conn.transaction()?;
        let others: i64 = {
            let mut statement = txn.prepare(
                r#"select "provider", "subject" from "external_identity" where "did" = ?1"#,
            )?;
            let rows = statement.query_map([did], |row| {
                Ok((row.get::<String>(0)?, row.get::<String>(1)?))
            })?;
            let mut count = 0;
            for row in rows {
                let (other_provider, other_subject) = row?;
                let same = other_provider == provider && other_subject == subject;
                if !same && usable.contains(&other_provider.as_str()) {
                    count += 1;
                }
            }
            count
        };
        if !has_password && others == 0 {
            return Ok(false);
        }
        let removed = txn.execute(
            r#"delete from "external_identity" where "did" = ?1 and "provider" = ?2 and "subject" = ?3"#,
            params![did, provider, subject],
        )?;
        if removed == 1 {
            enqueue_notice(&txn, notice_id, did, "unlinked", Some(provider), now_ms)?;
        }
        txn.commit()?;
        Ok(true)
    }

    /// Up to `limit` notices that are due at `now_ms`, the oldest first.
    pub fn due_notices(&self, now_ms: i64, limit: u32) -> Result<Vec<Notice>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select "id", "did", "recipient", "action", "provider", "createdAt", "attempts" from "sign_in_notice" where "nextAttemptAt" <= ?1 order by "nextAttemptAt" asc limit ?2"#,
        )?;
        let rows = statement.query_map(params![now_ms, limit], |row| {
            Ok(Notice {
                id: row.get(0)?,
                did: row.get(1)?,
                recipient: row.get(2)?,
                action: row.get(3)?,
                provider: row.get(4)?,
                created_ms: row.get(5)?,
                attempts: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// How many notices wait for delivery.
    pub fn notice_count(&self) -> Result<u64, StorageError> {
        let count: i64 =
            self.conn
                .query_row(r#"select count(*) from "sign_in_notice""#, [], |row| {
                    row.get(0)
                })?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// Removes a notice that was delivered.
    pub fn delete_notice(&self, id: &str) -> Result<(), StorageError> {
        self.conn
            .execute(r#"delete from "sign_in_notice" where "id" = ?1"#, [id])?;
        Ok(())
    }

    /// Records a failed delivery and when to try again.
    pub fn retry_notice(&self, id: &str, next_ms: i64) -> Result<(), StorageError> {
        self.conn.execute(
            r#"update "sign_in_notice" set "attempts" = "attempts" + 1, "nextAttemptAt" = ?2 where "id" = ?1"#,
            params![id, next_ms],
        )?;
        Ok(())
    }
}

/// What a link did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Linked {
    /// The identity is now the account's.
    Done,
    /// The identity was the account's already. Nothing changed.
    Already,
    /// The identity is another account's: its DID.
    Other(String),
}

/// A notice of a changed sign-in method that waits for delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The id, which the `Message-ID` of the mail carries.
    pub id: String,
    /// The account.
    pub did: String,
    /// The address of the account when the change happened.
    pub recipient: String,
    /// `linked`, `unlinked` or `password-set`.
    pub action: String,
    /// The provider of a link or an unlink.
    pub provider: Option<String>,
    /// When the change happened, in milliseconds.
    pub created_ms: i64,
    /// How many deliveries failed.
    pub attempts: i64,
}

/// Enqueues a notice for the account's address of this moment. An account
/// without an address gets none.
pub(crate) fn enqueue_notice(
    conn: &Connection,
    id: &str,
    did: &str,
    action: &str,
    provider: Option<&str>,
    now_ms: i64,
) -> Result<(), StorageError> {
    let recipient: Option<String> = conn
        .query_row(
            r#"select "email" from "account" where "did" = ?1"#,
            [did],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(recipient) = recipient {
        conn.execute(
            r#"insert into "sign_in_notice" ("id", "did", "recipient", "action", "provider", "createdAt", "nextAttemptAt", "attempts") values (?1, ?2, ?3, ?4, ?5, ?6, ?6, 0)"#,
            params![id, did, recipient, action, provider, now_ms],
        )?;
    }
    Ok(())
}
