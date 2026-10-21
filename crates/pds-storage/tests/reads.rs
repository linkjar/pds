//! The branches of the read queries that depend on a cursor or on `since`,
//! the two calls that report missing blocks, and the DID cache.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

mod common;

use cid::Cid;
use cid::multihash::Multihash;
use common::{ACCOUNT_SCHEMA, ACTOR_SCHEMA, DID_CACHE_SCHEMA, SEQUENCER_SCHEMA, Scratch};
use pds_storage::{AccountDb, ActorDb, DidCacheDb, SequencerDb};
use rusqlite::params;
use sha2::{Digest, Sha256};

/// The CID of the DAG-CBOR block `bytes`.
fn cid_of(bytes: &[u8]) -> Cid {
    let hash = Multihash::<64>::wrap(0x12, &Sha256::digest(bytes)).unwrap();
    Cid::new_v1(0x71, hash)
}

#[test]
fn export_pages_bound_bytes_without_skipping_blocks() {
    let scratch = Scratch::new();
    let (path, conn) = scratch.database("actor.sqlite", ACTOR_SCHEMA);
    for id in 0..12 {
        conn.execute(
            r#"insert into "repo_block" values (?1, 'rev', 1000000, zeroblob(1000000))"#,
            [format!("block-{id:02}")],
        )
        .unwrap();
    }
    let store = ActorDb::open(&path).unwrap();
    let mut after: Option<(String, String)> = None;
    let mut seen = Vec::new();
    loop {
        let page = store
            .block_page(
                None,
                after
                    .as_ref()
                    .map(|(rev, cid)| (rev.as_str(), cid.as_str())),
            )
            .unwrap();
        if page.is_empty() {
            break;
        }
        assert!(page.iter().map(|row| row.content.len()).sum::<usize>() <= 4 * 1024 * 1024);
        let last = page.last().unwrap();
        after = Some((last.rev.clone(), last.cid.clone()));
        seen.extend(page.into_iter().map(|row| row.cid));
    }
    assert_eq!(
        seen,
        (0..12)
            .rev()
            .map(|id| format!("block-{id:02}"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn firehose_pages_stop_before_the_next_event_exceeds_the_byte_budget() {
    let scratch = Scratch::new();
    let (path, conn) = scratch.database("sequencer.sqlite", SEQUENCER_SCHEMA);
    for size in [7, 7, 20, 1] {
        conn.execute(
            r#"insert into "repo_seq" ("did", "eventType", "event", "sequencedAt") values ('did:plc:a', 'append', zeroblob(?1), 't')"#,
            [size],
        ).unwrap();
    }
    let db = SequencerDb::open(&path).unwrap();
    let (page, end) = db.page(0, 64, 10).unwrap();
    assert_eq!(page.iter().map(|row| row.seq).collect::<Vec<_>>(), [1]);
    assert!(!end);
    assert_eq!(db.page(1, 64, 10).unwrap().0[0].seq, 2);
    let (page, end) = db.page(2, 64, 10).unwrap();
    assert_eq!(page.len(), 1); // An oversized existing event still makes progress.
    assert_eq!(page[0].event.len(), 20);
    assert!(!end);
    assert!(db.page(3, 64, 10).unwrap().1);
    assert_eq!(db.page(0, 0, 10).unwrap().0, []);
}

#[test]
fn list_repos_pages_by_creation_time_then_did() {
    let scratch = Scratch::new();
    let (path, conn) = scratch.database("account.sqlite", ACCOUNT_SCHEMA);
    // Two actors share a creation time, so the DID decides their order.
    let actors = [
        (
            "did:plc:cccccccccccccccccccccccc",
            "2026-01-01T00:00:00.000Z",
        ),
        (
            "did:plc:bbbbbbbbbbbbbbbbbbbbbbbb",
            "2026-01-02T00:00:00.000Z",
        ),
        (
            "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa",
            "2026-01-02T00:00:00.000Z",
        ),
    ];
    for (did, created) in actors {
        conn.execute(
            r#"insert into "actor" values (?1, null, ?2, null, null, null)"#,
            params![did, created],
        )
        .unwrap();
        conn.execute(
            r#"insert into "repo_root" values (?1, 'cid', 'rev', ?2)"#,
            params![did, created],
        )
        .unwrap();
    }
    let db = AccountDb::open(&path).unwrap();

    let mut seen = Vec::new();
    let mut after: Option<(String, String)> = None;
    loop {
        let cursor = after.as_ref().map(|(c, d)| (c.as_str(), d.as_str()));
        let page = db.list_repos(1, cursor).unwrap();
        let Some(entry) = page.first() else { break };
        assert_eq!(page.len(), 1);
        seen.push(entry.actor.did.clone());
        after = Some((entry.actor.created_at.clone(), entry.actor.did.clone()));
    }
    assert_eq!(
        seen,
        [
            "did:plc:cccccccccccccccccccccccc",
            "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa",
            "did:plc:bbbbbbbbbbbbbbbbbbbbbbbb",
        ]
    );
    // A cursor between the two creation times returns both later actors.
    let rest = db
        .list_repos(10, Some(("2026-01-01T12:00:00.000Z", "did:plc:zzz")))
        .unwrap();
    assert_eq!(rest.len(), 2);
}

#[test]
fn list_blobs_applies_since_and_cursor() {
    let scratch = Scratch::new();
    let (path, conn) = scratch.database("store.sqlite", ACTOR_SCHEMA);
    // Three records at revisions r1 to r3. Blob b2 belongs to two of them.
    for (rkey, rev, blob) in [("k1", "r1", "b1"), ("k2", "r2", "b2"), ("k3", "r3", "b3")] {
        let uri = format!("at://did:plc:a/c.d.e/{rkey}");
        conn.execute(
            r#"insert into "record" values (?1, 'cid', 'c.d.e', ?2, ?3, 't', null)"#,
            params![uri, rkey, rev],
        )
        .unwrap();
        conn.execute(
            r#"insert into "record_blob" values (?1, ?2)"#,
            params![blob, uri],
        )
        .unwrap();
    }
    conn.execute(
        r#"insert into "record_blob" values ('b2', 'at://did:plc:a/c.d.e/k3')"#,
        [],
    )
    .unwrap();
    let db = ActorDb::open(&path).unwrap();

    assert_eq!(db.list_blobs(None, None, 10).unwrap(), ["b1", "b2", "b3"]);
    // `since` keeps the blobs of records written after that revision.
    assert_eq!(
        db.list_blobs(Some("r0"), None, 10).unwrap(),
        ["b1", "b2", "b3"]
    );
    assert_eq!(db.list_blobs(Some("r1"), None, 10).unwrap(), ["b2", "b3"]);
    assert_eq!(db.list_blobs(Some("r2"), None, 10).unwrap(), ["b2", "b3"]);
    assert_eq!(
        db.list_blobs(Some("r3"), None, 10).unwrap(),
        [] as [String; 0]
    );
    // The cursor is the last CID of the previous page.
    assert_eq!(db.list_blobs(None, Some("b1"), 10).unwrap(), ["b2", "b3"]);
    assert_eq!(db.list_blobs(None, Some("b1"), 1).unwrap(), ["b2"]);
    assert_eq!(
        db.list_blobs(None, Some("b3"), 10).unwrap(),
        [] as [String; 0]
    );
    assert_eq!(db.list_blobs(Some("r1"), Some("b2"), 10).unwrap(), ["b3"]);
}

#[test]
fn blocks_reports_the_missing_cids_in_the_order_asked() {
    let scratch = Scratch::new();
    let (path, conn) = scratch.database("store.sqlite", ACTOR_SCHEMA);
    // 1,200 CIDs span three queries of 500. Every third one is not stored.
    let cids: Vec<Cid> = (0u32..1200).map(|n| cid_of(&n.to_be_bytes())).collect();
    let mut want_missing = Vec::new();
    for (n, cid) in cids.iter().enumerate() {
        if n % 3 == 0 {
            want_missing.push(*cid);
        } else {
            conn.execute(
                r#"insert into "repo_block" values (?1, 'r1', 4, ?2)"#,
                params![cid.to_string(), n.to_be_bytes().as_slice()],
            )
            .unwrap();
        }
    }
    let db = ActorDb::open(&path).unwrap();

    let blocks = db.blocks(&cids).unwrap();
    assert_eq!(blocks.missing, want_missing);
    assert_eq!(blocks.found.len(), 800);
    for (cid, content) in &blocks.found {
        let n = cids.iter().position(|have| have == cid).unwrap();
        assert_eq!(content, &n.to_be_bytes());
    }
    assert_eq!(db.missing_blocks(&cids).unwrap(), want_missing);
    assert_eq!(db.missing_blocks(&[]).unwrap(), [] as [Cid; 0]);
    assert_eq!(
        db.missing_blocks(&[cids[1], cids[2]]).unwrap(),
        [] as [Cid; 0]
    );
}

#[test]
fn the_did_cache_replaces_and_removes_a_document() {
    let scratch = Scratch::new();
    let (path, conn) = scratch.database("did_cache.sqlite", DID_CACHE_SCHEMA);
    drop(conn);
    let db = DidCacheDb::open(&path).unwrap();
    let did = "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa";

    assert_eq!(db.get(did).unwrap(), None);
    db.put(did, r#"{"v":1}"#, 100).unwrap();
    db.put(did, r#"{"v":2}"#, 200).unwrap();
    let cached = db.get(did).unwrap().unwrap();
    assert_eq!(cached.doc, r#"{"v":2}"#);
    assert_eq!(cached.updated_at_ms, 200);
    // Another DID is untouched by a removal.
    db.put("did:plc:bbbbbbbbbbbbbbbbbbbbbbbb", "{}", 300)
        .unwrap();
    db.clear(did).unwrap();
    assert_eq!(db.get(did).unwrap(), None);
    assert!(
        db.get("did:plc:bbbbbbbbbbbbbbbbbbbbbbbb")
            .unwrap()
            .is_some()
    );
    // Removing an absent document is not an error.
    db.clear(did).unwrap();
}

#[test]
fn the_sequencer_counts_events_and_reports_breaks() {
    let scratch = Scratch::new();
    let (path, conn) = scratch.database("sequencer.sqlite", SEQUENCER_SCHEMA);
    let db = SequencerDb::open(&path).unwrap();
    assert_eq!(db.event_count().unwrap(), 0);
    for seq in [1, 2, 5, 6] {
        conn.execute(
            r#"insert into "repo_seq" ("seq", "did", "eventType", "event", "sequencedAt") values (?1, 'did:plc:a', 'append', x'00', 't')"#,
            params![seq],
        )
        .unwrap();
    }
    assert_eq!(db.event_count().unwrap(), 4);
    let gaps = db.gaps().unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!((gaps[0].after, gaps[0].before), (2, 5));
}
