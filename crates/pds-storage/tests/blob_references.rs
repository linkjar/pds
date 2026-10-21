//! Blob retention depends on existence, not the number of references.

#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::Scratch;
use pds_storage::ActorDb;
use pds_storage::schema::create_actor_store;

#[test]
fn shared_blob_candidates_are_returned_once_after_replaced_records_are_removed() {
    let scratch = Scratch::new();
    let path = scratch.0.join("store.sqlite");
    drop(create_actor_store(&path, "2026-10-21T00:00:00Z").unwrap());
    let mut db = ActorDb::open_writable(&path).unwrap();
    let txn = db.begin().unwrap();
    for n in 0..10_000 {
        txn.associate_blob(
            "shared",
            &format!("at://did:plc:alice/com.example.note/r{n}"),
        )
        .unwrap();
    }
    let replaced = "at://did:plc:alice/com.example.note/replaced".to_owned();
    txn.associate_blob("shared", &replaced).unwrap();
    txn.associate_blob("orphan", &replaced).unwrap();
    txn.delete_record_blobs(&[replaced]).unwrap();

    // Repeated candidates cross the old query's batch boundary. Neither
    // fanout nor candidate repetition may multiply the result.
    let mut candidates = vec!["shared".to_owned(); 1001];
    candidates.extend(["orphan".to_owned(), "absent".to_owned()]);
    let retained = txn.referenced_blobs(&candidates).unwrap();
    assert_eq!(
        retained.len(),
        1,
        "return only distinct referenced candidates"
    );
    assert_eq!(retained, ["shared"]);
    assert_eq!(txn.referenced_blobs(&[]).unwrap(), Vec::<String>::new());
    assert!(!txn.has_records_for_blob("orphan").unwrap());
    txn.commit().unwrap();
    assert_eq!(db.all_record_blobs().unwrap().len(), 10_000);
}
