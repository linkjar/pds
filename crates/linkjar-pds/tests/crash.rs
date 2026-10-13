//! The boundary between a stored commit and its event (`docs/SPEC.md`
//! §7.4), with fail points. A commit and its event are in two files; when
//! the server stops between them, the repair at the next start states the
//! head with `#sync`.
//!
//! Fail points are process-wide, so this file holds one test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

mod common;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{ALICE, DataDir, JWT_SECRET};
use hmac::{Hmac, KeyInit, Mac};
use linkjar_pds::serve::app_state;
use linkjar_pds::verify::verify;
use pds_storage::fail::{self, Action};
use pds_storage::{ActorDb, OpsDb, SequencerDb};
use pds_xrpc::write::{Repaired, repair};
use serde_json::json;
use sha2::Sha256;

const NOTE: &str = "com.example.test.note";

fn token(did: &str) -> String {
    let head = URL_SAFE_NO_PAD.encode(json!({"alg": "HS256", "typ": "at+jwt"}).to_string());
    let claims = json!({"scope": "com.atproto.access", "sub": did, "aud": "did:web:pds.test", "exp": 9_999_999_999_u64});
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let mut mac = Hmac::<Sha256>::new_from_slice(JWT_SECRET.as_bytes()).unwrap();
    mac.update(format!("{head}.{body}").as_bytes());
    format!(
        "Bearer {head}.{body}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    )
}

/// A server on `dir`, as a fresh process would start it: new state, then
/// the repair.
async fn start(dir: &DataDir) -> (String, Repaired) {
    let state = app_state(&dir.config).unwrap();
    let repairing = state.clone();
    let repaired = tokio::task::spawn_blocking(move || repair(&repairing))
        .await
        .unwrap()
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, pds_xrpc::router(state)).await });
    (origin, repaired)
}

async fn create(origin: &str, rkey: &str) -> u16 {
    reqwest::Client::new()
        .post(format!("{origin}/xrpc/com.atproto.repo.createRecord"))
        .header("authorization", token(ALICE))
        .header("content-type", "application/json")
        .body(
            json!({"repo": ALICE, "collection": NOTE, "rkey": rkey, "record": {"$type": NOTE}})
                .to_string(),
        )
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

fn head(dir: &DataDir) -> String {
    ActorDb::open(&dir.config.layout.actor_db(ALICE))
        .unwrap()
        .root()
        .unwrap()
        .unwrap()
        .rev
}

fn events(dir: &DataDir) -> Vec<(String, Option<String>)> {
    SequencerDb::open(&dir.config.layout.sequencer_db)
        .unwrap()
        .range(3, 100)
        .unwrap()
        .into_iter()
        .map(|row| (row.event_type, pds_sequencer::event_rev(&row.event)))
        .collect()
}

fn notes(dir: &DataDir) -> usize {
    OpsDb::open(&dir.config.layout.ops_db)
        .unwrap()
        .pending_commits()
        .unwrap()
        .len()
}

#[tokio::test]
async fn a_stop_between_the_commit_and_its_event_is_repaired_with_sync() {
    let dir = DataDir::new(1);
    let (origin, repaired) = start(&dir).await;
    assert_eq!(repaired, Repaired::default());
    let seeded = head(&dir);

    // Before the commit is stored: nothing happened, and the note says a
    // commit was about to be.
    fail::arm("commit.before-store", Action::Error);
    assert_eq!(create(&origin, "a").await, 500);
    fail::disarm_all();
    assert_eq!(head(&dir), seeded);
    assert_eq!(events(&dir), []);
    assert_eq!(notes(&dir), 1);
    let (origin, repaired) = start(&dir).await;
    assert_eq!(
        repaired,
        Repaired {
            settled: 1,
            synced: 0
        }
    );
    assert_eq!(events(&dir), []);
    assert_eq!(notes(&dir), 0);

    // After the commit is stored and before its event: the head is ahead
    // of the log.
    fail::arm("commit.stored", Action::Error);
    assert_eq!(create(&origin, "b").await, 500);
    fail::disarm_all();
    let ahead = head(&dir);
    assert!(ahead > seeded);
    assert_eq!(events(&dir), []);
    let (origin, repaired) = start(&dir).await;
    assert_eq!(
        repaired,
        Repaired {
            settled: 0,
            synced: 1
        }
    );
    assert_eq!(events(&dir), [("sync".to_owned(), Some(ahead.clone()))]);
    assert_eq!(notes(&dir), 0);
    // The repair runs once.
    let (origin_again, repaired) = start(&dir).await;
    assert_eq!(repaired, Repaired::default());
    drop(origin_again);

    // After the event: only the note is left, and the repair removes it.
    fail::arm("commit.sequenced", Action::Error);
    assert_eq!(create(&origin, "c").await, 500);
    fail::disarm_all();
    let sequenced = head(&dir);
    assert_eq!(events(&dir).len(), 2);
    assert_eq!(events(&dir)[1], ("append".to_owned(), Some(sequenced)));
    assert_eq!(notes(&dir), 1);
    let (origin, repaired) = start(&dir).await;
    assert_eq!(
        repaired,
        Repaired {
            settled: 1,
            synced: 0
        }
    );
    assert_eq!(events(&dir).len(), 2);

    // The next write follows the repaired head.
    assert_eq!(create(&origin, "d").await, 200);
    assert_eq!(events(&dir).len(), 3);
    let report = verify(&dir.config).unwrap();
    assert!(report.findings.is_empty(), "{:#?}", report.findings);
}
