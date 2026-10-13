//! `account.sqlite`: the actors this server hosts and their status.

use std::path::Path;

use crate::sql::{Connection, OptionalExtension, Row, named_params, params};

use crate::error::StorageError;
use crate::layout::{open_read_only, open_read_write};

/// One row of `actor`: a hosted repository and its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    /// The DID.
    pub did: String,
    /// The handle, absent while none is assigned.
    pub handle: Option<String>,
    /// When the actor was created, as the Reference wrote it (ISO 8601).
    pub created_at: String,
    /// Present while the account is taken down.
    pub takedown_ref: Option<String>,
    /// Present while the account is deactivated.
    pub deactivated_at: Option<String>,
}

impl Actor {
    /// The status a sync method reports: `None` for an active account.
    pub const fn status(&self) -> Option<&'static str> {
        if self.takedown_ref.is_some() {
            Some("takendown")
        } else if self.deactivated_at.is_some() {
            Some("deactivated")
        } else {
            None
        }
    }
}

/// An actor with the rows of its account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// The actor.
    pub actor: Actor,
    /// The email address, absent for an actor without an account row.
    pub email: Option<String>,
    /// When the email address was confirmed.
    pub email_confirmed_at: Option<String>,
}

/// An app password: its name and whether it is privileged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPassword {
    /// The name the account gave it.
    pub name: String,
    /// Whether it may do what the account password may, except account
    /// management.
    pub privileged: bool,
}

/// One row of `refresh_token`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshToken {
    /// The account.
    pub did: String,
    /// When the token ends (ISO 8601).
    pub expires_at: String,
    /// The token that replaced this one, once it was used.
    pub next_id: Option<String>,
    /// The app password of the session, when it has one.
    pub app_password: Option<AppPassword>,
}

/// An invite code with its uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteCode {
    /// The code.
    pub code: String,
    /// How many accounts it can create.
    pub available: i64,
    /// Whether it is disabled.
    pub disabled: bool,
    /// The account it was made for, or `admin`.
    pub for_account: String,
    /// Who made it: `admin`, or the account itself.
    pub created_by: String,
    /// When it was made (ISO 8601).
    pub created_at: String,
    /// Who used it and when, the newest first.
    pub uses: Vec<(String, String)>,
}

/// Which invite codes a query returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invites<'a> {
    /// The codes made for one account.
    For(&'a str),
    /// The code that one account used.
    UsedBy(&'a str),
    /// A page of all codes, the newest first. The cursor is the creation
    /// time and the code of the last entry of the page before.
    Recent {
        /// The page size.
        limit: u32,
        /// Where the page starts.
        after: Option<(&'a str, &'a str)>,
    },
    /// A page of all codes, the most used first. The cursor is the count
    /// of uses and the code of the last entry of the page before.
    Usage {
        /// The page size.
        limit: u32,
        /// Where the page starts.
        after: Option<(i64, &'a str)>,
    },
}

/// The rows of a new account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAccount<'a> {
    /// The DID.
    pub did: &'a str,
    /// The handle.
    pub handle: &'a str,
    /// The email address and the stored hash of the password. An account
    /// without them has an actor row only.
    pub credentials: Option<(&'a str, &'a str)>,
    /// The identity at an identity provider that the account is created
    /// for. Its email address is then confirmed at creation.
    pub external_identity: Option<crate::linkjar::NewIdentity<'a>>,
    /// Whether the account gets a handle policy row.
    pub handle_policy: bool,
    /// The authorization the account is created in, which then holds the
    /// creation receipt: the request, its client and its device.
    pub signup: Option<(&'a str, &'a str, &'a str)>,
    /// The invite code the account used.
    pub invite_code: Option<&'a str>,
    /// The id of the first refresh token and when it ends.
    pub refresh_token: Option<(&'a str, &'a str)>,
    /// The CID and the revision of the first commit.
    pub repo_root: (&'a str, &'a str),
    /// The time of creation (ISO 8601).
    pub now: &'a str,
    /// When set, the account is created deactivated and is deleted at
    /// this time unless it is activated.
    pub delete_after: Option<&'a str>,
    /// Whether the account is created deactivated, with no time to
    /// delete it: an account that moves here.
    pub deactivated: bool,
}

/// Why the rows of a new account were not written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The invite code is unknown, disabled, used up, or its owner is
    /// taken down.
    Invite,
    /// The authorization that the account was to be created in is over,
    /// or it created an account already.
    Authorization,
    /// An actor has the DID, or an account has it.
    Exists,
}

/// Which accounts a lookup may return besides active ones.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Availability {
    /// Return an account that is taken down.
    pub include_taken_down: bool,
    /// Return an account that is deactivated.
    pub include_deactivated: bool,
}

