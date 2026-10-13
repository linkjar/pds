//! Writes and the firehose over a data directory in the Reference's schema,
//! through a real listener (`docs/SPEC.md` §6.3, §6.4, §7).
//!
//! The parity harness holds these answers to the Reference's. These tests
//! hold what can be checked without one: that a write is stored, indexed and
//! sequenced, that `verify` accepts the directory afterwards, and that a
//! subscriber receives each commit once and in order.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

mod common;

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{ALICE, DANA, DataDir, JWT_SECRET, POST};
use futures_util::StreamExt;
use hmac::{Hmac, KeyInit, Mac};
use linkjar_pds::serve::app_state;
use linkjar_pds::verify::verify;
use pds_repo::{Commit, read_car};
use pds_types::cbor::Reader;
use pds_types::{Data, Limits};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio_tungstenite::tungstenite::Message;

const NOTE: &str = "com.example.test.note";

struct Server {
    origin: String,
    http: reqwest::Client,
    state: pds_xrpc::AppState,
    dir: DataDir,
}

struct Answer {
    status: u16,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    fn error(&self) -> (u16, String) {
        (
            self.status,
            self.json()["error"].as_str().unwrap_or_default().to_owned(),
        )
    }

    fn ok(self) -> Value {
        assert_eq!(self.status, 200, "{}", String::from_utf8_lossy(&self.body));
        self.json()
    }
}

