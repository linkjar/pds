//! Compatible indexes on new and existing Reference actor stores.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

mod common;

use common::{ACTOR_SCHEMA, Scratch};
use pds_storage::ActorDb;
use pds_storage::schema::create_actor_store;
use rusqlite::Connection;

fn deletion_plan(conn: &Connection) -> String {
    conn.query_row(
        r#"explain query plan delete from "record_blob" where "recordUri" in (?1) returning "blobCid""#,
        ["at://did:plc:alice/com.example.note/one"],
        |row| row.get(3),
    )
    .unwrap()
}

#[test]
fn a_new_actor_store_indexes_blob_deletion_by_record_uri() {
    let scratch = Scratch::new();
    let conn = create_actor_store(&scratch.0.join("store.sqlite"), "2026-10-19T00:00:00Z").unwrap();
    let plan = deletion_plan(&conn);
    assert!(plan.contains("record_blob_record_uri_idx"), "{plan}");
    assert!(plan.contains("SEARCH"), "{plan}");
}

#[test]
fn opening_an_old_actor_for_writes_adds_the_index_without_changing_its_migrations() {
    let scratch = Scratch::new();
    let (path, conn) = scratch.database("store.sqlite", ACTOR_SCHEMA);
    conn.execute_batch(
        r#"create table "kysely_migration" ("name" varchar primary key, "timestamp" varchar not null);
           insert into "kysely_migration" values ('001', '2024-01-01T00:00:00Z');
           insert into "record_blob" values ('one', 'at://did:plc:alice/com.example.note/one');
           insert into "record_blob" values ('two', 'at://did:plc:alice/com.example.note/two');"#,
    )
    .unwrap();
    assert!(deletion_plan(&conn).contains("SCAN"));
    drop(ActorDb::open(&path).unwrap());
    assert!(deletion_plan(&conn).contains("SCAN"));
    drop(conn);

    drop(ActorDb::open_writable(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    let plan = deletion_plan(&conn);
    assert!(plan.contains("record_blob_record_uri_idx"), "{plan}");
    let mut db = ActorDb::open_writable(&path).unwrap();
    let migrations: Vec<(String, String)> = conn
        .prepare(r#"select "name", "timestamp" from "kysely_migration" order by "name""#)
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        migrations,
        vec![("001".to_owned(), "2024-01-01T00:00:00Z".to_owned())]
    );
    let txn = db.begin().unwrap();
    assert_eq!(
        txn.delete_record_blobs(&["at://did:plc:alice/com.example.note/one".to_owned()])
            .unwrap(),
        vec!["one"]
    );
    txn.commit().unwrap();
    assert_eq!(
        db.all_record_blobs().unwrap(),
        vec![(
            "two".to_owned(),
            "at://did:plc:alice/com.example.note/two".to_owned()
        )]
    );
}
