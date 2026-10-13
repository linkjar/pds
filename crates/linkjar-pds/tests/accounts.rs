//! Account creation over HTTP (`docs/SPEC.md` §10.1).
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

mod common;

use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use common::{DataDir, PASSWORD};
use linkjar_pds::serve::app_state;
use serde_json::{Value, json};

type Ops = Arc<Mutex<Vec<(String, Value)>>>;

/// A PLC directory that keeps what it is sent and resolves nothing.
async fn plc() -> (String, Ops) {
    let ops = Ops::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route(
            "/{did}",
            axum::routing::post(
                |State(ops): State<Ops>, Path(did): Path<String>, body: String| async move {
                    let op = serde_json::from_str(&body).unwrap();
                    ops.lock().unwrap().push((did, op));
                    StatusCode::OK
                },
            )
            .get(|| async { StatusCode::NOT_FOUND }),
        )
        .with_state(Arc::clone(&ops));
    tokio::spawn(async move { axum::serve(listener, app).await });
    (origin, ops)
}

struct Server {
    origin: String,
    http: reqwest::Client,
    dir: DataDir,
    ops: Ops,
}

impl Server {
    async fn start(extra: &[(&str, &str)]) -> Self {
        let (plc, ops) = plc().await;
        let mut vars = vec![
            ("PDS_DID_PLC_URL", plc.as_str()),
            ("PDS_SERVICE_HANDLE_DOMAINS", ".pds-users.org"),
            (
                "PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX",
                "0707070707070707070707070707070707070707070707070707070707070707",
            ),
        ];
        vars.extend_from_slice(extra);
        let dir = DataDir::new_with(1, &vars);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = pds_xrpc::router(app_state(&dir.config).unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await });
        Self {
            origin,
            http: reqwest::Client::new(),
            dir,
            ops,
        }
    }

    async fn post(&self, nsid: &str, token: Option<&str>, body: &Value) -> (u16, Value) {
        let mut request = self
            .http
            .post(format!("{}/xrpc/{nsid}", self.origin))
            .header("content-type", "application/json")
            .body(body.to_string());
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let bytes = response.bytes().await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn create(&self, body: &Value) -> (u16, Value) {
        self.post("com.atproto.server.createAccount", None, body)
            .await
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

#[tokio::test]
async fn an_account_is_created_with_its_identity_repository_session_and_events() {
    let server = Server::start(&[]).await;
    let (status, made) = server
        .create(&json!({"handle": "Bob.pds-users.org", "email": "Bob@Example.com", "password": PASSWORD}))
        .await;
    assert_eq!(status, 200, "{made}");
    assert_eq!(
        made.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["handle", "did", "accessJwt", "refreshJwt"]
    );
    assert_eq!(made["handle"], "bob.pds-users.org");
    let did = text(&made, "did").to_owned();
    assert!(did.starts_with("did:plc:") && did.len() == 32);

    // The directory got one genesis operation for that DID.
    let ops = server.ops.lock().unwrap().clone();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].0, did);
    let op = &ops[0].1;
    assert_eq!(op["alsoKnownAs"], json!(["at://bob.pds-users.org"]));
    assert_eq!(
        op["services"]["atproto_pds"]["endpoint"],
        "https://pds.test"
    );
    assert_eq!(op["rotationKeys"].as_array().unwrap().len(), 1);

    // The data directory verifies, and the log holds the four events.
    let report = linkjar_pds::verify::verify(&server.dir.config).unwrap();
    assert_eq!(report.findings.len(), 0, "{report}");
    let events: Vec<String> = common::open(&server.dir.root.join("sequencer.sqlite"))
        .prepare(r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq""#)
        .unwrap()
        .query_map([&did], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(events, ["identity", "account", "append", "sync"]);

    // The session works, the password opens another, and the repository
    // takes a write.
    let access = text(&made, "accessJwt");
    let (status, session) = server
        .post(
            "com.atproto.server.createSession",
            None,
            &json!({"identifier": "bob@example.com", "password": PASSWORD}),
        )
        .await;
    assert_eq!(status, 200, "{session}");
    assert_eq!(session["did"], did.as_str());
    assert_eq!(session["emailConfirmed"], false);
    let (status, refreshed) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&made, "refreshJwt")),
            &Value::Null,
        )
        .await;
    assert_eq!(status, 200, "{refreshed}");
    let record = json!({"$type": "com.example.test.note", "name": "first"});
    let (status, written) = server
        .post(
            "com.atproto.repo.createRecord",
            Some(access),
            &json!({"repo": did, "collection": "com.example.test.note", "record": record}),
        )
        .await;
    assert_eq!(status, 200, "{written}");
    let report = linkjar_pds::verify::verify(&server.dir.config).unwrap();
    assert_eq!(report.findings.len(), 0, "{report}");
}