/// A legacy access token as the Reference mints it.
fn token(did: &str) -> String {
    let head = URL_SAFE_NO_PAD.encode(json!({"alg": "HS256", "typ": "at+jwt"}).to_string());
    let claims = json!({"scope": "com.atproto.access", "sub": did, "aud": "did:web:pds.test", "iat": 1, "exp": 9_999_999_999_u64});
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let mut mac = Hmac::<Sha256>::new_from_slice(JWT_SECRET.as_bytes()).unwrap();
    mac.update(format!("{head}.{body}").as_bytes());
    format!(
        "Bearer {head}.{body}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    )
}

fn post(text: &str) -> Value {
    json!({"$type": POST, "text": text, "createdAt": "2026-01-01T00:00:00.000Z"})
}

/// One decoded frame of the firehose.
#[derive(Debug)]
struct Frame {
    kind: String,
    body: Data,
}

impl Frame {
    fn text(&self, key: &str) -> &str {
        match self.body.get(key) {
            Some(Data::String(text)) => text,
            other => panic!("{key}: {other:?}"),
        }
    }

    fn seq(&self) -> i64 {
        match self.body.get("seq") {
            Some(Data::Integer(seq)) => *seq,
            other => panic!("seq: {other:?}"),
        }
    }
}

fn decode(bytes: &[u8]) -> Frame {
    let mut reader = Reader::new(bytes, Limits::FRAME).unwrap();
    let head = reader.data(0).unwrap();
    let body = reader.data(0).unwrap();
    assert_eq!(reader.remaining(), [0_u8; 0]);
    let kind = match (head.get("op"), head.get("t")) {
        (Some(Data::Integer(1)), Some(Data::String(kind))) => kind.clone(),
        (Some(Data::Integer(-1)), _) => "error".to_owned(),
        other => panic!("header {other:?}"),
    };
    Frame { kind, body }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next_frame(socket: &mut Socket) -> Option<Frame> {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("a frame within ten seconds")?;
        match message.unwrap() {
            Message::Binary(bytes) => return Some(decode(&bytes)),
            Message::Close(_) => return None,
            _ => {}
        }
    }
}

impl Server {
    async fn start(posts: usize) -> Self {
        let dir = DataDir::new(posts);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let state = app_state(&dir.config).unwrap();
        let router = pds_xrpc::router(state.clone());
        tokio::spawn(async move { axum::serve(listener, router).await });
        Self {
            origin,
            http: reqwest::Client::new(),
            state,
            dir,
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Answer {
        let response = request.send().await.unwrap();
        Answer {
            status: response.status().as_u16(),
            body: response.bytes().await.unwrap().to_vec(),
        }
    }

    async fn get(&self, path: &str) -> Answer {
        self.send(self.http.get(format!("{}{path}", self.origin)))
            .await
    }

    async fn call(&self, nsid: &str, did: &str, body: &Value) -> Answer {
        self.send(
            self.http
                .post(format!("{}/xrpc/{nsid}", self.origin))
                .header("authorization", token(did))
                .header("content-type", "application/json")
                .body(body.to_string()),
        )
        .await
    }

    async fn create(&self, collection: &str, rkey: &str, record: Value) -> Answer {
        self.call(
            "com.atproto.repo.createRecord",
            ALICE,
            &json!({"repo": ALICE, "collection": collection, "rkey": rkey, "record": record}),
        )
        .await
    }

    async fn subscribe(&self, cursor: Option<i64>) -> Socket {
        let mut url = format!(
            "{}/xrpc/com.atproto.sync.subscribeRepos",
            self.origin.replace("http://", "ws://")
        );
        if let Some(cursor) = cursor {
            url = format!("{url}?cursor={cursor}");
        }
        tokio_tungstenite::connect_async(url).await.unwrap().0
    }

    fn head(&self) -> (String, String) {
        let store = pds_storage::ActorDb::open(&self.dir.config.layout.actor_db(ALICE)).unwrap();
        let root = store.root().unwrap().unwrap();
        (root.cid.to_string(), root.rev)
    }

    /// The type and the revision of the last event of `did` in the log.
    fn last_event(&self, did: &str) -> (String, String) {
        common::open(&self.dir.root.join("sequencer.sqlite"))
            .query_row(
                "select eventType, event from repo_seq where did = ?1 order by seq desc limit 1",
                [did],
                |row| {
                    let event: Vec<u8> = row.get(1)?;
                    Ok((row.get(0)?, pds_sequencer::event_rev(&event).unwrap()))
                },
            )
            .unwrap()
    }

    fn assert_verifies(&self) {
        let report = verify(&self.dir.config).unwrap();
        assert!(report.findings.is_empty(), "{:#?}", report.findings);
    }
}

#[tokio::test]
async fn a_created_record_is_stored_indexed_and_readable() {
    let server = Server::start(3).await;
    let (_, before) = server.head();
    let created = server
        .create(POST, "3aaaaaaaaaab2", post("hello"))
        .await
        .ok();
    assert_eq!(created["uri"], format!("at://{ALICE}/{POST}/3aaaaaaaaaab2"));
    assert_eq!(created["validationStatus"], "valid");
    let (cid, rev) = server.head();
    assert_eq!(created["commit"]["cid"], cid);
    assert_eq!(created["commit"]["rev"], rev);
    assert!(rev > before, "the revision increases");

    let read = server
        .get(&format!(
            "/xrpc/com.atproto.repo.getRecord?repo={ALICE}&collection={POST}&rkey=3aaaaaaaaaab2"
        ))
        .await
        .ok();
    assert_eq!(read["cid"], created["cid"]);
    assert_eq!(read["value"]["text"], "hello");

    // `listRepos` reads the root that the account database holds.
    let repos = server.get("/xrpc/com.atproto.sync.listRepos").await.ok();
    let alice = repos["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|repo| repo["did"] == ALICE)
        .unwrap();
    assert_eq!(alice["head"], cid);
    assert_eq!(alice["rev"], rev);
    server.assert_verifies();
}

#[tokio::test]
async fn a_write_is_refused_as_the_reference_refuses_it() {
    let server = Server::start(1).await;
    let create = |body: Value| {
        let server = &server;
        async move {
            server
                .call("com.atproto.repo.createRecord", ALICE, &body)
                .await
        }
    };
    let (head, _) = server.head();

    // No credential, and another account's repository.
    let anonymous = server
        .send(
            server
                .http
                .post(format!(
                    "{}/xrpc/com.atproto.repo.createRecord",
                    server.origin
                ))
                .header("content-type", "application/json")
                .body(json!({"repo": ALICE, "collection": POST, "record": post("x")}).to_string()),
        )
        .await;
    assert_eq!(anonymous.error(), (401, "AuthMissing".to_owned()));
    let foreign = server
        .call(
            "com.atproto.repo.createRecord",
            ALICE,
            &json!({"repo": "did:plc:nobodynobodynobodynobody", "collection": POST, "record": post("x")}),
        )
        .await;
    assert_eq!(foreign.error(), (403, "AccountNotFound".to_owned()));
    // A deactivated account does not write.
    let deactivated = server
        .call(
            "com.atproto.repo.createRecord",
            DANA,
            &json!({"repo": DANA, "collection": POST, "record": post("x")}),
        )
        .await;
    assert_eq!(deactivated.error(), (401, "AccountDeactivated".to_owned()));

    // The body, in the order the lexicon declares its properties.
    let missing = create(json!({"collection": POST, "record": post("x")})).await;
    assert_eq!(
        missing.json()["message"],
        r#"Missing required key "repo" at $"#
    );
    let bad_key =
        create(json!({"repo": ALICE, "collection": POST, "rkey": "a/b", "record": post("x")}))
            .await;
    assert_eq!(
        bad_key.json()["message"],
        r#"Invalid record key (got "a/b") at $.rkey"#
    );
    let float =
        create(json!({"repo": ALICE, "collection": NOTE, "record": {"$type": NOTE, "ratio": 1.5}}))
            .await;
    assert_eq!(
        float.json()["message"],
        "Expected one of null, boolean, integer, string, cid, bytes, array or object value type \
         (got 1.5) at $.record.ratio"
    );

    // The record.
    let invalid = create(json!({"repo": ALICE, "collection": POST, "rkey": "3aaaaaaaaaab2", "record": {"$type": POST, "createdAt": "2026-01-01T00:00:00.000Z"}})).await;
    assert_eq!(
        invalid.json()["message"],
        r#"Invalid app.bsky.feed.post record: Missing required key "text" at $.record"#
    );
    let not_a_tid =
        create(json!({"repo": ALICE, "collection": POST, "rkey": "nope", "record": post("x")}))
            .await;
    assert_eq!(
        not_a_tid.json()["message"],
        r#"Invalid record key for app.bsky.feed.post: Invalid TID string (got "nope") at $"#
    );
    let wrong_type = create(
        json!({"repo": ALICE, "collection": POST, "record": {"$type": "app.bsky.feed.like"}}),
    )
    .await;
    assert_eq!(
        wrong_type.json()["message"],
        "Invalid $type: expected app.bsky.feed.post, got app.bsky.feed.like"
    );
    let unknown = create(
        json!({"repo": ALICE, "collection": NOTE, "validate": true, "record": {"$type": NOTE}}),
    )
    .await;
    assert_eq!(
        unknown.json()["message"],
        "Unknown lexicon type: com.example.test.note"
    );
    let no_blob = create(json!({"repo": ALICE, "collection": NOTE, "record": {"$type": NOTE, "file": {"$type": "blob", "ref": {"$link": "bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku"}, "mimeType": "text/plain", "size": 11}}})).await;
    assert_eq!(no_blob.error(), (400, "BlobNotFound".to_owned()));

    // Swaps.
    let stale = create(json!({"repo": ALICE, "collection": NOTE, "record": {"$type": NOTE}, "swapCommit": "bafyreigdcnwvpvpvp2u63ysxt4jkdvjmvzqxjvnwonhsxlfyhtqbcxfhuy"})).await;
    assert_eq!(stale.error(), (400, "InvalidSwap".to_owned()));
    assert_eq!(stale.json()["message"], format!("Commit was at {head}"));

    // A key that exists: 400, where the Reference answers 500 (DD-1).
    let clash = create(
        json!({"repo": ALICE, "collection": POST, "rkey": "r0000", "validate": false, "record": post("x")}),
    )
    .await;
    assert_eq!(clash.error(), (400, "InvalidRequest".to_owned()));

    // Nothing was written by any of these.
    assert_eq!(server.head().0, head);
    server.assert_verifies();
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one repository carried through every kind of write
async fn put_delete_and_a_batch_write_what_they_say() {
    let server = Server::start(2).await;
    let put = |rkey: &'static str, record: Value, extra: Value| {
        let server = &server;
        async move {
            let mut body =
                json!({"repo": ALICE, "collection": NOTE, "rkey": rkey, "record": record});
            for (key, value) in extra.as_object().unwrap() {
                body[key] = value.clone();
            }
            server
                .call("com.atproto.repo.putRecord", ALICE, &body)
                .await
        }
    };
    let note = |n: u32| json!({"$type": NOTE, "n": n});

    let created = put("one", note(1), json!({})).await.ok();
    assert_eq!(created["validationStatus"], "unknown");
    let head = server.head();
    // The same content makes no commit.
    let same = put("one", note(1), json!({})).await.ok();
    assert!(same.get("commit").is_none());
    assert_eq!(server.head(), head);

    let updated = put("one", note(2), json!({})).await.ok();
    assert_ne!(updated["cid"], created["cid"]);
    let stale = put("one", note(3), json!({"swapRecord": created["cid"]})).await;
    assert_eq!(stale.error(), (400, "InvalidSwap".to_owned()));
    assert_eq!(
        stale.json()["message"],
        format!("Record was at {}", updated["cid"].as_str().unwrap())
    );
    put("one", note(3), json!({"swapRecord": updated["cid"]}))
        .await
        .ok();
    // `swapRecord: null` says there is no record.
    let exists = put("one", note(4), json!({"swapRecord": null})).await;
    assert_eq!(exists.error(), (400, "InvalidSwap".to_owned()));
    put("two", note(1), json!({"swapRecord": null})).await.ok();

    let batch = server
        .call(
            "com.atproto.repo.applyWrites",
            ALICE,
            &json!({"repo": ALICE, "writes": [
                {"$type": "com.atproto.repo.applyWrites#create", "collection": NOTE, "rkey": "three", "value": note(3)},
                {"$type": "com.atproto.repo.applyWrites#update", "collection": NOTE, "rkey": "two", "value": note(22)},
                {"$type": "com.atproto.repo.applyWrites#delete", "collection": NOTE, "rkey": "one"},
            ]}),
        )
        .await
        .ok();
    let kinds: Vec<&str> = batch["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["$type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "com.atproto.repo.applyWrites#createResult",
            "com.atproto.repo.applyWrites#updateResult",
            "com.atproto.repo.applyWrites#deleteResult"
        ]
    );

    // A batch with one bad operation writes nothing.
    let head = server.head();
    let atomic = server
        .call(
            "com.atproto.repo.applyWrites",
            ALICE,
            &json!({"repo": ALICE, "writes": [
                {"$type": "com.atproto.repo.applyWrites#create", "collection": NOTE, "rkey": "four", "value": note(4)},
                {"$type": "com.atproto.repo.applyWrites#create", "collection": NOTE, "rkey": "three", "value": note(5)},
            ]}),
        )
        .await;
    assert_eq!(atomic.error(), (400, "InvalidRequest".to_owned()));
    assert_eq!(server.head(), head);

    let deleted = server
        .call(
            "com.atproto.repo.deleteRecord",
            ALICE,
            &json!({"repo": ALICE, "collection": NOTE, "rkey": "three"}),
        )
        .await
        .ok();
    assert!(deleted["commit"]["cid"].is_string());
    let again = server
        .call(
            "com.atproto.repo.deleteRecord",
            ALICE,
            &json!({"repo": ALICE, "collection": NOTE, "rkey": "three"}),
        )
        .await
        .ok();
    assert!(again.get("commit").is_none());

    let listed = server
        .get(&format!(
            "/xrpc/com.atproto.repo.listRecords?repo={ALICE}&collection={NOTE}"
        ))
        .await
        .ok();
    let left: Vec<&str> = listed["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["uri"].as_str().unwrap())
        .collect();
    assert_eq!(left, [format!("at://{ALICE}/{NOTE}/two")]);
    server.assert_verifies();
}

#[tokio::test]
async fn an_upload_waits_for_a_record_and_goes_with_its_last_reference() {
    let server = Server::start(1).await;
    let bytes = b"\x89PNG\r\n\x1a\n and the rest of a picture".to_vec();
    let upload = |bytes: Vec<u8>| {
        let server = &server;
        async move {
            server
                .send(
                    server
                        .http
                        .post(format!(
                            "{}/xrpc/com.atproto.repo.uploadBlob",
                            server.origin
                        ))
                        .header("authorization", token(ALICE))
                        .header("content-type", "text/plain")
                        .body(bytes),
                )
                .await
        }
    };
    let blob = upload(bytes.clone()).await.ok()["blob"].clone();
    // The content says what it is, whatever the request declared.
    assert_eq!(blob["mimeType"], "image/png");
    assert_eq!(blob["size"], bytes.len());
    let cid = blob["ref"]["$link"].as_str().unwrap().to_owned();
    let get_blob = format!("/xrpc/com.atproto.sync.getBlob?did={ALICE}&cid={cid}");

    // No record references it yet: it is not served and not listed.
    assert_eq!(server.get(&get_blob).await.status, 400);
    let missing_type = server
        .send(
            server
                .http
                .post(format!(
                    "{}/xrpc/com.atproto.repo.uploadBlob",
                    server.origin
                ))
                .header("authorization", token(ALICE))
                .body(vec![1_u8, 2, 3]),
        )
        .await;
    assert_eq!(
        missing_type.json()["message"],
        "Request encoding (Content-Type) required but not provided"
    );

    // One byte over the limit: the client receives the refusal, because the
    // server reads the body to its end before it answers.
    let too_large = upload(vec![b'x'; 5 * 1024 * 1024 + 1]).await;
    assert_eq!(too_large.error(), (413, "PayloadTooLarge".to_owned()));
    let anonymous = server
        .send(
            server
                .http
                .post(format!(
                    "{}/xrpc/com.atproto.repo.uploadBlob",
                    server.origin
                ))
                .header("content-type", "text/plain")
                .body(vec![b'x'; 3 * 1024 * 1024]),
        )
        .await;
    assert_eq!(anonymous.error(), (401, "AuthMissing".to_owned()));

    // A record that states another size is refused.
    let mut wrong = blob.clone();
    wrong["size"] = json!(1);
    let refused = server
        .create(NOTE, "pic", json!({"$type": NOTE, "file": wrong}))
        .await;
    assert_eq!(refused.error(), (400, "InvalidSize".to_owned()));

    server
        .create(NOTE, "pic", json!({"$type": NOTE, "file": blob}))
        .await
        .ok();
    let answer = server.get(&get_blob).await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body, bytes);
    // The same bytes again: the store keeps the blob it has.
    assert_eq!(upload(bytes.clone()).await.ok()["blob"], blob);
    assert_eq!(server.get(&get_blob).await.status, 200);
    server.assert_verifies();

    // The last reference goes, and the blob with it.
    server
        .call(
            "com.atproto.repo.deleteRecord",
            ALICE,
            &json!({"repo": ALICE, "collection": NOTE, "rkey": "pic"}),
        )
        .await
        .ok();
    assert_eq!(server.get(&get_blob).await.status, 400);
    let staged = server.dir.root.join("blocks").join("tempt").join(ALICE);
    assert_eq!(
        std::fs::read_dir(staged).map_or(0, Iterator::count),
        0,
        "no staged file is left"
    );
    server.assert_verifies();
}

#[tokio::test]
async fn an_upload_that_no_record_references_is_collected_after_an_hour() {
    let server = Server::start(1).await;
    let upload = |bytes: &'static [u8]| {
        let server = &server;
        async move {
            server
                .send(
                    server
                        .http
                        .post(format!(
                            "{}/xrpc/com.atproto.repo.uploadBlob",
                            server.origin
                        ))
                        .header("authorization", token(ALICE))
                        .header("content-type", "text/plain")
                        .body(bytes),
                )
                .await
                .ok()["blob"]
                .clone()
        }
    };
    let old = upload(b"an old upload").await;
    let new = upload(b"a new upload").await;
    let kept = upload(b"an old upload with a record").await;
    server
        .create(NOTE, "kept", json!({"$type": NOTE, "file": kept}))
        .await
        .ok();
    let staged = server.dir.root.join("blocks").join("tempt").join(ALICE);
    assert_eq!(std::fs::read_dir(&staged).unwrap().count(), 2);

    // Two hours pass for two of the three.
    let store = server.dir.actor(ALICE);
    for blob in [&old, &kept] {
        store
            .execute(
                r#"update "blob" set "createdAt" = '2020-01-01T00:00:00.000Z' where "cid" = ?1"#,
                [blob["ref"]["$link"].as_str().unwrap()],
            )
            .unwrap();
    }
    let sweeping = server.state.clone();
    let collected =
        tokio::task::spawn_blocking(move || pds_xrpc::write::collect_uploads(&sweeping))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(collected, 1);
    assert_eq!(std::fs::read_dir(&staged).unwrap().count(), 1);
    let left: Vec<String> = store
        .prepare(r#"select "cid" from "blob" where "tempKey" is not null"#)
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(left, [new["ref"]["$link"].as_str().unwrap()]);
    // The collected upload can no longer be referenced.
    let gone = server
        .create(NOTE, "late", json!({"$type": NOTE, "file": old}))
        .await;
    assert_eq!(gone.error(), (400, "BlobNotFound".to_owned()));
    server.assert_verifies();
}

/// A repository of `did` with one note per name, signed by actor `signer`,
/// as a CAR.
fn repository(did: &str, signer: u8, names: &[&str]) -> Vec<u8> {
    use pds_repo::{MemoryBlockStore, Repo, Write, write_car};
    let records: Vec<(String, Vec<u8>)> = names
        .iter()
        .map(|name| {
            let record = Data::from_json_str(&json!({"$type": NOTE, "name": name}).to_string());
            (format!("{NOTE}/{name}"), record.unwrap().to_dag_cbor())
        })
        .collect();
    let writes: Vec<Write<'_>> = records
        .iter()
        .map(|(key, record)| Write::Create { key, record })
        .collect();
    let key = common::key(signer);
    let commit = Repo::format_init_commit(
        MemoryBlockStore::new(),
        &did.parse().unwrap(),
        "3zzzzzzzzzzz2".parse().unwrap(),
        &writes,
        |bytes| Ok(key.sign(bytes).to_vec()),
    )
    .unwrap();
    write_car(Some(&commit.cid), &commit.new_blocks)
}

#[tokio::test]
async fn an_import_replaces_the_repository_and_checks_the_commit() {
    let server = Server::start(2).await;
    let import = |car: Vec<u8>| {
        let server = &server;
        async move {
            server
                .send(
                    server
                        .http
                        .post(format!(
                            "{}/xrpc/com.atproto.repo.importRepo",
                            server.origin
                        ))
                        .header("authorization", token(ALICE))
                        .header("content-type", "application/vnd.ipld.car")
                        .body(car),
                )
                .await
        }
    };
    let notes = || {
        let server = &server;
        async move {
            let listed = server
                .get(&format!(
                    "/xrpc/com.atproto.repo.listRecords?repo={ALICE}&collection={NOTE}&reverse=true"
                ))
                .await
                .ok();
            listed["records"]
                .as_array()
                .unwrap()
                .iter()
                .map(|record| record["value"]["name"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }
    };

    // The own export changes nothing but the stored revision.
    let export = server
        .get(&format!("/xrpc/com.atproto.sync.getRepo?did={ALICE}"))
        .await;
    let (head, rev) = server.head();
    assert_eq!(import(export.body.clone()).await.status, 200);
    assert_eq!(server.head().0, head);
    assert!(server.head().1 > rev);
    server.assert_verifies();
    // The account is active, so the import states the head (DD-19). The
    // event names the revision of the commit block.
    assert_eq!(server.last_event(ALICE), ("sync".to_owned(), rev.clone()));

    // Another repository of the same account, signed with its key.
    assert_eq!(import(repository(ALICE, 0, &["a", "b"])).await.status, 200);
    assert_eq!(notes().await, ["a", "b"]);
    let posts = server
        .get(&format!(
            "/xrpc/com.atproto.repo.listRecords?repo={ALICE}&collection={POST}"
        ))
        .await
        .ok();
    assert_eq!(posts["records"].as_array().unwrap().len(), 0);
    // A second import states the difference only.
    assert_eq!(import(repository(ALICE, 0, &["b", "c"])).await.status, 200);
    assert_eq!(notes().await, ["b", "c"]);
    server.assert_verifies();
    let head = server.head().0;

    // Refused, and nothing is written: bytes that are not a CAR, a commit
    // that another key signed, and a commit for another account.
    for (car, what) in [
        (b"not a car".to_vec(), "not a CAR"),
        (repository(ALICE, 1, &["x"]), "another key"),
        (repository(DANA, 0, &["x"]), "another account"),
    ] {
        let refused = import(car).await;
        assert_eq!(
            refused.error(),
            (400, "InvalidRequest".to_owned()),
            "{what}"
        );
    }
    assert_eq!(server.head().0, head);
    assert_eq!(notes().await, ["b", "c"]);

    // An app password does not import.
    let wrong_type = server
        .send(
            server
                .http
                .post(format!(
                    "{}/xrpc/com.atproto.repo.importRepo",
                    server.origin
                ))
                .header("authorization", token(ALICE))
                .header("content-type", "application/json")
                .body("{}"),
        )
        .await;
    assert_eq!(
        wrong_type.json()["message"],
        "Wrong request encoding (Content-Type): application/json"
    );
}

#[tokio::test]
async fn a_subscriber_receives_each_commit_once_and_in_order() {
    let server = Server::start(1).await;
    // The seeded rows of the test directory hold no event body, so the
    // subscriber starts after them.
    let seeded = 3;
    let mut live = server.subscribe(None).await;
    let first = server
        .create(NOTE, "a", json!({"$type": NOTE, "n": 1}))
        .await
        .ok();
    let second = server
        .create(NOTE, "b", json!({"$type": NOTE, "n": 2}))
        .await
        .ok();

    let mut frames = Vec::new();
    for _ in 0..2 {
        frames.push(next_frame(&mut live).await.unwrap());
    }
    assert!(frames.iter().all(|frame| frame.kind == "#commit"));
    assert_eq!(frames[0].seq(), seeded + 1);
    assert_eq!(frames[1].seq(), seeded + 2);
    assert_eq!(frames[0].text("rev"), first["commit"]["rev"]);
    assert_eq!(frames[1].text("rev"), second["commit"]["rev"]);
    assert_eq!(frames[1].text("since"), first["commit"]["rev"]);
    assert_eq!(frames[0].text("repo"), ALICE);

    // The blocks carry the signed commit and the record of the operation.
    let Some(Data::Bytes(blocks)) = frames[1].body.get("blocks") else {
        panic!("blocks");
    };
    let car = read_car(&blocks.0).unwrap();
    let root = car.root().unwrap();
    assert_eq!(root.to_string(), second["commit"]["cid"]);
    let commit = Commit::decode(car.blocks.get(&root).unwrap()).unwrap();
    let key = common::key(0).public_key();
    commit
        .verify(|bytes, signature| key.verify(bytes, signature).is_ok())
        .unwrap();
    let Some(Data::Array(ops)) = frames[1].body.get("ops") else {
        panic!("ops");
    };
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].get("path"), Some(&Data::String(format!("{NOTE}/b"))));
    assert_eq!(
        ops[0].get("action"),
        Some(&Data::String("create".to_owned()))
    );
    let Some(Data::Link(record)) = ops[0].get("cid") else {
        panic!("op cid");
    };
    assert_eq!(record.0.to_string(), second["cid"]);
    assert!(car.blocks.contains(&record.0));
    assert!(frames[1].body.get("prevData").is_some());

    // A cursor replays what follows it, and nothing twice.
    let mut replay = server.subscribe(Some(seeded + 1)).await;
    let replayed = next_frame(&mut replay).await.unwrap();
    assert_eq!(replayed.seq(), seeded + 2);
    let third = server
        .create(NOTE, "c", json!({"$type": NOTE, "n": 3}))
        .await
        .ok();
    assert_eq!(
        next_frame(&mut replay).await.unwrap().text("rev"),
        third["commit"]["rev"]
    );
    assert_eq!(
        next_frame(&mut live).await.unwrap().text("rev"),
        third["commit"]["rev"]
    );

    // A cursor above the head, and one that is not a number.
    let mut future = server.subscribe(Some(1_000)).await;
    let refused = next_frame(&mut future).await.unwrap();
    assert_eq!(refused.kind, "error");
    assert_eq!(refused.text("error"), "FutureCursor");
    assert!(next_frame(&mut future).await.is_none(), "the stream closes");

    let url = format!(
        "{}/xrpc/com.atproto.sync.subscribeRepos?cursor=abc",
        server.origin.replace("http://", "ws://")
    );
    let mut invalid = tokio_tungstenite::connect_async(url).await.unwrap().0;
    assert_eq!(
        next_frame(&mut invalid).await.unwrap().text("error"),
        "InvalidRequest"
    );

    // Without an upgrade the method has no route (SPEC §7.2) and goes to
    // the proxy, which has no service to send it to in this directory.
    let plain = server.get("/xrpc/com.atproto.sync.subscribeRepos").await;
    assert_eq!(plain.error(), (400, "InvalidRequest".to_owned()));
}
