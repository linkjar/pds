//! The read-only server over a data directory in the Reference's schema,
//! through a real listener.
//!
//! The parity harness holds the server to the Reference's answers, byte for
//! byte (`parity/src/restore.ts`). These tests hold what can be checked
//! without a Reference: the content of the answers, the statuses of SPEC
//! §10.1, and the credentials that read through them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

mod common;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use common::{
    ADMIN_PASSWORD, ALICE, BLOB_BYTES, BLOB_CID, DANA, DataDir, JWT_SECRET, POST, TARA, key,
};
use hmac::{Hmac, KeyInit, Mac};
use linkjar_pds::serve::app_state;
use pds_repo::{Commit, read_car};
use serde_json::{Value, json};
use sha2::Sha256;

struct Server {
    _listener: common::http::Listener,
    origin: String,
    http: reqwest::Client,
    _dir: DataDir,
}

struct Answer {
    status: u16,
    headers: reqwest::header::HeaderMap,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        common::http::json(&self.body)
    }

    fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .map_or("", |value| value.to_str().unwrap())
    }

    fn error(&self) -> (u16, String) {
        (
            self.status,
            self.json()["error"].as_str().unwrap_or_default().to_owned(),
        )
    }
}

impl Server {
    async fn start(posts: usize) -> Self {
        let dir = DataDir::new(posts);
        let listener = common::http::bind().await;
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = pds_xrpc::router(app_state(&dir.config).unwrap());
        let listener = common::http::serve(listener, router);
        Self {
            _listener: listener,
            origin,
            http: reqwest::Client::new(),
            _dir: dir,
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Answer {
        let response = request.send().await.unwrap();
        Answer {
            status: response.status().as_u16(),
            headers: response.headers().clone(),
            body: response.bytes().await.unwrap().to_vec(),
        }
    }

    async fn get(&self, path: &str) -> Answer {
        self.send(self.http.get(format!("{}{path}", self.origin)))
            .await
    }

    async fn get_owned(&self, path: String) -> Answer {
        self.get(&path).await
    }

    async fn get_as(&self, path: &str, authorization: &str) -> Answer {
        self.send(
            self.http
                .get(format!("{}{path}", self.origin))
                .header("authorization", authorization),
        )
        .await
    }
}

fn admin() -> String {
    format!(
        "Basic {}",
        STANDARD.encode(format!("admin:{ADMIN_PASSWORD}"))
    )
}

/// A legacy access token as the Reference mints it.
fn access_token(did: &str, scope: &str) -> String {
    let head = URL_SAFE_NO_PAD.encode(json!({"alg": "HS256", "typ": "at+jwt"}).to_string());
    let claims = json!({"scope": scope, "sub": did, "aud": "did:web:pds.test", "iat": 1, "exp": 9_999_999_999_u64});
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let mut mac = Hmac::<Sha256>::new_from_slice(JWT_SECRET.as_bytes()).unwrap();
    mac.update(format!("{head}.{body}").as_bytes());
    format!(
        "Bearer {head}.{body}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    )
}

#[tokio::test]
async fn the_tls_check_names_the_server_and_its_active_handles_only() {
    let server = Server::start(1).await;
    let check = |domain: &str| server.get_owned(format!("/tls-check?domain={domain}"));
    assert_eq!(check("pds.test").await.json(), json!({"success": true}));
    assert_eq!(
        check("alice.pds.test").await.json(),
        json!({"did": ALICE, "handle": "alice.pds.test"})
    );
    // A deactivated account, a taken-down account and a free name under the
    // handle domain get no certificate, and neither does another domain.
    for domain in ["dana.pds.test", "tara.pds.test", "nobody.pds.test"] {
        assert_eq!(
            check(domain).await.error(),
            (404, "NotFound".to_owned()),
            "{domain}"
        );
    }
    assert_eq!(
        check("example.com").await.error(),
        (400, "InvalidRequest".to_owned())
    );
    assert_eq!(
        server.get("/tls-check").await.error(),
        (400, "InvalidRequest".to_owned())
    );
}

#[tokio::test]
async fn the_plain_routes_answer() {
    let server = Server::start(1).await;
    let banner = server.get("/").await;
    assert_eq!(banner.status, 200);
    assert!(
        String::from_utf8_lossy(&banner.body)
            .contains("This is an AT Protocol Personal Data Server")
    );
    assert_eq!(
        server.get("/robots.txt").await.header("content-type"),
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        server.get("/xrpc/_health").await.json(),
        json!({"version": "0.0.0-test"})
    );

    let missing = server.get("/nope").await;
    assert_eq!(
        (missing.status, missing.header("content-security-policy")),
        (404, "default-src 'none'")
    );
    assert!(String::from_utf8_lossy(&missing.body).contains("<pre>Cannot GET /nope</pre>"));

    let well_known = |host: &'static str| {
        server.send(
            server
                .http
                .get(format!("{}/.well-known/atproto-did", server.origin))
                .header("host", host),
        )
    };
    assert_eq!(well_known("alice.pds.test").await.body, ALICE.as_bytes());
    assert_eq!(well_known("dana.pds.test").await.status, 404);
    assert_eq!(well_known("nobody.pds.test").await.status, 404);
    assert_eq!(well_known("alice.example.com").await.status, 404);
}

#[tokio::test]
async fn every_answer_allows_any_origin_and_a_preflight_is_answered() {
    let server = Server::start(1).await;
    for path in [
        "/",
        "/nope",
        "/xrpc/com.atproto.sync.listRepos",
        "/xrpc/com.atproto.nope",
    ] {
        assert_eq!(
            server.get(path).await.header("access-control-allow-origin"),
            "*",
            "{path}"
        );
    }
    let preflight = server
        .send(
            server
                .http
                .request(
                    reqwest::Method::OPTIONS,
                    format!("{}/xrpc/com.atproto.sync.listRepos", server.origin),
                )
                .header("access-control-request-headers", "authorization, x-test"),
        )
        .await;
    assert_eq!(preflight.status, 204);
    assert_eq!(
        preflight.header("access-control-allow-methods"),
        "GET,HEAD,PUT,PATCH,POST,DELETE"
    );
    assert_eq!(
        preflight.header("access-control-allow-headers"),
        "authorization, x-test"
    );
    assert_eq!(preflight.header("access-control-max-age"), "86400");
}

#[tokio::test]
async fn the_export_is_the_whole_repository_and_its_commit_verifies() {
    // 700 records give more blocks than one page of the block table.
    let server = Server::start(700).await;
    let export = server
        .get(&format!("/xrpc/com.atproto.sync.getRepo?did={ALICE}"))
        .await;
    assert_eq!(
        (export.status, export.header("content-type")),
        (200, "application/vnd.ipld.car")
    );
    let car = read_car(&export.body).unwrap();
    let latest = server
        .get(&format!(
            "/xrpc/com.atproto.sync.getLatestCommit?did={ALICE}"
        ))
        .await
        .json();
    let root = car.root().unwrap();
    assert_eq!(root.to_string(), latest["cid"]);
    assert!(car.blocks.len() > 700);

    let commit = Commit::decode(car.blocks.get(&root).unwrap()).unwrap();
    let public = key(0).public_key();
    commit
        .verify(|message, signature| public.verify(message, signature).is_ok())
        .unwrap();
    assert_eq!(
        (commit.did.as_str(), commit.rev.to_string()),
        (ALICE, latest["rev"].as_str().unwrap().to_owned())
    );

    // Nothing is newer than the latest revision, and the first revision
    // is older than most of the repository.
    let since = server
        .get(&format!(
            "/xrpc/com.atproto.sync.getRepo?did={ALICE}&since={}",
            commit.rev
        ))
        .await;
    assert_eq!(read_car(&since.body).unwrap().blocks.len(), 0);
    let later = server
        .get(&format!(
            "/xrpc/com.atproto.sync.getRepo?did={ALICE}&since=3aaaaaaaaaaa2"
        ))
        .await;
    let later = read_car(&later.body).unwrap();
    assert_eq!(later.root().unwrap(), root);
    assert!(later.blocks.len() > 500 && later.blocks.len() < car.blocks.len());
    let checkout = server
        .get(&format!("/xrpc/com.atproto.sync.getCheckout?did={ALICE}"))
        .await;
    assert_eq!(checkout.body, export.body);
    let head = server
        .get(&format!("/xrpc/com.atproto.sync.getHead?did={ALICE}"))
        .await
        .json();
    assert_eq!(head, json!({"root": root.to_string()}));
}

#[tokio::test]
async fn a_record_comes_with_its_proof() {
    let server = Server::start(300).await;
    let record = server
        .get(&format!(
            "/xrpc/com.atproto.repo.getRecord?repo={ALICE}&collection={POST}&rkey=r0123"
        ))
        .await
        .json();
    assert_eq!(
        record["value"],
        json!({"text": "post 123", "$type": POST, "createdAt": "2026-01-01T00:00:00.000Z"})
    );
    assert_eq!(record["uri"], format!("at://{ALICE}/{POST}/r0123"));

    let proof = server
        .get(&format!(
            "/xrpc/com.atproto.sync.getRecord?did={ALICE}&collection={POST}&rkey=r0123"
        ))
        .await;
    let car = read_car(&proof.body).unwrap();
    let cids: Vec<String> = car.blocks.cids().map(ToString::to_string).collect();
    // The commit first, then the record, then the tree nodes from the root down.
    assert_eq!(cids[0], car.root().unwrap().to_string());
    assert_eq!(cids[1], record["cid"]);
    assert!(cids.len() >= 3);

    // A proof of absence has the commit and the path, and no record.
    let absent = server
        .get(&format!(
            "/xrpc/com.atproto.sync.getRecord?did={ALICE}&collection={POST}&rkey=r9999"
        ))
        .await;
    let absent = read_car(&absent.body).unwrap();
    assert!(absent.blocks.len() >= 2);
    assert!(
        !absent
            .blocks
            .cids()
            .any(|cid| cid.to_string() == record["cid"])
    );

    let blocks = server
        .get(&format!(
            "/xrpc/com.atproto.sync.getBlocks?did={ALICE}&cids={}",
            record["cid"].as_str().unwrap()
        ))
        .await;
    let blocks = read_car(&blocks.body).unwrap();
    assert_eq!(blocks.roots, []);
    assert_eq!(blocks.blocks.len(), 1);
    let lacking = server
        .get(&format!(
            "/xrpc/com.atproto.sync.getBlocks?did={ALICE}&cids={BLOB_CID}"
        ))
        .await;
    assert_eq!(
        lacking.json()["message"],
        format!("Could not find cids: {BLOB_CID}")
    );
}

#[tokio::test]
async fn records_are_listed_newest_first_and_paged() {
    let server = Server::start(5).await;
    let list = |query: &'static str| {
        let server = &server;
        async move {
            let answer = server
                .get(&format!("/xrpc/com.atproto.repo.listRecords?repo=alice.pds.test&collection={POST}{query}"))
                .await
                .json();
            let keys: Vec<String> = answer["records"]
                .as_array()
                .unwrap()
                .iter()
                .map(|record| {
                    record["uri"]
                        .as_str()
                        .unwrap()
                        .rsplit('/')
                        .next()
                        .unwrap()
                        .to_owned()
                })
                .collect();
            (keys, answer["cursor"].as_str().map(str::to_owned))
        }
    };
    assert_eq!(
        list("").await.0,
        ["r0004", "r0003", "r0002", "r0001", "r0000"]
    );
    assert_eq!(
        list("&limit=2").await,
        (
            vec!["r0004".to_owned(), "r0003".to_owned()],
            Some("r0003".to_owned())
        )
    );
    assert_eq!(list("&limit=2&cursor=r0003").await.0, ["r0002", "r0001"]);
    assert_eq!(list("&reverse=true&limit=2").await.0, ["r0000", "r0001"]);
    assert_eq!(list("&cursor=r0000").await, (vec![], None));
}

#[tokio::test]
async fn large_record_responses_preserve_json_bytes_and_content_type() {
    let server = Server::start(1).await;
    let collection = "com.example.large";
    let payload = "x".repeat(48 * 1024);
    let record = json!({
        "$type": collection,
        "payload": payload,
        "nested": {"z": [true, null, 42, {"$bytes": "AQID"}, {"$link": BLOB_CID}], "a": "line\n雪"}
    });
    let written = server.send(
        server.http.post(format!("{}/xrpc/com.atproto.repo.createRecord", server.origin))
            .header("authorization", access_token(ALICE, "com.atproto.access"))
            .header("content-type", "application/json")
            .body(json!({"repo": ALICE, "collection": collection, "rkey": "large", "validate": false, "record": record}).to_string())
    ).await;
    assert_eq!(written.status, 200);
    let cid = written.json()["cid"].as_str().unwrap().to_owned();
    let value = format!(
        r#"{{"$type":"{collection}","nested":{{"a":"line\n雪","z":[true,null,42,{{"$bytes":"AQID"}},{{"$link":"{BLOB_CID}"}}]}},"payload":"{payload}"}}"#
    );
    let expected =
        format!(r#"{{"uri":"at://{ALICE}/{collection}/large","cid":"{cid}","value":{value}}}"#);
    let get = server
        .get(&format!(
            "/xrpc/com.atproto.repo.getRecord?repo={ALICE}&collection={collection}&rkey=large"
        ))
        .await;
    assert_eq!(get.status, 200);
    assert_eq!(
        get.header("content-type"),
        "application/json; charset=utf-8"
    );
    assert_eq!(get.body, expected.as_bytes());
    let list = server
        .get(&format!(
            "/xrpc/com.atproto.repo.listRecords?repo={ALICE}&collection={collection}&limit=1"
        ))
        .await;
    assert_eq!(list.status, 200);
    assert_eq!(
        list.header("content-type"),
        "application/json; charset=utf-8"
    );
    assert_eq!(
        list.body,
        format!(r#"{{"records":[{expected}],"cursor":"large"}}"#).as_bytes()
    );
}

#[tokio::test]
async fn describe_repo_reads_the_cached_document() {
    let server = Server::start(2).await;
    let described = server
        .get("/xrpc/com.atproto.repo.describeRepo?repo=alice.pds.test")
        .await
        .json();
    assert_eq!(described["did"], ALICE);
    assert_eq!(described["handle"], "alice.pds.test");
    assert_eq!(described["handleIsCorrect"], true);
    assert_eq!(described["collections"], json!([POST]));
    assert_eq!(described["didDoc"]["id"], ALICE);

    let server_info = server
        .get("/xrpc/com.atproto.server.describeServer")
        .await
        .json();
    assert_eq!(server_info["did"], "did:web:pds.test");
    assert_eq!(server_info["availableUserDomains"], json!([".pds.test"]));
    assert_eq!(server_info["inviteCodeRequired"], false);
    assert_eq!(server_info["blobUploadLimit"], 5 * 1024 * 1024);

    let resolved = server
        .get("/xrpc/com.atproto.identity.resolveHandle?handle=ALICE.pds.test")
        .await
        .json();
    assert_eq!(resolved, json!({"did": ALICE}));
    let nobody = server
        .get("/xrpc/com.atproto.identity.resolveHandle?handle=nobody.pds.test")
        .await;
    assert_eq!(nobody.error(), (400, "HandleNotFound".to_owned()));
    // No AppView is configured here, and the server does not resolve a
    // foreign handle itself yet.
    let foreign = server
        .get("/xrpc/com.atproto.identity.resolveHandle?handle=someone.example.com")
        .await;
    assert_eq!(foreign.error(), (400, "InvalidRequest".to_owned()));
}

#[tokio::test]
async fn a_blob_is_served_as_a_download_that_a_browser_will_not_run() {
    let server = Server::start(1).await;
    let blob = server
        .get(&format!(
            "/xrpc/com.atproto.sync.getBlob?did={ALICE}&cid={BLOB_CID}"
        ))
        .await;
    assert_eq!((blob.status, blob.body.as_slice()), (200, BLOB_BYTES));
    assert_eq!(blob.header("content-type"), "text/plain; charset=utf-8");
    assert_eq!(blob.header("content-length"), BLOB_BYTES.len().to_string());
    assert_eq!(blob.header("x-content-type-options"), "nosniff");
    assert_eq!(
        blob.header("content-security-policy"),
        "default-src 'none'; sandbox"
    );
    assert_eq!(
        blob.header("content-disposition"),
        format!("attachment; filename=\"{BLOB_CID}\"")
    );

    let listed = server
        .get(&format!("/xrpc/com.atproto.sync.listBlobs?did={ALICE}"))
        .await
        .json();
    assert_eq!(listed, json!({"cursor": BLOB_CID, "cids": [BLOB_CID]}));
    let elsewhere = server
        .get(&format!(
            "/xrpc/com.atproto.sync.getBlob?did={DANA}&cid={BLOB_CID}"
        ))
        .await;
    assert_eq!(elsewhere.status, 400);
}

#[tokio::test]
async fn an_account_that_is_not_active_is_read_by_its_owner_and_by_an_admin() {
    let server = Server::start(1).await;
    for (did, error, status) in [
        (DANA, "RepoDeactivated", "deactivated"),
        (TARA, "RepoTakendown", "takendown"),
    ] {
        for method in [
            "getRepo",
            "getLatestCommit",
            "getHead",
            "getCheckout",
            "listBlobs",
        ] {
            let path = format!("/xrpc/com.atproto.sync.{method}?did={did}");
            assert_eq!(
                server.get(&path).await.error(),
                (400, error.to_owned()),
                "{path}"
            );
            assert_eq!(
                server
                    .get_as(&path, &access_token(ALICE, "com.atproto.access"))
                    .await
                    .error()
                    .1,
                error,
                "{path}"
            );
            assert_eq!(
                server
                    .get_as(&path, &access_token(did, "com.atproto.access"))
                    .await
                    .status,
                200,
                "{path}"
            );
            assert_eq!(server.get_as(&path, &admin()).await.status, 200, "{path}");
        }
        // The status itself is public.
        let state = server
            .get(&format!("/xrpc/com.atproto.sync.getRepoStatus?did={did}"))
            .await
            .json();
        assert_eq!(
            state,
            json!({"did": did, "active": false, "status": status})
        );
        // The record methods find no such repository.
        let record = server
            .get(&format!(
                "/xrpc/com.atproto.repo.listRecords?repo={did}&collection={POST}"
            ))
            .await;
        assert_eq!(
            record.json()["message"],
            format!("Could not find repo: {did}")
        );
    }
    let listed = server.get("/xrpc/com.atproto.sync.listRepos").await.json();
    let states: Vec<(&str, bool, Option<&str>)> = listed["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repo| {
            (
                repo["did"].as_str().unwrap(),
                repo["active"].as_bool().unwrap(),
                repo["status"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        states,
        [
            (ALICE, true, None),
            (DANA, false, Some("deactivated")),
            (TARA, false, Some("takendown"))
        ]
    );
    // The scope of a taken-down account reads its own export and nothing else.
    let limited = access_token(TARA, "com.atproto.takendown");
    assert_eq!(
        server
            .get_as(
                &format!("/xrpc/com.atproto.sync.getRepo?did={TARA}"),
                &limited
            )
            .await
            .status,
        200
    );
    assert_eq!(
        server
            .get_as(
                &format!("/xrpc/com.atproto.sync.getLatestCommit?did={TARA}"),
                &limited
            )
            .await
            .error(),
        (400, "InvalidToken".to_owned())
    );
}

#[tokio::test]
async fn a_method_without_a_handler_needs_a_service_to_go_to() {
    let server = Server::start(1).await;
    let path = "/xrpc/app.bsky.actor.getProfile?actor=alice.pds.test";
    // Authenticate before resolving the destination. An authenticated
    // request still reports that this directory names no AppView.
    assert_eq!(
        server.get(path).await.error(),
        (401, "AuthMissing".to_owned())
    );
    assert_eq!(
        server.get_as(path, &admin()).await.error(),
        (400, "InvalidToken".to_owned())
    );
    let answer = server
        .get_as(path, &access_token(ALICE, "com.atproto.access"))
        .await;
    assert_eq!(answer.error(), (400, "InvalidRequest".to_owned()));
    assert_eq!(
        answer.json()["message"],
        "No service configured for app.bsky.actor.getProfile"
    );
    assert_eq!(
        server.get("/xrpc/a/b").await.json()["message"],
        "invalid xrpc path"
    );
    let post = server
        .send(
            server
                .http
                .post(format!("{}/xrpc/com.atproto.sync.listRepos", server.origin)),
        )
        .await;
    assert_eq!(
        post.json()["message"],
        "Incorrect HTTP method (POST) expected GET"
    );
}
