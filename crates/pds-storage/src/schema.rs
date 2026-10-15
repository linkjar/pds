//! The Reference schema of the databases this crate writes
//! (`docs/SPEC.md` §8.2).
//!
//! The statements are the ones the Reference's migrations run at the pin
//! (`actor-store/db/migrations/001-init.ts`,
//! `sequencer/db/migrations/001-init.ts` and
//! `did-cache/db/migrations/001-init.ts`), so a database created here is one
//! the Reference serves. The statements of the shared databases are in
//! `schema_shared`. `DataLayout::create_missing` creates the files a data
//! directory lacks; the simulation and the tests use this module directly.

use std::path::Path;

use rusqlite::Connection;

use crate::error::StorageError;

/// The tables and indexes of one actor's `store.sqlite`.
pub const ACTOR_STORE: &str = r#"
create table "repo_root" ("did" varchar primary key, "cid" varchar not null, "rev" varchar not null, "indexedAt" varchar not null);
create table "repo_block" ("cid" varchar primary key, "repoRev" varchar not null, "size" integer not null, "content" blob not null);
create index "repo_block_repo_rev_idx" on "repo_block" ("repoRev", "cid");
create table "record" ("uri" varchar primary key, "cid" varchar not null, "collection" varchar not null, "rkey" varchar not null, "repoRev" varchar not null, "indexedAt" varchar not null, "takedownRef" varchar);
create index "record_cid_idx" on "record" ("cid");
create index "record_collection_idx" on "record" ("collection");
create index "record_repo_rev_idx" on "record" ("repoRev");
create table "blob" ("cid" varchar primary key, "mimeType" varchar not null, "size" integer not null, "tempKey" varchar, "width" integer, "height" integer, "createdAt" varchar not null, "takedownRef" varchar);
create index "blob_tempkey_idx" on "blob" ("tempKey");
create table "record_blob" ("blobCid" varchar not null, "recordUri" varchar not null, constraint "record_blob_pkey" primary key ("blobCid", "recordUri"));
create table "backlink" ("uri" varchar not null, "path" varchar not null, "linkTo" varchar not null, constraint "backlinks_pkey" primary key ("uri", "path"));
create index "backlink_link_to_idx" on "backlink" ("path", "linkTo");
create table "account_pref" ("id" integer primary key autoincrement, "name" varchar not null, "valueJson" text not null);
"#;

/// The table and indexes of `sequencer.sqlite`.
pub const SEQUENCER: &str = r#"
create table "repo_seq" ("seq" integer primary key autoincrement, "did" varchar not null, "eventType" varchar not null, "event" blob not null, "invalidated" int2 default 0 not null, "sequencedAt" varchar not null);
create index "repo_seq_did_idx" on "repo_seq" ("did");
create index "repo_seq_event_type_idx" on "repo_seq" ("eventType");
create index "repo_seq_sequenced_at_index" on "repo_seq" ("sequencedAt");
"#;

/// The table of `did_cache.sqlite`.
pub const DID_CACHE: &str = r#"
create table "did_doc" ("did" varchar primary key, "doc" text not null, "updatedAt" bigint not null);
"#;

/// Creates a database at `path` with `schema`, in WAL mode as the Reference
/// keeps its databases (SPEC §8.3). The file must not exist.
pub fn create(path: &Path, schema: &str) -> Result<Connection, StorageError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| StorageError::io(dir, source))?;
    }
    if path.exists() {
        return Err(StorageError::io(
            path,
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, "the database exists"),
        ));
    }
    let conn = crate::io::open(path, rusqlite::OpenFlags::default())?;
    conn.pragma_update(None, "journal_mode", "wal")?;
    conn.execute_batch(schema)?;
    Ok(conn)
}

/// The bookkeeping tables of the Reference's migrator. The Reference
/// finds its migrations recorded and runs none.
const MIGRATOR: &str = r#"
create table "kysely_migration" ("name" varchar(255) not null primary key, "timestamp" varchar(255) not null);
create table "kysely_migration_lock" ("id" varchar(255) not null primary key, "is_locked" integer default 0 not null);
insert into "kysely_migration_lock" ("id", "is_locked") values ('migration_lock', 0);
"#;

fn record_migrations(conn: &Connection, names: &[&str], now: &str) -> Result<(), StorageError> {
    for name in names {
        conn.execute(
            r#"insert into "kysely_migration" ("name", "timestamp") values (?1, ?2)"#,
            [name, &now],
        )?;
    }
    Ok(())
}

/// Creates one of the shared databases as the Reference creates it at the
/// pin (`schema_shared`), with its migrations recorded at `now`. The file
/// must not exist.
pub fn create_shared(
    path: &Path,
    shared: &crate::schema_shared::Shared,
    now: &str,
) -> Result<Connection, StorageError> {
    let conn = create(path, "")?;
    for statement in shared.statements {
        conn.execute_batch(statement)?;
    }
    conn.execute(
        r#"insert into "kysely_migration_lock" ("id", "is_locked") values ('migration_lock', 0)"#,
        [],
    )?;
    record_migrations(&conn, shared.migrations, now)?;
    Ok(conn)
}

/// Creates the tables of a shared database over a connection of the
/// shared tier, with its migrations recorded at `now`. On Postgres the
/// statements get the types of Postgres.
pub fn fill_shared(
    conn: &crate::sql::Connection,
    shared: &crate::schema_shared::Shared,
    now: &str,
) -> Result<(), StorageError> {
    for statement in shared.statements {
        conn.execute_batch(statement)?;
    }
    conn.execute(
        r#"insert into "kysely_migration_lock" ("id", "is_locked") values ('migration_lock', 0)"#,
        [],
    )?;
    for name in shared.migrations {
        conn.execute(
            r#"insert into "kysely_migration" ("name", "timestamp") values (?1, ?2)"#,
            [name, &now],
        )?;
    }
    Ok(())
}

/// Creates the store of a new actor as the Reference creates it: the
/// tables of [`ACTOR_STORE`] and its one migration, recorded at `now`.
pub fn create_actor_store(path: &Path, now: &str) -> Result<Connection, StorageError> {
    let conn = create(path, ACTOR_STORE)?;
    conn.execute_batch(MIGRATOR)?;
    record_migrations(&conn, &["001"], now)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_schema_is_valid_sql_and_a_database_is_not_created_twice() {
        let dir = std::env::temp_dir().join(format!("pds-schema-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (name, schema) in [
            ("store.sqlite", ACTOR_STORE),
            ("sequencer.sqlite", SEQUENCER),
            ("did_cache.sqlite", DID_CACHE),
        ] {
            let path = dir.join(name);
            let conn = create(&path, schema).unwrap();
            let mode: String = conn
                .query_row("pragma journal_mode", [], |row| row.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
            drop(conn);
            assert!(create(&path, schema).is_err());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