impl Availability {
    /// Every account, whatever its status.
    pub const ANY: Self = Self {
        include_taken_down: true,
        include_deactivated: true,
    };
    /// Active accounts only.
    pub const ACTIVE: Self = Self {
        include_taken_down: false,
        include_deactivated: false,
    };
}

/// One entry of `com.atproto.sync.listRepos`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoListing {
    /// The actor.
    pub actor: Actor,
    /// The CID of the latest commit.
    pub head: String,
    /// The revision of the latest commit.
    pub rev: String,
}

/// A connection to `account.sqlite`.
#[derive(Debug)]
pub struct AccountDb {
    pub(crate) conn: Connection,
}

const ACTOR_COLUMNS: &str =
    r#"actor."did", actor."handle", actor."createdAt", actor."takedownRef", actor."deactivatedAt""#;

fn actor_from_row(row: &Row<'_>) -> crate::sql::Result<Actor> {
    Ok(Actor {
        did: row.get(0)?,
        handle: row.get(1)?,
        created_at: row.get(2)?,
        takedown_ref: row.get(3)?,
        deactivated_at: row.get(4)?,
    })
}

fn invite_available(conn: &Connection, code: &str) -> Result<bool, StorageError> {
    let invite: Option<(i64, Option<i64>)> = conn
        .query_row(
            r#"select invite_code."availableUses", invite_code."disabled" from "invite_code" left join "actor" on actor."did" = invite_code."forAccount" where actor."takedownRef" is null and invite_code."code" = ?1"#,
            [code],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((available, disabled)) = invite else {
        return Ok(false);
    };
    if disabled.unwrap_or(0) != 0 {
        return Ok(false);
    }
    let uses: i64 = conn.query_row(
        r#"select count(*) from "invite_code_use" where "code" = ?1"#,
        [code],
        |row| row.get(0),
    )?;
    Ok(available > uses)
}

impl AccountDb {
    /// Opens the database for reading.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            conn: open_read_only(path)?.into(),
        })
    }

    /// Opens the database for reading and writing.
    pub fn open_writable(path: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            conn: open_read_write(path)?.into(),
        })
    }

    /// Records the latest commit of an account, which `listRepos` reads.
    pub fn update_repo_root(
        &self,
        did: &str,
        cid: &str,
        rev: &str,
        now: &str,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "repo_root" ("did", "cid", "rev", "indexedAt") values (:did, :cid, :rev, :now) on conflict ("did") do update set "cid" = :cid, "rev" = :rev"#,
            named_params! { ":did": did, ":cid": cid, ":rev": rev, ":now": now },
        )?;
        Ok(())
    }

    /// Runs the query the health route runs.
    pub fn ping(&self) -> Result<(), StorageError> {
        self.conn.query_row("select 1", [], |_| Ok(()))?;
        Ok(())
    }

    /// The actor with this DID or this handle. A value that starts with
    /// `did:` is a DID, as in the Reference; a handle is compared as given.
    pub fn actor(
        &self,
        handle_or_did: &str,
        availability: Availability,
    ) -> Result<Option<Actor>, StorageError> {
        let column = if handle_or_did.starts_with("did:") {
            "did"
        } else {
            "handle"
        };
        let mut sql =
            format!(r#"select {ACTOR_COLUMNS} from "actor" where actor."{column}" = :id"#);
        if !availability.include_taken_down {
            sql.push_str(r#" and actor."takedownRef" is null"#);
        }
        if !availability.include_deactivated {
            sql.push_str(r#" and actor."deactivatedAt" is null"#);
        }
        Ok(self
            .conn
            .query_row(&sql, named_params! { ":id": handle_or_did }, actor_from_row)
            .optional()?)
    }

    /// The account with this DID, this handle, or this email address when
    /// `by_email` is set. An email address is compared in lower case.
    pub fn account(
        &self,
        id: &str,
        by_email: bool,
        availability: Availability,
    ) -> Result<Option<Account>, StorageError> {
        let condition = if by_email {
            r#"lower(account."email") = :id"#
        } else if id.starts_with("did:") {
            r#"actor."did" = :id"#
        } else {
            r#"actor."handle" = :id"#
        };
        let mut sql = format!(
            r#"select {ACTOR_COLUMNS}, account."email", account."emailConfirmedAt" from "actor" left join "account" on account."did" = actor."did" where {condition}"#
        );
        if !availability.include_taken_down {
            sql.push_str(r#" and actor."takedownRef" is null"#);
        }
        if !availability.include_deactivated {
            sql.push_str(r#" and actor."deactivatedAt" is null"#);
        }
        Ok(self
            .conn
            .query_row(&sql, named_params! { ":id": id }, |row| {
                Ok(Account {
                    actor: actor_from_row(row)?,
                    email: row.get(5)?,
                    email_confirmed_at: row.get(6)?,
                })
            })
            .optional()?)
    }

    /// The stored hash of the account password: `<salt>:<hash>`, both hex.
    pub fn password_hash(&self, did: &str) -> Result<Option<String>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "passwordScrypt" from "account" where "did" = ?1"#,
                [did],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The app password of `did` with this stored hash.
    pub fn app_password(&self, did: &str, hash: &str) -> Result<Option<AppPassword>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "name", "privileged" from "app_password" where "did" = ?1 and "passwordScrypt" = ?2"#,
                [did, hash],
                |row| {
                    Ok(AppPassword {
                        name: row.get(0)?,
                        privileged: row.get::<i64>(1)? == 1,
                    })
                },
            )
            .optional()?)
    }

    /// Takes an account down with the reference `takedown`, or lifts the
    /// takedown with `None`. A takedown ends the sessions of the account.
    pub fn set_takedown(&mut self, did: &str, takedown: Option<&str>) -> Result<(), StorageError> {
        let txn = self.conn.transaction()?;
        txn.execute(
            r#"update "actor" set "takedownRef" = ?2 where "did" = ?1"#,
            params![did, takedown],
        )?;
        txn.execute(r#"delete from "refresh_token" where "did" = ?1"#, [did])?;
        txn.commit()?;
        Ok(())
    }

    /// Whether the account may give out invite codes: `None` for an actor
    /// without an account row.
    pub fn invites_disabled(&self, did: &str) -> Result<Option<bool>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "invitesDisabled" from "account" where "did" = ?1"#,
                [did],
                |row| row.get::<i64>(0).map(|value| value == 1),
            )
            .optional()?)
    }

    /// Gives an account a handle. Returns `false`, and changes nothing,
    /// when another account has the handle or the account is not there.
    pub fn update_handle(&self, did: &str, handle: &str) -> Result<bool, StorageError> {
        let updated = self.conn.execute(
            r#"update "actor" set "handle" = ?2 where "did" = ?1 and not exists (select 1 from "actor" where "handle" = ?2 and "did" != ?1)"#,
            [did, handle],
        )?;
        Ok(updated > 0)
    }

    /// Deactivates an account at `now`. With `delete_after` it is deleted
    /// at that time unless it is activated. Returns whether the actor
    /// exists.
    pub fn deactivate(
        &self,
        did: &str,
        now: &str,
        delete_after: Option<&str>,
    ) -> Result<bool, StorageError> {
        let updated = self.conn.execute(
            r#"update "actor" set "deactivatedAt" = ?2, "deleteAfter" = ?3 where "did" = ?1"#,
            params![did, now, delete_after],
        )?;
        Ok(updated > 0)
    }

    /// Activates an account that is not taken down. Returns whether there
    /// is one.
    pub fn activate(&self, did: &str) -> Result<bool, StorageError> {
        let updated = self.conn.execute(
            r#"update "actor" set "deactivatedAt" = null, "deleteAfter" = null where "did" = ?1 and "takedownRef" is null"#,
            [did],
        )?;
        Ok(updated > 0)
    }

    /// Stores the email token of `did` for `purpose`, in place of the one
    /// before.
    pub fn create_email_token(
        &self,
        did: &str,
        purpose: &str,
        token: &str,
        now: &str,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "email_token" ("purpose", "did", "token", "requestedAt") values (?1, ?2, ?3, ?4) on conflict ("purpose", "did") do update set "token" = excluded."token", "requestedAt" = excluded."requestedAt""#,
            [purpose, did, token, now],
        )?;
        Ok(())
    }

    /// The account and the request time of an email token. With `did` the
    /// token must be that account's.
    pub fn email_token(
        &self,
        purpose: &str,
        token: &str,
        did: Option<&str>,
    ) -> Result<Option<(String, String)>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "did", "requestedAt" from "email_token" where "purpose" = ?1 and "token" = ?2 and (?3 is null or "did" = ?3)"#,
                params![purpose, token, did],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// Removes the email token of `did` for `purpose`.
    pub fn delete_email_token(&self, did: &str, purpose: &str) -> Result<(), StorageError> {
        self.conn.execute(
            r#"delete from "email_token" where "did" = ?1 and "purpose" = ?2"#,
            [did, purpose],
        )?;
        Ok(())
    }

    /// Marks the email address of `did` as confirmed at `now` and removes
    /// the token that confirmed it.
    pub fn confirm_email(&mut self, did: &str, now: &str) -> Result<(), StorageError> {
        let txn = self.conn.transaction()?;
        txn.execute(
            r#"delete from "email_token" where "did" = ?1 and "purpose" = 'confirm_email'"#,
            [did],
        )?;
        txn.execute(
            r#"update "account" set "emailConfirmedAt" = ?2 where "did" = ?1"#,
            [did, now],
        )?;
        txn.commit()?;
        Ok(())
    }

    /// Gives an account another email address, not confirmed, and removes
    /// its email tokens. Returns `false`, and changes nothing, when
    /// another account has the address.
    pub fn update_email(&mut self, did: &str, email: &str) -> Result<bool, StorageError> {
        let email = email.to_lowercase();
        let txn = self.conn.transaction()?;
        let taken: bool = txn.query_row(
            r#"select count(*) > 0 from "account" where lower("email") = ?1 and "did" != ?2"#,
            [&email, did],
            |row| row.get(0),
        )?;
        if taken {
            return Ok(false);
        }
        txn.execute(
            r#"update "account" set "email" = ?2, "emailConfirmedAt" = null where "did" = ?1"#,
            [did, &email],
        )?;
        txn.execute(r#"delete from "email_token" where "did" = ?1"#, [did])?;
        txn.commit()?;
        Ok(true)
    }

    /// Gives an account another password. Its sessions end, and the token
    /// that reset the password is removed.
    pub fn update_password(
        &mut self,
        did: &str,
        hash: &str,
        notice: Option<(&str, i64)>,
    ) -> Result<(), StorageError> {
        let txn = self.conn.transaction()?;
        // A profile that mails notices of sign-in changes gets one in the
        // transaction of the change: its id and the time.
        if let Some((id, now_ms)) = notice {
            crate::linkjar::enqueue_notice(&txn, id, did, "password-set", None, now_ms)?;
        }
        txn.execute(
            r#"update "account" set "passwordScrypt" = ?2 where "did" = ?1"#,
            [did, hash],
        )?;
        txn.execute(
            r#"delete from "email_token" where "did" = ?1 and "purpose" = 'reset_password'"#,
            [did],
        )?;
        txn.execute(r#"delete from "refresh_token" where "did" = ?1"#, [did])?;
        txn.commit()?;
        Ok(())
    }

    /// Stores invite codes: each code with the account it is for.
    pub fn create_invite_codes(
        &mut self,
        codes: &[(String, String)],
        use_count: i64,
        created_by: &str,
        now: &str,
    ) -> Result<(), StorageError> {
        let txn = self.conn.transaction()?;
        for (code, for_account) in codes {
            txn.execute(
                r#"insert into "invite_code" ("code", "availableUses", "disabled", "forAccount", "createdBy", "createdAt") values (?1, ?2, 0, ?3, ?4, ?5)"#,
                params![code, use_count, for_account, created_by, now],
            )?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Invite codes with their uses.
    pub fn invite_codes(&self, which: Invites<'_>) -> Result<Vec<InviteCode>, StorageError> {
        const SELECT: &str = r#"select * from (select invite_code."code" as code, invite_code."availableUses" as available, invite_code."disabled" as disabled, invite_code."forAccount" as forAccount, invite_code."createdBy" as createdBy, invite_code."createdAt" as createdAt, (select count(*) from "invite_code_use" where invite_code_use."code" = invite_code."code") as uses from "invite_code") as codes"#;
        let (tail, params): (String, Vec<crate::sql::Value>) = match which {
            Invites::For(did) => (" where forAccount = ?1".to_owned(), vec![did.to_owned().into()]),
            Invites::UsedBy(did) => (
                r#" where code in (select distinct "code" from "invite_code_use" where "usedBy" = ?1)"#.to_owned(),
                vec![did.to_owned().into()],
            ),
            Invites::Recent { limit, after } => match after {
                Some((created_at, code)) => (
                    " where (createdAt, code) < (?1, ?2) order by createdAt desc, code desc limit ?3".to_owned(),
                    vec![created_at.to_owned().into(), code.to_owned().into(), i64::from(limit).into()],
                ),
                None => (
                    " order by createdAt desc, code desc limit ?1".to_owned(),
                    vec![i64::from(limit).into()],
                ),
            },
            Invites::Usage { limit, after } => match after {
                Some((uses, code)) => (
                    " where (uses, code) < (?1, ?2) order by uses desc, code desc limit ?3".to_owned(),
                    vec![uses.into(), code.to_owned().into(), i64::from(limit).into()],
                ),
                None => (
                    " order by uses desc, code desc limit ?1".to_owned(),
                    vec![i64::from(limit).into()],
                ),
            },
        };
        let mut statement = self.conn.prepare(&format!("{SELECT}{tail}"))?;
        let rows = statement.query_map(crate::sql::params_from_iter(params), |row| {
            Ok(InviteCode {
                code: row.get(0)?,
                available: row.get(1)?,
                disabled: row.get::<Option<i64>>(2)?.unwrap_or(0) == 1,
                for_account: row.get(3)?,
                created_by: row.get(4)?,
                created_at: row.get(5)?,
                uses: Vec::new(),
            })
        })?;
        let mut codes: Vec<InviteCode> = rows.collect::<Result<_, _>>()?;
        let mut uses = self.conn.prepare(
            r#"select "usedBy", "usedAt" from "invite_code_use" where "code" = ?1 order by "usedAt" desc"#,
        )?;
        for code in &mut codes {
            code.uses = uses
                .query_map([&code.code], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;
        }
        Ok(codes)
    }

    /// Disables the codes named and every code of the accounts named.
    pub fn disable_invite_codes(
        &mut self,
        codes: &[String],
        accounts: &[String],
    ) -> Result<(), StorageError> {
        let txn = self.conn.transaction()?;
        for code in codes {
            txn.execute(
                r#"update "invite_code" set "disabled" = 1 where "code" = ?1"#,
                [code],
            )?;
        }
        for account in accounts {
            txn.execute(
                r#"update "invite_code" set "disabled" = 1 where "forAccount" = ?1"#,
                [account],
            )?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Says whether the account may hand out invites.
    pub fn set_invites_disabled(&self, did: &str, disabled: bool) -> Result<(), StorageError> {
        self.conn.execute(
            r#"update "account" set "invitesDisabled" = ?2 where "did" = ?1"#,
            params![did, i64::from(disabled)],
        )?;
        Ok(())
    }

    /// Whether the invite code can still be used: it exists, it is not
    /// disabled, its owner is not taken down, and it has uses left.
    pub fn invite_available(&self, code: &str) -> Result<bool, StorageError> {
        invite_available(&self.conn, code)
    }

    /// Writes the rows of a new account in one transaction: the actor, the
    /// account, the use of the invite code, the first refresh token and
    /// the repository root.
    pub fn create_account(
        &mut self,
        new: &NewAccount<'_>,
    ) -> Result<Result<(), Refused>, StorageError> {
        let txn = self.conn.transaction()?;
        if let Some(code) = new.invite_code
            && !invite_available(&txn, code)?
        {
            return Ok(Err(Refused::Invite));
        }
        let deactivated_at = (new.deactivated || new.delete_after.is_some()).then_some(new.now);
        let actor = txn.execute(
            r#"insert into "actor" ("did", "handle", "createdAt", "deactivatedAt", "deleteAfter") values (?1, ?2, ?3, ?4, ?5) on conflict do nothing"#,
            params![new.did, new.handle, new.now, deactivated_at, new.delete_after],
        )?;
        if actor == 0 {
            return Ok(Err(Refused::Exists));
        }
        if let Some((email, hash)) = new.credentials {
            let account = txn.execute(
                r#"insert into "account" ("did", "email", "passwordScrypt") values (?1, ?2, ?3) on conflict do nothing"#,
                [new.did, &email.to_lowercase(), hash],
            )?;
            if account == 0 {
                return Ok(Err(Refused::Exists));
            }
        }
        if let Some(identity) = &new.external_identity {
            if !crate::linkjar::insert_identity(&txn, new.did, identity, new.now)? {
                return Ok(Err(Refused::Exists));
            }
            txn.execute(
                r#"update "account" set "emailConfirmedAt" = ?2 where "did" = ?1"#,
                [new.did, new.now],
            )?;
        }
        if let Some((request, client, device)) = new.signup
            && !crate::linkjar::register_signup(&txn, new.did, request, client, device, new.now)?
        {
            return Ok(Err(Refused::Authorization));
        }
        if new.handle_policy {
            txn.execute(
                r#"insert into "linkjar_handle_policy" ("did", "createdAt", "renameHandle") values (?1, ?2, null)"#,
                [new.did, new.now],
            )?;
        }
        if let Some(code) = new.invite_code {
            txn.execute(
                r#"insert into "invite_code_use" ("code", "usedBy", "usedAt") values (?1, ?2, ?3)"#,
                [code, new.did, new.now],
            )?;
        }
        if let Some((id, expires_at)) = new.refresh_token {
            txn.execute(
                r#"insert into "refresh_token" ("id", "did", "expiresAt") values (?1, ?2, ?3) on conflict do nothing"#,
                [id, new.did, expires_at],
            )?;
        }
        txn.execute(
            r#"insert into "repo_root" ("did", "cid", "rev", "indexedAt") values (?1, ?2, ?3, ?4) on conflict ("did") do update set "cid" = excluded."cid", "rev" = excluded."rev", "indexedAt" = excluded."indexedAt""#,
            [new.did, new.repo_root.0, new.repo_root.1, new.now],
        )?;
        txn.commit()?;
        Ok(Ok(()))
    }

    /// Removes every row of an account: for a creation that failed after
    /// its rows were written.
    pub fn delete_account(&mut self, did: &str) -> Result<(), StorageError> {
        let txn = self.conn.transaction()?;
        for table in [
            "repo_root",
            "email_token",
            "refresh_token",
            "linkjar_handle_policy",
            "external_identity",
            "sign_in_notice",
            "token",
            "account_device",
            "authorized_client",
            "account",
            "actor",
        ] {
            // A test directory may lack a table that nothing in it uses.
            let exists = txn.table_exists(table)?;
            if exists {
                txn.execute(&format!(r#"delete from "{table}" where "did" = ?1"#), [did])?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    /// Stores an app password. Returns `false`, and stores nothing, when
    /// the account has one of this name.
    pub fn create_app_password(
        &self,
        did: &str,
        name: &str,
        hash: &str,
        created_at: &str,
        privileged: bool,
    ) -> Result<bool, StorageError> {
        let stored = self.conn.execute(
            r#"insert into "app_password" ("did", "name", "passwordScrypt", "createdAt", "privileged") values (?1, ?2, ?3, ?4, ?5) on conflict do nothing"#,
            params![did, name, hash, created_at, i64::from(privileged)],
        )?;
        Ok(stored > 0)
    }

    /// The app passwords of `did`, the newest first: name, creation time
    /// and whether it is privileged.
    pub fn list_app_passwords(
        &self,
        did: &str,
    ) -> Result<Vec<(String, String, bool)>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select "name", "createdAt", "privileged" from "app_password" where "did" = ?1 order by "createdAt" desc"#,
        )?;
        let rows = statement.query_map([did], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<i64>(2)? == 1))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Removes an app password and the sessions it opened.
    pub fn revoke_app_password(&mut self, did: &str, name: &str) -> Result<(), StorageError> {
        let txn = self.conn.transaction()?;
        txn.execute(
            r#"delete from "app_password" where "did" = ?1 and "name" = ?2"#,
            [did, name],
        )?;
        txn.execute(
            r#"delete from "refresh_token" where "did" = ?1 and "appPasswordName" = ?2"#,
            [did, name],
        )?;
        txn.commit()?;
        Ok(())
    }

    /// Stores a refresh token. One that is stored already stays as it is:
    /// a token is granted again during the grace period of a refresh.
    pub fn store_refresh_token(
        &self,
        id: &str,
        did: &str,
        expires_at: &str,
        app_password: Option<&str>,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "refresh_token" ("id", "did", "appPasswordName", "expiresAt") values (?1, ?2, ?3, ?4) on conflict do nothing"#,
            params![id, did, app_password, expires_at],
        )?;
        Ok(())
    }

    /// The refresh token with this id.
    pub fn refresh_token(&self, id: &str) -> Result<Option<RefreshToken>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select refresh_token."did", refresh_token."expiresAt", refresh_token."nextId", refresh_token."appPasswordName", app_password."privileged" from "refresh_token" left join "app_password" on app_password."did" = refresh_token."did" and app_password."name" = refresh_token."appPasswordName" where "id" = ?1"#,
                [id],
                |row| {
                    let name: Option<String> = row.get(3)?;
                    let privileged: Option<i64> = row.get(4)?;
                    Ok(RefreshToken {
                        did: row.get(0)?,
                        expires_at: row.get(1)?,
                        next_id: row.get(2)?,
                        app_password: name.map(|name| AppPassword {
                            name,
                            privileged: privileged == Some(1),
                        }),
                    })
                },
            )
            .optional()?)
    }

    /// Removes the refresh tokens of `did` that ended at or before `now`.
    pub fn delete_expired_refresh_tokens(&self, did: &str, now: &str) -> Result<(), StorageError> {
        self.conn.execute(
            r#"delete from "refresh_token" where "did" = ?1 and "expiresAt" <= ?2"#,
            [did, now],
        )?;
        Ok(())
    }

    /// Replaces a refresh token: the old one ends at `expires_at` and
    /// names `next`, and `next` is stored. Returns `false`, and changes
    /// nothing, when the old token names another successor already.
    pub fn rotate_refresh_token(
        &mut self,
        id: &str,
        expires_at: &str,
        next: &str,
        next_expires_at: &str,
    ) -> Result<bool, StorageError> {
        let txn = self.conn.transaction()?;
        let updated = txn.execute(
            r#"update "refresh_token" set "expiresAt" = ?2, "nextId" = ?3 where "id" = ?1 and ("nextId" is null or "nextId" = ?3)"#,
            [id, expires_at, next],
        )?;
        if updated == 0 {
            return Ok(false);
        }
        txn.execute(
            r#"insert into "refresh_token" ("id", "did", "appPasswordName", "expiresAt") select ?2, "did", "appPasswordName", ?3 from "refresh_token" where "id" = ?1 on conflict do nothing"#,
            [id, next, next_expires_at],
        )?;
        txn.commit()?;
        Ok(true)
    }

    /// Removes a refresh token. Returns whether it was there.
    pub fn revoke_refresh_token(&self, id: &str) -> Result<bool, StorageError> {
        Ok(self
            .conn
            .execute(r#"delete from "refresh_token" where "id" = ?1"#, [id])?
            > 0)
    }

    /// Ends every legacy session of `did`. Returns how many it ended.
    pub fn revoke_refresh_tokens(&self, did: &str) -> Result<usize, StorageError> {
        Ok(self
            .conn
            .execute(r#"delete from "refresh_token" where "did" = ?1"#, [did])?)
    }

    /// The legacy sessions of `did`: the app password of each, if any, and
    /// when it ends.
    pub fn refresh_tokens_of(
        &self,
        did: &str,
    ) -> Result<Vec<(Option<String>, String)>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select "appPasswordName", "expiresAt" from "refresh_token" where "did" = ?1 order by "expiresAt""#,
        )?;
        let rows = statement.query_map([did], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Removes what has ended before `now`: legacy sessions, OAuth
    /// sessions and authorization requests. Returns how many rows went.
    pub fn purge_expired(&self, now: &str) -> Result<usize, StorageError> {
        let mut removed = self.conn.execute(
            r#"delete from "used_refresh_token" where "tokenId" in (select "id" from "token" where "expiresAt" < ?1)"#,
            [now],
        )?;
        for table in ["refresh_token", "token", "authorization_request"] {
            removed += self.conn.execute(
                &format!(r#"delete from "{table}" where "expiresAt" < ?1"#),
                [now],
            )?;
        }
        Ok(removed)
    }

    /// The number of actors.
    pub fn count_actors(&self) -> Result<u64, StorageError> {
        let count: i64 = self
            .conn
            .query_row(r#"select count(*) from "actor""#, [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// A page of hosted repositories in order of creation, then DID. `after`
    /// is the creation time and DID of the last entry of the previous page.
    pub fn list_repos(
        &self,
        limit: u32,
        after: Option<(&str, &str)>,
    ) -> Result<Vec<RepoListing>, StorageError> {
        let mut sql = format!(
            r#"select {ACTOR_COLUMNS}, repo_root."cid", repo_root."rev" from "actor" inner join "repo_root" on repo_root."did" = actor."did""#
        );
        if after.is_some() {
            sql.push_str(r#" where ((actor."createdAt", actor."did") > (:created, :did))"#);
        }
        sql.push_str(r#" order by actor."createdAt" asc, actor."did" asc limit :limit"#);
        let mut statement = self.conn.prepare(&sql)?;
        let map = |row: &Row<'_>| {
            Ok(RepoListing {
                actor: actor_from_row(row)?,
                head: row.get(5)?,
                rev: row.get(6)?,
            })
        };
        let rows = match after {
            Some((created, did)) => statement.query_map(
                named_params! { ":created": created, ":did": did, ":limit": limit },
                map,
            )?,
            None => statement.query_map(named_params! { ":limit": limit }, map)?,
        };
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every actor, in DID order. `verify` walks this list.
    pub fn all_actors(&self) -> Result<Vec<Actor>, StorageError> {
        let sql = format!(r#"select {ACTOR_COLUMNS} from "actor" order by actor."did" asc"#);
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map([], actor_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The commit CID and revision that `account.sqlite` records for an
    /// actor.
    pub fn repo_root(&self, did: &str) -> Result<Option<(String, String)>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "cid", "rev" from "repo_root" where "did" = :did"#,
                named_params! { ":did": did },
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }
}
