//! A scratch directory and the tables of the Reference's schema that the
//! tests of this crate read. The whole schema is in
//! `crates/linkjar-pds/tests/common/mod.rs`.

#![allow(dead_code)] // each test file uses a part of this module

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use rusqlite::Connection;

pub const ACCOUNT_SCHEMA: &str = r#"
CREATE TABLE "actor" ("did" varchar primary key, "handle" varchar, "createdAt" varchar not null, "takedownRef" varchar, "deactivatedAt" varchar, "deleteAfter" varchar);
CREATE TABLE "repo_root" ("did" varchar primary key, "cid" varchar not null, "rev" varchar not null, "indexedAt" varchar not null);
"#;
pub const ACTOR_SCHEMA: &str = r#"
CREATE TABLE "repo_block" ("cid" varchar primary key, "repoRev" varchar not null, "size" integer not null, "content" blob not null);
CREATE TABLE "record" ("uri" varchar primary key, "cid" varchar not null, "collection" varchar not null, "rkey" varchar not null, "repoRev" varchar not null, "indexedAt" varchar not null, "takedownRef" varchar);
CREATE TABLE "record_blob" ("blobCid" varchar not null, "recordUri" varchar not null, constraint "record_blob_pkey" primary key ("blobCid", "recordUri"));
"#;
pub const SEQUENCER_SCHEMA: &str = r#"
CREATE TABLE "repo_seq" ("seq" integer primary key autoincrement, "did" varchar not null, "eventType" varchar not null, "event" blob not null, "invalidated" int2 default 0 not null, "sequencedAt" varchar not null);
"#;
pub const DID_CACHE_SCHEMA: &str = r#"
CREATE TABLE "did_doc" ("did" varchar primary key, "doc" text not null, "updatedAt" bigint not null);
"#;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A directory that is removed when the value is dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pds-storage-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Creates the database `name` with `schema` and returns its path and a
    /// write connection to it.
    pub fn database(&self, name: &str, schema: &str) -> (PathBuf, Connection) {
        let path = self.0.join(name);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(schema).unwrap();
        (path, conn)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