#[tokio::test]
async fn a_creation_is_refused_as_the_reference_refuses_it() {
    let server = Server::start(&[]).await;
    let base = json!({"handle": "carol.pds-users.org", "email": "carol@example.com", "password": PASSWORD});
    let with = |key: &str, value: Value| {
        let mut body = base.clone();
        body[key] = value;
        body
    };
    for (body, error, message) in [
        (
            with("plcOp", json!({})),
            "InvalidRequest",
            "Unsupported input: \"plcOp\"",
        ),
        (
            with("password", json!("x".repeat(257))),
            "InvalidRequest",
            "Password too long. Maximum length is 256 characters.",
        ),
        (
            json!({"handle": "carol.pds-users.org"}),
            "InvalidRequest",
            "Email is required",
        ),
        (
            with("email", json!("not an address")),
            "InvalidRequest",
            "This email address is not supported, please use a different email.",
        ),
        (
            with("handle", json!("carol.elsewhere.org")),
            "UnsupportedDomain",
            "Not a supported handle domain",
        ),
        (
            with("handle", json!("a.b.pds-users.org")),
            "InvalidHandle",
            "Invalid characters in handle",
        ),
        (
            with("handle", json!("ab.pds-users.org")),
            "InvalidHandle",
            "Handle too short",
        ),
        (
            with("handle", json!("abcdefghijklmnopqrs.pds-users.org")),
            "InvalidHandle",
            "Handle too long",
        ),
        (
            with("handle", json!("admin.pds-users.org")),
            "HandleNotAvailable",
            "Reserved handle",
        ),
        (
            with("handle", json!("carol.pds-users.invalid")),
            "InvalidHandle",
            "Handle TLD is invalid or disallowed",
        ),
        (
            with("email", json!("Alice@example.com")),
            "InvalidRequest",
            "Email already taken: Alice@example.com",
        ),
    ] {
        let (status, answer) = server.create(&body).await;
        assert_eq!(
            (status, text(&answer, "error"), text(&answer, "message")),
            (400, error, message),
            "{body}"
        );
    }
    let (status, answer) = server
        .create(&with("did", json!("did:plc:bbbbbbbbbbbbbbbbbbbbbbbb")))
        .await;
    assert_eq!(
        (status, text(&answer, "error")),
        (401, "AuthenticationRequired")
    );
    // Nothing was sent to the directory and nothing was stored.
    assert_eq!(server.ops.lock().unwrap().len(), 0);

    // A handle is taken once, whatever the case.
    assert_eq!(server.create(&base).await.0, 200);
    let (status, answer) = server
        .create(&with("email", json!("other@example.com")))
        .await;
    assert_eq!(
        (status, text(&answer, "message")),
        (400, "Handle already taken: carol.pds-users.org")
    );
}

#[tokio::test]
async fn an_invite_code_is_required_and_used_once() {
    let server = Server::start(&[("PDS_INVITE_REQUIRED", "true")]).await;
    common::open(&server.dir.root.join("account.sqlite"))
        .execute(
            r#"insert into "invite_code" values ('pds-users-org-abcde', 1, 0, 'admin', 'admin', '2026-01-01T00:00:00.000Z')"#,
            [],
        )
        .unwrap();
    let body = |name: &str, code: Option<&str>| {
        let mut body = json!({"handle": format!("{name}.pds-users.org"),
                              "email": format!("{name}@example.com"), "password": PASSWORD});
        if let Some(code) = code {
            body["inviteCode"] = json!(code);
        }
        body
    };
    let (status, answer) = server.create(&body("dave", None)).await;
    assert_eq!(
        (status, text(&answer, "error"), text(&answer, "message")),
        (400, "InvalidInviteCode", "No invite code provided")
    );
    let (status, answer) = server.create(&body("dave", Some("unknown"))).await;
    assert_eq!(
        (status, text(&answer, "error"), text(&answer, "message")),
        (
            400,
            "InvalidInviteCode",
            "Provided invite code not available"
        )
    );
    assert_eq!(
        server
            .create(&body("dave", Some("pds-users-org-abcde")))
            .await
            .0,
        200
    );
    let (status, answer) = server
        .create(&body("erin", Some("pds-users-org-abcde")))
        .await;
    assert_eq!((status, text(&answer, "error")), (400, "InvalidInviteCode"));
    assert_eq!(server.ops.lock().unwrap().len(), 1);
}
