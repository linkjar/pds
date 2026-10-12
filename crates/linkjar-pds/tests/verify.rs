//! `verify` on a data directory in the Reference's schema: a clean one has
//! no differences, and each kind of damage is named.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

mod common;

use common::{ALICE, BLOB_CID, DANA, DataDir, POST};
use linkjar_pds::verify::{Report, verify};
use rusqlite::params;

fn kinds(report: &Report) -> Vec<(&str, &str)> {
    report
        .findings
        .iter()
        .map(|finding| (finding.subject.as_str(), finding.kind))
        .collect()
}

#[test]
fn a_clean_data_directory_has_no_differences() {
    let dir = DataDir::new(40);
    let report = verify(&dir.config).unwrap();
    assert_eq!(report.findings, []);
    assert_eq!(
        (report.actors, report.repositories, report.records),
        (3, 3, 42)
    );
    assert_eq!((report.blob_references, report.events), (1, 3));
    assert!(report.blob_store_checked);
    assert_eq!(report.to_json()["differences"], 0);
}

#[test]
fn a_record_missing_from_the_index_is_found_twice_over() {
    let dir = DataDir::new(40);
    dir.actor(ALICE)
        .execute(r#"delete from "record" where "rkey" = 'r0007'"#, [])
        .unwrap();
    // The tree holds a key the index lacks, and the index no longer gives
    // the commit's root.
    assert_eq!(
        kinds(&verify(&dir.config).unwrap()),
        [
            (ALICE, "record-index-mismatch"),
            (ALICE, "root-recompute-mismatch")
        ]
    );
}

#[test]
fn a_missing_record_block_is_found() {
    let dir = DataDir::new(3);
    let store = dir.actor(ALICE);
    let cid: String = store
        .query_row(
            r#"select "cid" from "record" where "rkey" = 'r0001'"#,
            [],
            |row| row.get(0),
        )
        .unwrap();
    store
        .execute(r#"delete from "repo_block" where "cid" = ?1"#, params![cid])
        .unwrap();
    assert_eq!(
        kinds(&verify(&dir.config).unwrap()),
        [(ALICE, "record-block-missing")]
    );
}

#[test]
fn a_missing_tree_node_is_found() {
    let dir = DataDir::new(3);
    let store = dir.actor(ALICE);
    // The tree nodes are the blocks that are neither records nor the commit.
    store
        .execute(
            r#"delete from "repo_block" where "cid" not in (select "cid" from "record") and "cid" not in (select "cid" from "repo_root")"#,
            [],
        )
        .unwrap();
    assert_eq!(
        kinds(&verify(&dir.config).unwrap()),
        [(ALICE, "tree-block-missing")]
    );
}

#[test]
fn a_key_that_did_not_sign_the_commit_is_found() {
    let dir = DataDir::new(3);
    std::fs::write(dir.config.layout.actor_key(ALICE), [7_u8; 32]).unwrap();
    assert_eq!(
        kinds(&verify(&dir.config).unwrap()),
        [(ALICE, "signature-invalid")]
    );
    std::fs::write(dir.config.layout.actor_key(ALICE), [0_u8; 32]).unwrap();
    assert_eq!(
        kinds(&verify(&dir.config).unwrap()),
        [(ALICE, "key-unreadable")]
    );
    std::fs::remove_file(dir.config.layout.actor_key(ALICE)).unwrap();
    assert_eq!(
        kinds(&verify(&dir.config).unwrap()),
        [(ALICE, "key-unreadable")]
    );
}

#[test]
fn a_root_that_the_account_database_does_not_agree_on_is_found() {
    let dir = DataDir::new(3);
    common::open(&dir.config.layout.account_db)
        .execute(
            r#"update "repo_root" set "rev" = '3zzzzzzzzzzzz' where "did" = ?1"#,
            params![DANA],
        )
        .unwrap();
    assert_eq!(
        kinds(&verify(&dir.config).unwrap()),
        [(DANA, "root-mismatch")]
    );
}

#[test]
fn a_missing_store_and_a_missing_root_are_found() {
    let dir = DataDir::new(3);
    dir.actor(ALICE)
        .execute(r#"delete from "repo_root""#, [])
        .unwrap();
    std::fs::remove_file(dir.config.layout.actor_db(DANA)).unwrap();
    assert_eq!(
        kinds(&verify(&dir.config).unwrap()),
        [(ALICE, "root-missing"), (DANA, "store-missing")]
    );
}

#[test]
fn a_blob_that_is_referenced_and_absent_is_found() {
    let dir = DataDir::new(3);
    std::fs::remove_file(dir.root.join("blocks").join(ALICE).join(BLOB_CID)).unwrap();
    assert_eq!(
        kinds(&verify(&dir.config).unwrap()),
        [(ALICE, "blob-missing")]
    );
    // A blob that is taken down is in quarantine, and its absence is expected.
    dir.actor(ALICE)
        .execute(r#"update "blob" set "takedownRef" = 'mod-2'"#, [])
        .unwrap();
    assert_eq!(kinds(&verify(&dir.config).unwrap()), []);
    dir.actor(ALICE)
        .execute(r#"delete from "blob""#, [])
        .unwrap();
    assert_eq!(
        kinds(&verify(&dir.config).unwrap()),
        [(ALICE, "blob-unindexed")]
    );
}

#[test]
fn a_break_in_the_event_log_is_found() {
    let dir = DataDir::new(3);
    common::open(&dir.config.layout.sequencer_db)
        .execute(r#"delete from "repo_seq" where "seq" = 2"#, [])
        .unwrap();
    let report = verify(&dir.config).unwrap();
    assert_eq!(kinds(&report), [("sequencer", "sequence-gap")]);
    assert_eq!(report.findings[0].detail, "no event between 1 and 3");
}

#[test]
fn a_record_of_another_repository_in_the_index_is_found() {
    let dir = DataDir::new(3);
    dir.actor(ALICE)
        .execute(
            r#"update "record" set "uri" = ?1 where "rkey" = 'r0002'"#,
            params![format!("at://{DANA}/{POST}/r0002")],
        )
        .unwrap();
    let report = verify(&dir.config).unwrap();
    assert_eq!(kinds(&report).first(), Some(&(ALICE, "record-uri")));
}

/// A tree whose root points at one node from both sides of its key: the
/// shape an unpatched Reference accepts on import (SPEC §2.3, DD-6).
#[test]
fn a_tree_that_shares_a_node_is_named_as_such() {
    use pds_repo::UnsignedCommit;
    use pds_repo::mst::node::{WireEntry, WireNode, layer_of};
    use pds_types::content::cid_for_cbor;

    let dir = DataDir::new(1);
    let store = dir.actor(ALICE);
    let record: String = store
        .query_row(r#"select "cid" from "record" limit 1"#, [], |row| {
            row.get(0)
        })
        .unwrap();
    let record = cid::Cid::try_from(record.as_str()).unwrap();
    // A key of the lower layer for the shared node, and one of the layer
    // above for the root.
    let key_at = |layer: u32| {
        (0..10_000)
            .map(|i| format!("{POST}/k{i:04}"))
            .find(|key| layer_of(key.as_bytes()) == layer)
            .unwrap()
    };
    let entry = |key: String, tree| WireEntry {
        key,
        value: record,
        tree,
    };
    let shared = WireNode {
        left: None,
        entries: vec![entry(key_at(0), None)],
    }
    .encode();
    let shared_cid = cid_for_cbor(&shared);
    let root = WireNode {
        left: Some(shared_cid),
        entries: vec![entry(key_at(1), Some(shared_cid))],
    }
    .encode();
    let root_cid = cid_for_cbor(&root);
    let keypair = common::key(0);
    let commit = UnsignedCommit::new(
        ALICE.parse().unwrap(),
        "3aaaaaaaaaaa7".parse().unwrap(),
        root_cid,
    )
    .sign(|bytes| Ok::<_, std::convert::Infallible>(keypair.sign(bytes).to_vec()))
    .unwrap();
    for (cid, bytes) in [
        (shared_cid, shared),
        (root_cid, root),
        (commit.cid(), commit.to_bytes()),
    ] {
        store
            .execute(
                r#"insert into "repo_block" values (?1, '3aaaaaaaaaaa7', ?2, ?3)"#,
                params![cid.to_string(), i64::try_from(bytes.len()).unwrap(), bytes],
            )
            .unwrap();
    }
    let head = commit.cid().to_string();
    store
        .execute(
            r#"update "repo_root" set "cid" = ?1, "rev" = '3aaaaaaaaaaa7'"#,
            params![head],
        )
        .unwrap();
    common::open(&dir.config.layout.account_db)
        .execute(
            r#"update "repo_root" set "cid" = ?1, "rev" = '3aaaaaaaaaaa7' where "did" = ?2"#,
            params![head, ALICE],
        )
        .unwrap();

    let report = verify(&dir.config).unwrap();
    let found = kinds(&report);
    assert!(found.contains(&(ALICE, "tree-shared-node")), "{found:?}");
    assert!(!found.contains(&(ALICE, "signature-invalid")), "{found:?}");
}
