//! `account.sqlite`: the actors this server hosts and their status.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, named_params};

use crate::error::StorageError;
use crate::layout::open_read_only;

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

/// A read connection to `account.sqlite`.
#[derive(Debug)]
pub struct AccountDb {
    conn: Connection,
}

const ACTOR_COLUMNS: &str =
    r#"actor."did", actor."handle", actor."createdAt", actor."takedownRef", actor."deactivatedAt""#;

fn actor_from_row(row: &Row<'_>) -> rusqlite::Result<Actor> {
    Ok(Actor {
        did: row.get(0)?,
        handle: row.get(1)?,
        created_at: row.get(2)?,
        takedown_ref: row.get(3)?,
        deactivated_at: row.get(4)?,
    })
}

impl AccountDb {
    /// Opens the database for reading.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            conn: open_read_only(path)?,
        })
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
