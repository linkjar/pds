//! Account creation over HTTP (`docs/SPEC.md` §10.1).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    // A test here walks one account through a whole procedure.
    clippy::too_many_lines,
    missing_docs
)]

mod common;

use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
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
        .route(
            "/{did}/log/last",
            axum::routing::get(
                |State(ops): State<Ops>, Path(did): Path<String>| async move {
                    let ops = ops.lock().unwrap();
                    match ops.iter().rev().find(|(sent, _)| *sent == did) {
                        Some((_, op)) => op.to_string().into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                },
            ),
        )
        .route(
            "/{did}/data",
            axum::routing::get(
                |State(ops): State<Ops>, Path(did): Path<String>| async move {
                    let ops = ops.lock().unwrap();
                    match ops.iter().rev().find(|(sent, _)| *sent == did) {
                        Some((_, op)) => op.to_string().into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                },
            ),
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

/// A server on an empty directory creates the databases of the Reference,
/// with its migrations recorded, and takes an account.
#[tokio::test]
async fn a_new_data_directory_gets_the_databases_of_the_reference() {
    let (plc, _ops) = plc().await;
    let root = std::env::temp_dir().join(format!("linkjar-pds-fresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("blocks")).unwrap();
    let vars = [
        ("PDS_HOSTNAME", "pds.test"),
        ("PDS_DATA_DIRECTORY", root.to_str().unwrap()),
        (
            "PDS_BLOBSTORE_DISK_LOCATION",
            root.join("blocks").to_str().unwrap(),
        ),
        ("PDS_JWT_SECRET", common::JWT_SECRET),
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
        ("PDS_INVITE_REQUIRED", "false"),
        ("PDS_DID_PLC_URL", plc.as_str()),
        ("PDS_SERVICE_HANDLE_DOMAINS", ".pds-users.org"),
        (
            "PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX",
            "0707070707070707070707070707070707070707070707070707070707070707",
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let config = linkjar_pds::config::Config::from_vars(&vars).unwrap();
    linkjar_pds::serve::create_missing_databases(&config).unwrap();
    // A second start changes nothing.
    linkjar_pds::serve::create_missing_databases(&config).unwrap();

    for (file, tables, migrations) in [
        ("account.sqlite", 18, 7),
        ("sequencer.sqlite", 3, 1),
        ("did_cache.sqlite", 3, 1),
    ] {
        let db = common::open(&root.join(file));
        let count = |sql: &str| -> i64 { db.query_row(sql, [], |row| row.get(0)).unwrap() };
        assert_eq!(
            count(
                "select count(*) from sqlite_master where type = 'table' and name not like 'sqlite_%'"
            ),
            tables,
            "{file}"
        );
        assert_eq!(
            count("select count(*) from kysely_migration"),
            migrations,
            "{file}"
        );
        assert_eq!(
            count("select is_locked from kysely_migration_lock"),
            0,
            "{file}"
        );
        let mode: String = db
            .query_row("pragma journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal", "{file}");
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = pds_xrpc::router(app_state(&config).unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await });
    let response = reqwest::Client::new()
        .post(format!("{origin}/xrpc/com.atproto.server.createAccount"))
        .header("content-type", "application/json")
        .body(json!({"handle": "zoe-one.pds-users.org", "email": "zoe@example.com", "password": PASSWORD}).to_string())
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let made: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(status, 200, "{made}");
    let report = linkjar_pds::verify::verify(&config).unwrap();
    assert_eq!((report.actors, report.findings.len()), (1, 0), "{report}");
    // The actor store records its migration too.
    let store = common::open(&config.layout.actor_db(text(&made, "did")));
    let recorded: String = store
        .query_row("select name from kysely_migration", [], |row| row.get(0))
        .unwrap();
    assert_eq!(recorded, "001");
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn an_account_is_deactivated_checked_and_activated() {
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "frank.pds-users.org", "email": "frank@example.com", "password": PASSWORD}))
        .await;
    let (did, access) = (
        text(&made, "did").to_owned(),
        text(&made, "accessJwt").to_owned(),
    );
    let get = |nsid: &'static str| {
        let (http, origin, access) = (server.http.clone(), server.origin.clone(), access.clone());
        async move {
            let response = http
                .get(format!("{origin}/xrpc/{nsid}"))
                .bearer_auth(access)
                .send()
                .await
                .unwrap();
            let status = response.status().as_u16();
            let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            (status, body)
        }
    };
    let events = || -> Vec<String> {
        common::open(&server.dir.root.join("sequencer.sqlite"))
            .prepare(r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq""#)
            .unwrap()
            .query_map([&did], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };

    let (status, checked) = get("com.atproto.server.checkAccountStatus").await;
    assert_eq!(status, 200, "{checked}");
    assert_eq!(
        (
            &checked["activated"],
            &checked["validDid"],
            &checked["repoBlocks"],
            &checked["indexedRecords"]
        ),
        // The commit and the empty root of the tree.
        (&json!(true), &json!(true), &json!(2), &json!(0))
    );
    assert_eq!(checked.as_object().unwrap().len(), 9);

    let deactivate = "com.atproto.server.deactivateAccount";
    assert_eq!(
        server.post(deactivate, Some(&access), &json!({})).await.0,
        200
    );
    let (_, session) = get("com.atproto.server.getSession").await;
    assert_eq!(
        (&session["active"], &session["status"]),
        (&json!(false), &json!("deactivated"))
    );
    let (_, checked) = get("com.atproto.server.checkAccountStatus").await;
    assert_eq!(checked["activated"], false);
    // A deactivated repository takes no write.
    let (status, refused) = server
        .post(
            "com.atproto.repo.createRecord",
            Some(&access),
            &json!({"repo": did, "collection": "com.example.test.note", "record": {"$type": "com.example.test.note"}}),
        )
        .await;
    assert_eq!(
        (status, text(&refused, "error")),
        (401, "AccountDeactivated")
    );

    let (status, body) = server
        .post(
            "com.atproto.server.activateAccount",
            Some(&access),
            &Value::Null,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, session) = get("com.atproto.server.getSession").await;
    assert_eq!(session["active"], true);
    assert_eq!(
        events(),
        [
            "identity", "account", "append", "sync", "account", "account", "identity", "sync"
        ]
    );
    let report = linkjar_pds::verify::verify(&server.dir.config).unwrap();
    assert_eq!(report.findings.len(), 0, "{report}");
}

#[tokio::test]
async fn a_handle_is_changed_in_the_directory_the_account_and_the_log() {
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "gina.pds-users.org", "email": "gina@example.com", "password": PASSWORD}))
        .await;
    let (did, access) = (
        text(&made, "did").to_owned(),
        text(&made, "accessJwt").to_owned(),
    );
    let update = "com.atproto.identity.updateHandle";

    let (status, body) = server
        .post(
            update,
            Some(&access),
            &json!({"handle": "Georgina.pds-users.org"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let ops = server.ops.lock().unwrap().clone();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[1].0, did);
    assert_eq!(
        ops[1].1["alsoKnownAs"],
        json!(["at://georgina.pds-users.org"])
    );
    assert!(ops[1].1["prev"].as_str().unwrap().starts_with("bafyrei"));
    let (status, session) = server
        .post(
            "com.atproto.server.createSession",
            None,
            &json!({"identifier": "georgina.pds-users.org", "password": PASSWORD}),
        )
        .await;
    assert_eq!((status, &session["did"]), (200, &json!(did)));
    let last: String = common::open(&server.dir.root.join("sequencer.sqlite"))
        .query_row(
            r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq" desc limit 1"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(last, "identity");

    for (handle, error, message) in [
        (
            "carol.pds.example",
            "InvalidHandle",
            "Handle TLD is invalid or disallowed",
        ),
        ("ab.pds-users.org", "InvalidHandle", "Handle too short"),
        (
            "admin.pds-users.org",
            "HandleNotAvailable",
            "Reserved handle",
        ),
        (
            "unreachable.elsewhere.org",
            "InvalidRequest",
            "External handle did not resolve to DID",
        ),
    ] {
        let (status, body) = server
            .post(update, Some(&access), &json!({"handle": handle}))
            .await;
        assert_eq!(
            (status, text(&body, "error"), text(&body, "message")),
            (400, error, message),
            "{handle}"
        );
    }
    // Another account's handle is not available.
    server
        .create(&json!({"handle": "henry.pds-users.org", "email": "henry@example.com", "password": PASSWORD}))
        .await;
    let (status, body) = server
        .post(
            update,
            Some(&access),
            &json!({"handle": "henry.pds-users.org"}),
        )
        .await;
    assert_eq!(
        (status, text(&body, "error"), text(&body, "message")),
        (
            400,
            "HandleNotAvailable",
            "Handle already taken: henry.pds-users.org"
        )
    );
    assert_eq!(server.ops.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn the_admin_reads_and_changes_the_status_of_accounts_records_and_blobs() {
    use base64::Engine as _;
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "irene.pds-users.org", "email": "irene@example.com", "password": PASSWORD}))
        .await;
    let (did, access) = (
        text(&made, "did").to_owned(),
        text(&made, "accessJwt").to_owned(),
    );
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("admin:{}", common::ADMIN_PASSWORD))
    );
    let admin_get = |path: String| {
        let (http, origin, basic) = (server.http.clone(), server.origin.clone(), basic.clone());
        async move {
            let response = http
                .get(format!("{origin}/xrpc/{path}"))
                .header("authorization", basic)
                .send()
                .await
                .unwrap();
            let status = response.status().as_u16();
            let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            (status, body)
        }
    };
    let admin_post = |nsid: &'static str, body: Value| {
        let (http, origin, basic) = (server.http.clone(), server.origin.clone(), basic.clone());
        async move {
            let response = http
                .post(format!("{origin}/xrpc/{nsid}"))
                .header("authorization", basic)
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .await
                .unwrap();
            let status = response.status().as_u16();
            let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            (status, body)
        }
    };
    let info = format!("com.atproto.admin.getAccountInfo?did={did}");
    let subject = format!("com.atproto.admin.getSubjectStatus?did={did}");
    let update = "com.atproto.admin.updateSubjectStatus";

    // Only the admin.
    let (status, _) = server.post(update, Some(&access), &json!({})).await;
    assert_eq!(status, 401);
    let (status, account) = admin_get(info.clone()).await;
    assert_eq!(status, 200, "{account}");
    assert_eq!(
        (
            &account["handle"],
            &account["email"],
            &account["invitesDisabled"]
        ),
        (
            &json!("irene.pds-users.org"),
            &json!("irene@example.com"),
            &json!(false)
        )
    );
    let (status, body) = admin_get(
        "com.atproto.admin.getAccountInfo?did=did:plc:zzzzzzzzzzzzzzzzzzzzzzzz".to_owned(),
    )
    .await;
    assert_eq!((status, text(&body, "error")), (400, "NotFound"));

    // A takedown ends the sessions and is in the log; lifting it restores
    // the account.
    let repo = json!({"$type": "com.atproto.admin.defs#repoRef", "did": did});
    let (status, body) = admin_post(
        update,
        json!({"subject": repo, "takedown": {"applied": true, "ref": "case-1"}}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["takedown"], json!({"applied": true, "ref": "case-1"}));
    let (_, state) = admin_get(subject.clone()).await;
    assert_eq!(state["takedown"], json!({"applied": true, "ref": "case-1"}));
    assert_eq!(state["deactivated"], json!({"applied": false}));
    let (status, body) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&made, "refreshJwt")),
            &Value::Null,
        )
        .await;
    assert_eq!((status, text(&body, "error")), (401, "AccountTakedown"));
    let (status, _) = admin_post(
        update,
        json!({"subject": repo, "takedown": {"applied": false}}),
    )
    .await;
    assert_eq!(status, 200);
    let (_, state) = admin_get(subject).await;
    assert_eq!(state["takedown"], json!({"applied": false}));
    let events: Vec<String> = common::open(&server.dir.root.join("sequencer.sqlite"))
        .prepare(r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq""#)
        .unwrap()
        .query_map([&did], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        events,
        [
            "identity", "account", "append", "sync", "account", "account"
        ]
    );

    // A record is taken down and no longer read.
    let (_, session) = server
        .post(
            "com.atproto.server.createSession",
            None,
            &json!({"identifier": did, "password": PASSWORD}),
        )
        .await;
    let access = text(&session, "accessJwt");
    let note = "com.example.test.note";
    let (_, written) = server
        .post(
            "com.atproto.repo.createRecord",
            Some(access),
            &json!({"repo": did, "collection": note, "rkey": "one", "record": {"$type": note}}),
        )
        .await;
    let uri = text(&written, "uri").to_owned();
    let strong = json!({"$type": "com.atproto.repo.strongRef", "uri": uri, "cid": written["cid"]});
    let (status, _) = admin_post(
        update,
        json!({"subject": strong, "takedown": {"applied": true}}),
    )
    .await;
    assert_eq!(status, 200);
    let (_, state) = admin_get(format!("com.atproto.admin.getSubjectStatus?uri={uri}")).await;
    assert_eq!(state["takedown"]["applied"], true);
    assert!(state["takedown"]["ref"].is_string());
    let read = server
        .http
        .get(format!(
            "{}/xrpc/com.atproto.repo.getRecord?repo={did}&collection={note}&rkey=one",
            server.origin
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status().as_u16(), 400);
    let (status, body) = admin_post(
        update,
        json!({"subject": {"$type": "other"}, "takedown": {"applied": true}}),
    )
    .await;
    assert_eq!(
        (status, text(&body, "message")),
        (400, "Invalid subject (other)")
    );
}

#[tokio::test]
async fn the_admin_makes_invite_codes_and_sees_who_used_them() {
    use base64::Engine as _;
    let server = Server::start(&[("PDS_INVITE_REQUIRED", "true")]).await;
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("admin:{}", common::ADMIN_PASSWORD))
    );
    let call = |method: reqwest::Method, path: String, body: Option<Value>| {
        let mut request = server
            .http
            .request(method, format!("{}/xrpc/{path}", server.origin))
            .header("authorization", &basic);
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        async move {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            (status, body)
        }
    };
    let post = reqwest::Method::POST;
    let get = reqwest::Method::GET;

    let (status, _) = server
        .post(
            "com.atproto.server.createInviteCode",
            None,
            &json!({"useCount": 1}),
        )
        .await;
    assert_eq!(status, 401);
    let (status, made) = call(
        post.clone(),
        "com.atproto.server.createInviteCode".to_owned(),
        Some(json!({"useCount": 2})),
    )
    .await;
    assert_eq!(status, 200, "{made}");
    let code = text(&made, "code").to_owned();
    assert!(
        code.starts_with("pds-test-") && code.len() == "pds-test-".len() + 11,
        "{code}"
    );
    let (_, batch) = call(
        post,
        "com.atproto.server.createInviteCodes".to_owned(),
        Some(json!({"codeCount": 3, "useCount": 1})),
    )
    .await;
    assert_eq!(batch["codes"][0]["account"], "admin");
    assert_eq!(batch["codes"][0]["codes"].as_array().unwrap().len(), 3);

    let (status, joined) = server
        .create(&json!({"handle": "jules.pds-users.org", "email": "jules@example.com", "password": PASSWORD, "inviteCode": code}))
        .await;
    assert_eq!(status, 200, "{joined}");
    let did = text(&joined, "did");

    let (_, info) = call(
        get.clone(),
        format!("com.atproto.admin.getAccountInfo?did={did}"),
        None,
    )
    .await;
    assert_eq!(info["invitedBy"]["code"], code.as_str());
    assert_eq!(info["invitedBy"]["uses"][0]["usedBy"], did);
    assert_eq!(info["invites"], json!([]));
    let (_, infos) = call(
        get.clone(),
        format!(
            "com.atproto.admin.getAccountInfos?dids={did}&dids={}",
            common::ALICE
        ),
        None,
    )
    .await;
    assert_eq!(infos["infos"].as_array().unwrap().len(), 2);

    let (status, listed) = call(
        get.clone(),
        "com.atproto.admin.getInviteCodes?sort=usage&limit=2".to_owned(),
        None,
    )
    .await;
    assert_eq!(status, 200, "{listed}");
    let codes = listed["codes"].as_array().unwrap();
    assert_eq!(codes.len(), 2);
    assert_eq!(
        (&codes[0]["code"], &codes[0]["available"]),
        (&json!(code), &json!(2))
    );
    let cursor = text(&listed, "cursor").to_owned();
    let (_, rest) = call(
        get.clone(),
        format!("com.atproto.admin.getInviteCodes?sort=usage&limit=10&cursor={cursor}"),
        None,
    )
    .await;
    assert_eq!(rest["codes"].as_array().unwrap().len(), 2);
    let (_, recent) = call(
        get.clone(),
        "com.atproto.admin.getInviteCodes".to_owned(),
        None,
    )
    .await;
    assert_eq!(recent["codes"].as_array().unwrap().len(), 4);
    let (status, body) = call(
        get,
        "com.atproto.admin.getInviteCodes?sort=other".to_owned(),
        None,
    )
    .await;
    assert_eq!(
        (status, text(&body, "message")),
        (400, "unknown sort method: other")
    );
}

#[tokio::test]
async fn email_tokens_confirm_an_address_change_it_and_reset_a_password() {
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "karen.pds-users.org", "email": "karen@example.com", "password": PASSWORD}))
        .await;
    let (did, access) = (
        text(&made, "did").to_owned(),
        text(&made, "accessJwt").to_owned(),
    );
    let db = || common::open(&server.dir.root.join("account.sqlite"));
    let token = |purpose: &str| -> String {
        db().query_row(
            r#"select "token" from "email_token" where "did" = ?1 and "purpose" = ?2"#,
            [did.as_str(), purpose],
            |row| row.get(0),
        )
        .unwrap()
    };
    let call = |nsid: &'static str, body: Value| {
        let (server, access) = (&server, &access);
        async move { server.post(nsid, Some(access), &body).await }
    };
    let refused = |answer: (u16, Value)| (answer.0, text(&answer.1, "error").to_owned());

    // Confirm the address.
    assert_eq!(
        call("com.atproto.server.requestEmailConfirmation", Value::Null)
            .await
            .0,
        200
    );
    let confirm = token("confirm_email");
    assert_eq!(confirm.len(), 11);
    assert_eq!(confirm, confirm.to_uppercase());
    let confirm_email = "com.atproto.server.confirmEmail";
    assert_eq!(
        refused(
            call(
                confirm_email,
                json!({"email": "other@example.com", "token": confirm})
            )
            .await
        ),
        (400, "InvalidEmail".to_owned())
    );
    assert_eq!(
        refused(
            call(
                confirm_email,
                json!({"email": "karen@example.com", "token": "AAAAA-BBBBB"})
            )
            .await
        ),
        (400, "InvalidToken".to_owned())
    );
    // A token is read in any case, once.
    let (status, body) = call(
        confirm_email,
        json!({"email": "Karen@Example.com", "token": confirm.to_lowercase()}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        refused(
            call(
                confirm_email,
                json!({"email": "karen@example.com", "token": confirm})
            )
            .await
        ),
        (400, "InvalidToken".to_owned())
    );

    // Change the address: a confirmed one needs a token.
    let update = "com.atproto.server.updateEmail";
    let (_, asked) = call("com.atproto.server.requestEmailUpdate", Value::Null).await;
    assert_eq!(asked, json!({"tokenRequired": true}));
    assert_eq!(
        refused(call(update, json!({"email": "karen2@example.com"})).await),
        (400, "TokenRequired".to_owned())
    );
    let (status, body) = call(
        update,
        json!({"email": "alice@example.com", "token": token("update_email")}),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = call(
        update,
        json!({"email": "Karen2@example.com", "token": token("update_email")}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (_, asked) = call("com.atproto.server.requestEmailUpdate", Value::Null).await;
    assert_eq!(asked, json!({"tokenRequired": false}));

    // Reset the password: a token past 15 minutes is expired, a fresh one
    // sets the password and ends the sessions.
    let reset = "com.atproto.server.resetPassword";
    let (status, body) = server
        .post(
            "com.atproto.server.requestPasswordReset",
            None,
            &json!({"email": "nobody@example.com"}),
        )
        .await;
    assert_eq!(
        (status, text(&body, "message")),
        (400, "account does not have an email address")
    );
    assert_eq!(
        server
            .post(
                "com.atproto.server.requestPasswordReset",
                None,
                &json!({"email": "KAREN2@example.com"})
            )
            .await
            .0,
        200
    );
    let first = token("reset_password");
    db().execute(
        r#"update "email_token" set "requestedAt" = '2000-01-01T00:00:00.000Z' where "purpose" = 'reset_password'"#,
        [],
    )
    .unwrap();
    assert_eq!(
        refused(
            server
                .post(
                    reset,
                    None,
                    &json!({"token": first, "password": "new-password"})
                )
                .await
        ),
        (400, "ExpiredToken".to_owned())
    );
    server
        .post(
            "com.atproto.server.requestPasswordReset",
            None,
            &json!({"email": "karen2@example.com"}),
        )
        .await;
    let (status, body) = server
        .post(
            reset,
            None,
            &json!({"token": token("reset_password"), "password": "new-password"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let login = |password: &'static str| {
        let body = json!({"identifier": did, "password": password});
        let server = &server;
        async move {
            server
                .post("com.atproto.server.createSession", None, &body)
                .await
        }
    };
    assert_eq!(login(PASSWORD).await.0, 401);
    assert_eq!(login("new-password").await.0, 200);
    let (status, body) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&made, "refreshJwt")),
            &Value::Null,
        )
        .await;
    assert_eq!((status, text(&body, "error")), (400, "ExpiredToken"));
}

#[tokio::test]
async fn an_account_is_deleted_by_its_owner_and_by_the_admin() {
    use base64::Engine as _;
    let server = Server::start(&[]).await;
    let make = |name: &'static str| {
        let body = json!({"handle": format!("{name}.pds-users.org"),
                          "email": format!("{name}@example.com"), "password": PASSWORD});
        let server = &server;
        async move { server.create(&body).await.1 }
    };
    let (laura, nolan) = (make("laura").await, make("nolan").await);
    let did = text(&laura, "did").to_owned();
    let access = text(&laura, "accessJwt").to_owned();
    let events = |did: &str| -> Vec<String> {
        common::open(&server.dir.root.join("sequencer.sqlite"))
            .prepare(r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq""#)
            .unwrap()
            .query_map([did], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };

    assert_eq!(
        server
            .post(
                "com.atproto.server.requestAccountDelete",
                Some(&access),
                &Value::Null
            )
            .await
            .0,
        200
    );
    let token: String = common::open(&server.dir.root.join("account.sqlite"))
        .query_row(
            r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'delete_account'"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    let delete = "com.atproto.server.deleteAccount";
    let (status, body) = server
        .post(
            delete,
            None,
            &json!({"did": did, "password": "wrong", "token": token}),
        )
        .await;
    assert_eq!(
        (status, text(&body, "message")),
        (401, "Invalid did or password")
    );
    let (status, body) = server
        .post(
            delete,
            None,
            &json!({"did": did, "password": PASSWORD, "token": "AAAAA-BBBBB"}),
        )
        .await;
    assert_eq!((status, text(&body, "error")), (400, "InvalidToken"));
    let (status, body) = server
        .post(
            delete,
            None,
            &json!({"did": did, "password": PASSWORD, "token": token}),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // The account is gone: its rows, its store, and its events but one.
    assert!(!server.dir.config.layout.actor_store_dir(&did).exists());
    assert_eq!(events(&did), ["account"]);
    let (status, _) = server
        .post(
            "com.atproto.server.createSession",
            None,
            &json!({"identifier": did, "password": PASSWORD}),
        )
        .await;
    assert_eq!(status, 401);
    let (status, body) = server
        .post(
            delete,
            None,
            &json!({"did": did, "password": PASSWORD, "token": token}),
        )
        .await;
    assert_eq!((status, text(&body, "message")), (400, "account not found"));
    let report = linkjar_pds::verify::verify(&server.dir.config).unwrap();
    assert_eq!(report.findings.len(), 0, "{report}");

    // The admin deletes without a password and a token.
    let other = text(&nolan, "did").to_owned();
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("admin:{}", common::ADMIN_PASSWORD))
    );
    let response = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.deleteAccount",
            server.origin
        ))
        .header("authorization", basic)
        .header("content-type", "application/json")
        .body(json!({"did": other}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(events(&other), ["account"]);
    assert!(!server.dir.config.layout.actor_store_dir(&other).exists());
}

#[tokio::test]
async fn the_admin_changes_the_email_the_handle_and_the_password_of_an_account() {
    use base64::Engine as _;
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "oscar.pds-users.org", "email": "oscar@example.com", "password": PASSWORD}))
        .await;
    let did = text(&made, "did").to_owned();
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("admin:{}", common::ADMIN_PASSWORD))
    );
    let admin = |nsid: &'static str, body: Value| {
        let request = server
            .http
            .post(format!("{}/xrpc/{nsid}", server.origin))
            .header("authorization", &basic)
            .header("content-type", "application/json")
            .body(body.to_string());
        async move {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            let bytes = response.bytes().await.unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    let login = |identifier: &'static str, password: &'static str| {
        let body = json!({"identifier": identifier, "password": password});
        let server = &server;
        async move {
            server
                .post("com.atproto.server.createSession", None, &body)
                .await
                .0
        }
    };

    let email = "com.atproto.admin.updateAccountEmail";
    assert_eq!(
        admin(
            email,
            json!({"account": "oscar.pds-users.org", "email": "Oscar2@example.com"})
        )
        .await
        .0,
        200
    );
    let (status, body) = admin(
        email,
        json!({"account": "nobody.pds-users.org", "email": "x@example.com"}),
    )
    .await;
    assert_eq!(
        (status, text(&body, "message")),
        (400, "Account does not exist: nobody.pds-users.org")
    );
    assert_eq!(login("oscar2@example.com", PASSWORD).await, 200);

    // The admin may give a reserved name; the account may not.
    let (status, body) = admin(
        "com.atproto.admin.updateAccountHandle",
        json!({"did": did, "handle": "admin.pds-users.org"}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(login("admin.pds-users.org", PASSWORD).await, 200);

    let password = "com.atproto.admin.updateAccountPassword";
    assert_eq!(
        admin(password, json!({"did": did, "password": "x".repeat(257)}))
            .await
            .0,
        400
    );
    assert_eq!(
        admin(
            password,
            json!({"did": did, "password": "changed-by-admin"})
        )
        .await
        .0,
        200
    );
    assert_eq!(login("oscar2@example.com", PASSWORD).await, 401);
    assert_eq!(login("oscar2@example.com", "changed-by-admin").await, 200);
    let (status, _) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&made, "refreshJwt")),
            &Value::Null,
        )
        .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn a_plc_operation_is_recommended_signed_and_submitted() {
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "petra.pds-users.org", "email": "petra@example.com", "password": PASSWORD}))
        .await;
    let (did, access) = (
        text(&made, "did").to_owned(),
        text(&made, "accessJwt").to_owned(),
    );
    let genesis = server.ops.lock().unwrap()[0].1.clone();

    let response = server
        .http
        .get(format!(
            "{}/xrpc/com.atproto.identity.getRecommendedDidCredentials",
            server.origin
        ))
        .bearer_auth(&access)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let recommended: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(recommended["alsoKnownAs"], genesis["alsoKnownAs"]);
    assert_eq!(
        recommended["verificationMethods"],
        genesis["verificationMethods"]
    );
    assert_eq!(recommended["rotationKeys"], genesis["rotationKeys"]);
    assert_eq!(recommended["services"], genesis["services"]);

    // Signing needs the token of the mail.
    let sign = "com.atproto.identity.signPlcOperation";
    let (status, body) = server.post(sign, Some(&access), &json!({})).await;
    assert_eq!(
        (status, text(&body, "message")),
        (
            400,
            "email confirmation token required to sign PLC operations"
        )
    );
    assert_eq!(
        server
            .post(
                "com.atproto.identity.requestPlcOperationSignature",
                Some(&access),
                &Value::Null
            )
            .await
            .0,
        200
    );
    let token: String = common::open(&server.dir.root.join("account.sqlite"))
        .query_row(
            r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'plc_operation'"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    let extra = "did:key:zQ3shokFTS3brHcDQrn82RUDfCZESWL1ZdCEJwekUDPQiYBme";
    let mut keys = vec![json!(extra)];
    keys.extend(genesis["rotationKeys"].as_array().unwrap().iter().cloned());
    let (status, signed) = server
        .post(
            sign,
            Some(&access),
            &json!({"token": token, "rotationKeys": keys}),
        )
        .await;
    assert_eq!(status, 200, "{signed}");
    let operation = signed["operation"].clone();
    assert_eq!(operation["rotationKeys"][0], extra);
    assert_eq!(operation["alsoKnownAs"], genesis["alsoKnownAs"]);
    assert!(operation["prev"].is_string() && operation["sig"].is_string());
    // The token is used up.
    let (status, body) = server
        .post(sign, Some(&access), &json!({"token": token}))
        .await;
    assert_eq!((status, text(&body, "error")), (400, "InvalidToken"));

    // Submitting keeps the account on this server.
    let submit = "com.atproto.identity.submitPlcOperation";
    let with = |change: &dyn Fn(&mut Value)| {
        let mut op = operation.clone();
        change(&mut op);
        json!({ "operation": op })
    };
    for (body, message) in [
        (
            json!({"operation": {"type": "plc_tombstone"}}),
            "Invalid operation",
        ),
        (
            with(&|op| op["rotationKeys"] = json!([extra])),
            "Rotation keys do not include server's rotation key",
        ),
        (
            with(&|op| op["services"]["atproto_pds"]["endpoint"] = json!("https://elsewhere.test")),
            "Incorrect endpoint on atproto_pds service",
        ),
        (
            with(&|op| op["verificationMethods"]["atproto"] = json!(extra)),
            "Incorrect signing key",
        ),
        (
            with(&|op| op["alsoKnownAs"] = json!(["at://someone.else"])),
            "Incorrect handle in alsoKnownAs",
        ),
    ] {
        let (status, answer) = server.post(submit, Some(&access), &body).await;
        assert_eq!((status, text(&answer, "message")), (400, message));
    }
    let (status, body) = server.post(submit, Some(&access), &with(&|_| {})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(server.ops.lock().unwrap().len(), 2);
    let last: String = common::open(&server.dir.root.join("sequencer.sqlite"))
        .query_row(
            r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq" desc limit 1"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(last, "identity");
}
