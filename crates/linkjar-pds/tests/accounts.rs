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
    ops_origin: String,
    http: reqwest::Client,
    dir: DataDir,
    ops: Ops,
}

impl Server {
    async fn start(extra: &[(&str, &str)]) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (plc, ops) = plc().await;
        Self::start_on(listener, &plc, ops, extra).await
    }

    /// A server on a listener that exists, with a PLC directory that
    /// another server may share.
    async fn start_on(
        listener: tokio::net::TcpListener,
        plc: &str,
        ops: Ops,
        extra: &[(&str, &str)],
    ) -> Self {
        let mut vars = vec![
            ("PDS_DID_PLC_URL", plc),
            ("PDS_SERVICE_HANDLE_DOMAINS", ".pds-users.org"),
            (
                "PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX",
                "0707070707070707070707070707070707070707070707070707070707070707",
            ),
        ];
        vars.extend_from_slice(extra);
        let dir = DataDir::new_with(1, &vars);
        let origin = format!("http://{}", listener.local_addr().unwrap());
        // The start of the server applies the migrations of the profile.
        linkjar_pds::serve::create_missing_databases(&dir.config).unwrap();
        let state = app_state(&dir.config).unwrap();
        let router = pds_xrpc::router(state.clone());
        tokio::spawn(async move { axum::serve(listener, router).await });
        // The ops listener, on an address of its own (SPEC §22.1).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ops_origin = format!("http://{}", listener.local_addr().unwrap());
        let router = linkjar_pds::serve::ops_router(state, &dir.config);
        tokio::spawn(async move { axum::serve(listener, router).await });
        Self {
            origin,
            ops_origin,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
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

    /// A request of the admin to the ops listener. `reason` goes in the
    /// header of an action.
    async fn operate(
        &self,
        nsid: &str,
        reason: Option<&str>,
        body: Option<&Value>,
    ) -> (u16, Value) {
        let url = format!("{}/xrpc/{nsid}", self.ops_origin);
        let mut request = match body {
            Some(body) => self
                .http
                .post(url)
                .header("content-type", "application/json")
                .body(body.to_string()),
            None => self.http.get(url),
        }
        .basic_auth("admin", Some(common::ADMIN_PASSWORD));
        if let Some(reason) = reason {
            request = request.header("x-ops-reason", reason);
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

#[tokio::test]
async fn a_signing_key_is_reserved_once_for_a_did() {
    let server = Server::start(&[]).await;
    let reserve = "com.atproto.server.reserveSigningKey";
    let did = json!({"did": "did:plc:aaaaaaaaaaaaaaaaaaaaaaaz"});
    let (status, first) = server.post(reserve, None, &did).await;
    assert_eq!(status, 200, "{first}");
    let key = first["signingKey"].as_str().unwrap();
    assert!(key.starts_with("did:key:zQ3s"), "{key}");
    let (_, again) = server.post(reserve, None, &did).await;
    assert_eq!(again["signingKey"], first["signingKey"]);
    // Without a DID each call reserves another key.
    let (_, one) = server.post(reserve, None, &json!({})).await;
    let (_, two) = server.post(reserve, None, &json!({})).await;
    assert_ne!(one["signingKey"], two["signingKey"]);
    let (status, body) = server.post(reserve, None, &json!({"did": "../x"})).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("InvalidRequest"))
    );
}

const LINKJAR: &[(&str, &str)] = &[
    ("PDS_PROFILE", "linkjar"),
    ("PDS_SERVICE_HANDLE_DOMAINS", ".linkjar.social"),
];

#[tokio::test]
async fn the_linkjar_profile_reserves_names_and_allows_one_hosted_rename() {
    let server = Server::start(LINKJAR).await;
    let account = |label: &str| {
        json!({
            "handle": format!("{label}.linkjar.social"),
            "email": format!("{}@example.com", label.replace('-', "")),
            "password": PASSWORD,
        })
    };
    for label in [
        "linkjar", "pds2", "api-prod", "link-jar", "l1nkj4r", "supp0rt",
    ] {
        let (status, body) = server.create(&account(label)).await;
        assert_eq!(
            (status, text(&body, "error"), text(&body, "message")),
            (400, "HandleNotAvailable", "Reserved handle"),
            "{label}"
        );
    }
    let (status, _) = server.create(&account("jared")).await;
    assert_eq!(
        status, 200,
        "a name that only contains a service word is free"
    );

    let (status, created) = server.create(&account("fresh")).await;
    assert_eq!(status, 200, "{created}");
    let did = text(&created, "did").to_owned();
    let access = text(&created, "accessJwt").to_owned();
    let update = "com.atproto.identity.updateHandle";
    let rename = |handle: &str| json!({ "handle": format!("{handle}.linkjar.social") });
    // A password account has no policy row: it renames as upstream allows.
    for name in ["fresh2", "fresh3"] {
        let (status, body) = server.post(update, Some(&access), &rename(name)).await;
        assert_eq!(status, 200, "{name}: {body}");
    }
    // An account that an identity provider created has a row.
    let accounts = common::open(&server.dir.config.layout.account_db);
    let created_at = pds_sequencer::iso_millis(
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap(),
    );
    accounts
        .execute(
            r#"insert into "linkjar_handle_policy" ("did", "createdAt") values (?1, ?2)"#,
            [&did, &created_at],
        )
        .unwrap();
    let (status, _) = server.post(update, Some(&access), &rename("fresh3")).await;
    assert_eq!(status, 200, "an unchanged handle spends nothing");
    let (status, body) = server.post(update, Some(&access), &rename("chosen")).await;
    assert_eq!(status, 200, "{body}");
    let (status, _) = server.post(update, Some(&access), &rename("chosen")).await;
    assert_eq!(status, 200, "the chosen target stays available");
    let (status, body) = server.post(update, Some(&access), &rename("another")).await;
    assert_eq!(
        (status, text(&body, "error")),
        (400, "HandleNotAvailable"),
        "{body}"
    );
    assert!(text(&body, "message").starts_with("One hosted handle change is available"));
    let claimed: Option<String> = accounts
        .query_row(
            r#"select "renameHandle" from "linkjar_handle_policy" where "did" = ?1"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(claimed.as_deref(), Some("chosen.linkjar.social"));
    let migrations: i64 = accounts
        .query_row(
            r#"select count(*) from "kysely_migration" where "name" like '007_-linkjar-%'"#,
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(migrations, 4);
}

#[tokio::test]
async fn a_handle_host_serves_the_did_and_a_redirect_and_nothing_else() {
    let server = Server::start(LINKJAR).await;
    let (status, created) = server
        .create(&json!({"handle": "nova.linkjar.social", "email": "nova@example.com", "password": PASSWORD}))
        .await;
    assert_eq!(status, 200, "{created}");
    let did = text(&created, "did").to_owned();
    let get = |host: &'static str, path: &'static str| {
        let request = server
            .http
            .get(format!("{}{path}", server.origin))
            .header("host", host)
            .header("cookie", "session=x");
        async move {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            (status, headers, response.text().await.unwrap())
        }
    };
    let host = "nova.linkjar.social";
    let (status, headers, body) = get(host, "/.well-known/atproto-did").await;
    assert_eq!((status, body.as_str()), (200, did.as_str()));
    // The proxy in front adds the headers to the answers it forwards.
    assert!(headers.get("content-security-policy").is_none());
    for path in [
        "/.well-known/oauth-authorization-server",
        "/xrpc/com.atproto.server.describeServer",
        "/xrpc/_health",
    ] {
        let (status, headers, body) = get(host, path).await;
        assert_eq!(status, 404, "{path}");
        assert_eq!(body, "Use the PDS service endpoint at pds.test", "{path}");
        assert_eq!(
            headers["content-security-policy"],
            "default-src 'none'; sandbox"
        );
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["referrer-policy"], "no-referrer");
    }
    let page = format!("https://linkjar.io/jar/{}", did.replace(':', "%3A"));
    for path in [
        "/",
        "/anything/else?x=1",
        "/oauth/authorize",
        "/account",
        "/.well-known/linkjar-profile",
    ] {
        let (status, headers, _) = get(host, path).await;
        assert_eq!(status, 302, "{path}");
        assert_eq!(headers["location"].to_str().unwrap(), page, "{path}");
        assert!(headers.get("set-cookie").is_none());
    }
    // A handle nobody holds, a reserved one, and the service host.
    assert_eq!(get("nobody.linkjar.social", "/").await.0, 404);
    assert_eq!(get("support.linkjar.social", "/").await.0, 404);
    let (status, _, body) = get("pds.test", "/").await;
    assert_eq!(status, 200);
    assert!(body.contains("AT Protocol Personal Data Server"));
    assert_eq!(get("pds.test", "/.well-known/linkjar-profile").await.0, 404);
}

#[tokio::test]
async fn a_configured_captcha_closes_the_legacy_sign_up_in_the_linkjar_profile() {
    let captcha = [
        ("PDS_HCAPTCHA_SITE_KEY", "site"),
        ("PDS_HCAPTCHA_SECRET_KEY", "secret"),
        ("PDS_HCAPTCHA_TOKEN_SALT", "salt"),
    ];
    let account = json!({"handle": "fresh.linkjar.social", "email": "fresh@example.com", "password": PASSWORD});
    let mut vars = LINKJAR.to_vec();
    vars.extend_from_slice(&captcha);
    let server = Server::start(&vars).await;
    let (status, body) = server.create(&account).await;
    assert_eq!(
        (status, text(&body, "error"), text(&body, "message")),
        (
            400,
            "InvalidRequest",
            "Create an account through the OAuth signup page"
        )
    );
    // The stock profile keeps the method open, as upstream does.
    let mut vars = vec![("PDS_SERVICE_HANDLE_DOMAINS", ".linkjar.social")];
    vars.extend_from_slice(&captcha);
    let stock = Server::start(&vars).await;
    assert_eq!(stock.create(&account).await.0, 200);
}

#[tokio::test]
async fn account_and_admin_actions_are_in_an_audit_log_that_verifies() {
    use base64::Engine as _;
    let server = Server::start(&[
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
        (
            "PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX",
            "0909090909090909090909090909090909090909090909090909090909090909",
        ),
    ])
    .await;
    let account =
        json!({"handle": "judy.pds-users.org", "email": "judy@example.com", "password": PASSWORD});
    let (status, made) = server.create(&account).await;
    assert_eq!(status, 200, "{made}");
    let did = text(&made, "did").to_owned();
    // A refused creation, a repository write and an admin action.
    assert_eq!(server.create(&account).await.0, 400);
    let record = json!({"repo": did, "collection": "com.example.note", "record": {"$type": "com.example.note"}});
    let (status, _) = server
        .post(
            "com.atproto.repo.createRecord",
            Some(text(&made, "accessJwt")),
            &record,
        )
        .await;
    assert_eq!(status, 200);
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("admin:{}", common::ADMIN_PASSWORD))
    );
    let takedown = json!({
        "subject": {"$type": "com.atproto.admin.defs#repoRef", "did": did},
        "takedown": {"applied": true, "ref": "case-1"},
    });
    let response = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.updateSubjectStatus",
            server.origin
        ))
        .header("authorization", &basic)
        .header("content-type", "application/json")
        .body(takedown.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);

    // The log has the three audited actions and not the repository write.
    let key = pds_audit::Signer::from_seed(&[9; 32]);
    let log = pds_xrpc::audit::AuditLog::open(&server.dir.config.layout.audit_db, key).unwrap();
    log.checkpoint("2026-10-13T00:00:00.000Z").unwrap().unwrap();
    let bundle = log.export(1, u64::MAX).unwrap();
    let entries: Vec<pds_audit::Entry> = bundle["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(entry.as_str().unwrap())
                .unwrap();
            pds_audit::Entry::from_bytes(&bytes).unwrap()
        })
        .collect();
    let seen: Vec<(&str, &str, Option<&str>, &str)> = entries
        .iter()
        .map(|entry| {
            let what = &entry.what;
            (
                what.action.as_str(),
                what.actor.as_str(),
                what.subject.as_deref(),
                what.outcome.as_str(),
            )
        })
        .collect();
    let key_id = seen_key(&entries);
    assert!(key_id.is_some_and(|id| id.starts_with("ed25519:")));
    let created = "com.atproto.server.createAccount";
    assert_eq!(
        seen,
        [
            (created, "anonymous", Some(did.as_str()), "ok"),
            (created, "anonymous", None, "InvalidRequest"),
            (
                "com.atproto.admin.updateSubjectStatus",
                "admin",
                Some(did.as_str()),
                "ok"
            ),
            ("audit.key", "server", key_id, "ok"),
        ]
    );
    let report = pds_audit::verify::verify(&bundle);
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!((report.entries, report.checkpoints), (4, 1));
}

/// The subject of the key entry: the id of the key, whatever it is.
fn seen_key(entries: &[pds_audit::Entry]) -> Option<&str> {
    entries
        .iter()
        .find(|entry| entry.what.action == "audit.key")
        .and_then(|entry| entry.what.subject.as_deref())
}

#[tokio::test]
async fn a_checkpoint_is_published_as_a_record_of_the_operator() {
    let dir = DataDir::new_with(
        1,
        &[
            (
                "PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX",
                "0909090909090909090909090909090909090909090909090909090909090909",
            ),
            ("PDS_AUDIT_CHECKPOINT_REPO", common::ALICE),
        ],
    );
    let state = app_state(&dir.config).unwrap();
    let action = |name: &str| pds_audit::Draft {
        actor: "admin".to_owned(),
        action: name.to_owned(),
        outcome: "ok".to_owned(),
        ..pds_audit::Draft::default()
    };
    pds_xrpc::audit::record(&state, action("first")).await;
    pds_xrpc::audit::checkpoint(&state).await;
    // Nothing new: no second checkpoint and no second record.
    pds_xrpc::audit::checkpoint(&state).await;
    pds_xrpc::audit::record(&state, action("second")).await;
    pds_xrpc::audit::checkpoint(&state).await;

    let store = pds_storage::ActorDb::open(&dir.config.layout.actor_db(common::ALICE)).unwrap();
    let records = store
        .list_records(
            common::ALICE,
            "io.linkjar.pds.audit.checkpoint",
            10,
            true,
            None,
        )
        .unwrap();
    assert_eq!(records.len(), 2);
    let fields = |row: &pds_storage::RecordRow| match pds_types::Data::from_dag_cbor(&row.content) {
        Ok(pds_types::Data::Object(fields)) => fields,
        other => panic!("not a record: {other:?}"),
    };
    let (first, second) = (fields(&records[0]), fields(&records[1]));
    let number =
        |fields: &std::collections::BTreeMap<String, pds_types::Data>, key: &str| match fields
            .get(key)
        {
            Some(pds_types::Data::Integer(n)) => *n,
            other => panic!("{key}: {other:?}"),
        };
    let bytes =
        |fields: &std::collections::BTreeMap<String, pds_types::Data>, key: &str| match fields
            .get(key)
        {
            Some(pds_types::Data::Bytes(bytes)) => bytes.0.clone(),
            other => panic!("{key}: {other:?}"),
        };
    // The first covers the entry and the entry of its key.
    assert_eq!(
        (number(&first, "firstSeq"), number(&first, "lastSeq")),
        (1, 2)
    );
    assert_eq!(
        (number(&second, "firstSeq"), number(&second, "lastSeq")),
        (3, 3)
    );
    assert_eq!(bytes(&first, "previousRoot"), vec![0; 32]);
    assert_eq!(bytes(&second, "previousRoot"), bytes(&first, "root"));
    assert_eq!(bytes(&first, "signature").len(), 64);
    // The record went out on the firehose as a commit of the repository.
    let events: i64 = common::open(&dir.config.layout.sequencer_db)
        .query_row(
            r#"select count(*) from "repo_seq" where "did" = ?1 and "eventType" = 'append'"#,
            [common::ALICE],
            |row| row.get(0),
        )
        .unwrap();
    assert!(events >= 2, "{events}");
}

#[tokio::test]
async fn an_account_moves_here_with_its_did_and_starts_deactivated() {
    const MOVER: &str = "did:plc:mmmmmmmmmmmmmmmmmmmmmmmm";
    let server = Server::start(&[]).await;
    // The DID document of an account that another server hosts.
    let multikey = common::key(9).public_key().to_multikey();
    let doc = format!(
        r#"{{"@context":["https://www.w3.org/ns/did/v1"],"id":"{MOVER}","alsoKnownAs":["at://mover.elsewhere.org"],"verificationMethod":[{{"id":"{MOVER}#atproto","type":"Multikey","controller":"{MOVER}","publicKeyMultibase":"{multikey}"}}]}}"#
    );
    common::open(&server.dir.config.layout.did_cache_db)
        .execute(
            r#"insert into "did_doc" values (?1, ?2, ?3)"#,
            rusqlite::params![MOVER, doc, i64::MAX / 2],
        )
        .unwrap();
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap();
    let token = |key: u8| {
        pds_xrpc::api::proxy::service_jwt(
            &common::key(key),
            MOVER,
            "did:web:pds.test",
            Some("com.atproto.server.createAccount"),
            now,
            now + 60,
            &[key; 16],
        )
    };
    let account = json!({
        "did": MOVER,
        "handle": "mover.pds-users.org",
        "email": "mover@example.com",
        "password": PASSWORD,
    });
    let create = "com.atproto.server.createAccount";
    let before = server.ops.lock().unwrap().len();

    // Without a token.
    let (status, body) = server.create(&account).await;
    assert_eq!(
        (status, text(&body, "message")),
        (
            401,
            format!("Missing auth to create account with did: {MOVER}").as_str()
        )
    );
    let (status, made) = server.post(create, Some(&token(9)), &account).await;
    assert_eq!(status, 200, "{made}");
    assert_eq!(text(&made, "did"), MOVER);
    assert_eq!(text(&made, "handle"), "mover.pds-users.org");
    let access = text(&made, "accessJwt").to_owned();

    // The account is here, deactivated. The server wrote nothing to the
    // PLC directory and nothing to the firehose.
    assert_eq!(server.ops.lock().unwrap().len(), before);
    let deactivated: Option<String> = common::open(&server.dir.config.layout.account_db)
        .query_row(
            r#"select "deactivatedAt" from "actor" where "did" = ?1"#,
            [MOVER],
            |row| row.get(0),
        )
        .unwrap();
    assert!(deactivated.is_some());
    let events: i64 = common::open(&server.dir.config.layout.sequencer_db)
        .query_row(
            r#"select count(*) from "repo_seq" where "did" = ?1"#,
            [MOVER],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(events, 0);
    let response = server
        .http
        .get(format!(
            "{}/xrpc/com.atproto.server.checkAccountStatus",
            server.origin
        ))
        .bearer_auth(&access)
        .send()
        .await
        .unwrap();
    let status: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(status["activated"], false, "{status}");
    assert_eq!(status["repoBlocks"], 2);

    // A token that another key signed brings no DID. The check comes
    // last: a bad signature makes the server ask the directory again, and
    // this directory does not know the DID.
    let other = json!({
        "did": MOVER,
        "handle": "mover2.pds-users.org",
        "email": "mover2@example.com",
        "password": PASSWORD,
    });
    let (status, _) = server.post(create, Some(&token(8)), &other).await;
    assert_eq!(status, 401);
}

#[tokio::test]
async fn the_metrics_count_requests_by_route_and_read_the_audit_log() {
    let dir = DataDir::new(1);
    let state = app_state(&dir.config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = pds_xrpc::router(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await });
    let http = reqwest::Client::new();
    for path in [
        "/xrpc/_health",
        "/xrpc/_health",
        "/xrpc/app.bsky.feed.getTimeline",
        "/wp-login.php",
        "/xrpc/com.atproto.admin.getAccountInfo?did=did:plc:aaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        http.get(format!("{origin}{path}")).send().await.unwrap();
    }
    let text = tokio::task::spawn_blocking(move || pds_xrpc::metrics::render(&state))
        .await
        .unwrap();
    for line in [
        "pds_build_info{version=",
        "pds_http_requests_total{route=\"/xrpc/_health\",status=\"200\"} 2",
        // A method of another service and a path nobody serves have one label each.
        "pds_http_requests_total{route=\"proxied\",status=\"400\"} 1",
        "pds_http_requests_total{route=\"other\",status=\"404\"} 1",
        "pds_http_request_duration_seconds_count{route=\"/xrpc/_health\"} 2",
        // The admin method was refused and recorded.
        "pds_audit_entries 1",
        "pds_audit_unsigned_entries 1",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    assert!(!text.contains("wp-login"));
}

#[tokio::test]
async fn the_admin_reads_and_acts_on_the_ops_listener_with_a_reason() {
    let server = Server::start(&[("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD)]).await;
    let account = json!({"handle": "olive.pds-users.org", "email": "olive@example.com", "password": PASSWORD});
    let (status, made) = server.create(&account).await;
    assert_eq!(status, 200, "{made}");
    let did = text(&made, "did").to_owned();

    // No credential, and the credential of an account.
    let anonymous = server
        .http
        .get(format!(
            "{}/xrpc/internal.ops.getOverview",
            server.ops_origin
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);
    let session = server
        .http
        .get(format!(
            "{}/xrpc/internal.ops.getOverview",
            server.ops_origin
        ))
        .bearer_auth(text(&made, "accessJwt"))
        .send()
        .await
        .unwrap();
    assert_eq!(session.status(), 401);

    // Reads.
    let (status, operator) = server.operate("internal.ops.getOperator", None, None).await;
    assert_eq!(status, 200);
    assert_eq!(operator, json!({"actor": "admin", "role": "admin"}));
    let (status, overview) = server.operate("internal.ops.getOverview", None, None).await;
    assert_eq!(status, 200, "{overview}");
    assert_eq!(overview["accounts"], 4);
    assert_eq!(overview["subscribers"], 0);
    assert_eq!(overview["profile"], "stock");
    // The fixture has three accounts.
    let (_, first) = server
        .operate("internal.ops.listAccounts?limit=3", None, None)
        .await;
    assert_eq!(first["accounts"].as_array().unwrap().len(), 3);
    let cursor = text(&first, "cursor").replace(':', "%3A");
    let (_, second) = server
        .operate(
            &format!("internal.ops.listAccounts?limit=3&cursor={cursor}"),
            None,
            None,
        )
        .await;
    assert_eq!(second["accounts"].as_array().unwrap().len(), 1, "{second}");
    assert!(second["cursor"].is_null());
    let listed = [&first, &second]
        .iter()
        .flat_map(|page| page["accounts"].as_array().unwrap())
        .filter(|account| account["did"] == did.as_str())
        .count();
    assert_eq!(listed, 1);
    let (status, card) = server
        .operate(&format!("internal.ops.getAccount?did={did}"), None, None)
        .await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(card["email"], "olive@example.com");
    assert_eq!(card["status"], "active");
    assert_eq!(card["repo"]["records"], 0);
    assert_eq!(card["legacySessions"].as_array().unwrap().len(), 1);
    let (status, report) = server.operate("internal.ops.verify", None, None).await;
    assert_eq!(status, 200, "{report}");

    // An action needs a reason, and a method the listener does not have
    // writes no entry.
    let block = json!({"domain": "Spam.Example"});
    let (status, refused) = server
        .operate("internal.ops.blockEmailDomain", None, Some(&block))
        .await;
    assert_eq!(status, 400, "{refused}");
    let (status, _) = server
        .operate(
            "internal.ops.noSuchAction",
            Some("a test"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 501);
    let (status, _) = server
        .operate(
            "com.atproto.repo.createRecord",
            Some("a test"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 501);

    // A blocked email domain closes the sign-up to its addresses.
    let (status, _) = server
        .operate(
            "internal.ops.blockEmailDomain",
            Some("a wave of sign-ups"),
            Some(&block),
        )
        .await;
    assert_eq!(status, 200);
    let spam = json!({"handle": "spam.pds-users.org", "email": "a@mail.spam.example", "password": PASSWORD});
    let (status, refused) = server.create(&spam).await;
    assert_eq!(status, 400, "{refused}");
    let (_, domains) = server
        .operate("internal.ops.listBlockedEmailDomains", None, None)
        .await;
    assert_eq!(domains["domains"][0]["domain"], "spam.example");
    assert_eq!(domains["domains"][0]["reason"], "a wave of sign-ups");
    let lift = json!({"domain": "spam.example"});
    let (status, _) = server
        .operate(
            "internal.ops.unblockEmailDomain",
            Some("the wave ended"),
            Some(&lift),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(server.create(&spam).await.0, 200);

    // A role, for an account of the server only.
    let grant = json!({"did": did, "role": "moderator"});
    let (status, _) = server
        .operate(
            "internal.ops.grantRole",
            Some("joins the team"),
            Some(&grant),
        )
        .await;
    assert_eq!(status, 200);
    let stranger = json!({"did": "did:plc:zzzzzzzzzzzzzzzzzzzzzzzz", "role": "operator"});
    let (status, refused) = server
        .operate("internal.ops.grantRole", Some("a test"), Some(&stranger))
        .await;
    assert_eq!((status, text(&refused, "error")), (400, "AccountNotFound"));
    let (_, roles) = server.operate("internal.ops.listRoles", None, None).await;
    assert_eq!(roles["roles"][0]["role"], "moderator");
    assert_eq!(roles["roles"][0]["grantedBy"], "admin");

    // `#sync` for a repository.
    let events = |did: &str| -> i64 {
        common::open(&server.dir.config.layout.sequencer_db)
            .query_row(
                r#"select count(*) from "repo_seq" where "did" = ?1 and "eventType" = 'sync'"#,
                [did],
                |row| row.get(0),
            )
            .unwrap()
    };
    let before = events(&did);
    let (status, synced) = server
        .operate(
            "internal.ops.emitSync",
            Some("a relay lost the repository"),
            Some(&json!({"did": did})),
        )
        .await;
    assert_eq!(status, 200, "{synced}");
    assert!(synced["seq"].as_i64().unwrap() > 0);
    assert_eq!(events(&did), before + 1);

    // The sessions of an account end.
    let (status, ended) = server
        .operate(
            "internal.ops.revokeSessions",
            Some("a stolen phone"),
            Some(&json!({"did": did})),
        )
        .await;
    assert_eq!(status, 200, "{ended}");
    assert_eq!(ended["legacySessions"], 1);
    let (status, _) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&made, "refreshJwt")),
            &json!({}),
        )
        .await;
    assert_eq!(status, 400);

    // An admin method through the listener, and the housekeeping.
    let takedown = json!({
        "subject": {"$type": "com.atproto.admin.defs#repoRef", "did": did},
        "takedown": {"applied": true, "ref": "ops-1"},
    });
    let (status, body) = server
        .operate(
            "com.atproto.admin.updateSubjectStatus",
            Some("spam"),
            Some(&takedown),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, info) = server
        .operate(
            &format!("com.atproto.admin.getSubjectStatus?did={did}"),
            None,
            None,
        )
        .await;
    assert_eq!(status, 200, "{info}");
    assert_eq!(info["takedown"]["applied"], true);
    let (status, _) = server
        .operate(
            "internal.ops.purgeExpired",
            Some("housekeeping"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 200);
    let (status, refused) = server
        .operate(
            "internal.ops.disconnectSubscriber",
            Some("a test"),
            Some(&json!({"id": 99})),
        )
        .await;
    assert_eq!(
        (status, text(&refused, "error")),
        (400, "SubscriberNotFound")
    );

    // Each action is an entry with the operator, the reason and the
    // console. The newest is first. A read of the admin is an entry too.
    let (status, audit) = server
        .operate("internal.ops.listAudit?limit=5", None, None)
        .await;
    assert_eq!(status, 200, "{audit}");
    let entries = audit["entries"].as_array().unwrap();
    let actions: Vec<&str> = entries.iter().map(|entry| text(entry, "action")).collect();
    assert_eq!(
        actions,
        [
            "internal.ops.disconnectSubscriber",
            "internal.ops.purgeExpired",
            "com.atproto.admin.getSubjectStatus",
            "com.atproto.admin.updateSubjectStatus",
            "internal.ops.revokeSessions",
        ]
    );
    assert_eq!(entries[0]["outcome"], "SubscriberNotFound");
    let takedown = &entries[3];
    assert_eq!(takedown["actor"], "admin");
    assert_eq!(takedown["credential"], "admin");
    assert_eq!(takedown["reason"], "spam");
    assert_eq!(takedown["sidecar"], "console");
    assert_eq!(takedown["subject"], did.as_str());
    assert_eq!(entries[4]["subject"], did.as_str());
    assert_eq!(entries[2]["reason"], Value::Null);
    assert!(audit["before"].as_u64().unwrap() > 1);
    let all: Vec<String> = server
        .operate("internal.ops.listAudit?limit=500", None, None)
        .await
        .1["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| text(entry, "action").to_owned())
        .collect();
    assert!(!all.iter().any(|action| action.contains("noSuchAction")));
    assert!(
        !all.iter()
            .any(|action| action == "com.atproto.repo.createRecord")
    );
}

#[tokio::test]
async fn a_subscriber_is_seen_live_and_disconnected_by_the_operator() {
    use base64::Engine as _;
    use futures_util::StreamExt as _;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

    let server = Server::start(&[("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD)]).await;
    let account =
        json!({"handle": "piet.pds-users.org", "email": "piet@example.com", "password": PASSWORD});
    assert_eq!(server.create(&account).await.0, 200);

    let mut request = format!(
        "{}/xrpc/com.atproto.sync.subscribeRepos?cursor=0",
        server.origin.replace("http://", "ws://")
    )
    .into_client_request()
    .unwrap();
    request
        .headers_mut()
        .insert("user-agent", "test-relay/1".parse().unwrap());
    let mut firehose = tokio_tungstenite::connect_async(request).await.unwrap().0;
    assert!(matches!(
        firehose.next().await,
        Some(Ok(Message::Binary(_)))
    ));

    // The live stream of the ops listener shows the subscriber.
    let mut request = format!(
        "{}/xrpc/internal.ops.subscribeState",
        server.ops_origin.replace("http://", "ws://")
    )
    .into_client_request()
    .unwrap();
    let basic = base64::engine::general_purpose::STANDARD
        .encode(format!("admin:{}", common::ADMIN_PASSWORD));
    request
        .headers_mut()
        .insert("authorization", format!("Basic {basic}").parse().unwrap());
    let mut live = tokio_tungstenite::connect_async(request).await.unwrap().0;
    let Some(Ok(Message::Text(frame))) = live.next().await else {
        panic!("the live stream sent no state");
    };
    let frame: Value = serde_json::from_str(frame.as_str()).unwrap();
    assert_eq!(frame["overview"]["subscribers"], 1, "{frame}");
    let subscriber = &frame["subscribers"][0];
    assert_eq!(subscriber["userAgent"], "test-relay/1");
    assert!(subscriber["frames"].as_u64().unwrap() >= 1);
    assert!(subscriber["bytes"].as_u64().unwrap() > 0);

    // The operator disconnects it: the stream ends with a close frame.
    let id = json!({"id": subscriber["id"]});
    let (status, body) = server
        .operate(
            "internal.ops.disconnectSubscriber",
            Some("a runaway consumer"),
            Some(&id),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let closed = loop {
        match firehose.next().await {
            Some(Ok(Message::Binary(_))) => {}
            other => break other,
        }
    };
    let Some(Ok(Message::Close(Some(close)))) = closed else {
        panic!("the firehose did not close: {closed:?}");
    };
    assert_eq!(close.reason.as_str(), "disconnected by the operator");
    let (_, listed) = server
        .operate("internal.ops.listSubscribers", None, None)
        .await;
    assert_eq!(listed["subscribers"], json!([]));
}

/// The variables of a server whose shared tier is in the Postgres that
/// `PDS_TEST_POSTGRES_URL` names, in schemas of its own. `None` without
/// the variable: the test then does nothing.
fn postgres(name: &str) -> Option<(String, String)> {
    let url = std::env::var("PDS_TEST_POSTGRES_URL").ok()?;
    Some((url, format!("t{}_{name}", std::process::id())))
}

#[tokio::test]
async fn the_shared_tier_runs_on_postgres() {
    use futures_util::StreamExt as _;
    use tokio_tungstenite::tungstenite::Message;
    let Some((url, prefix)) = postgres("accounts") else {
        return;
    };
    let server = Server::start(&[
        ("PDS_POSTGRES_URL", &url),
        ("PDS_POSTGRES_SCHEMA_PREFIX", &prefix),
        ("PDS_PROFILE", "linkjar"),
        ("PDS_SERVICE_HANDLE_DOMAINS", ".linkjar.social"),
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
    ])
    .await;
    let tier = server.dir.config.layout.shared.clone().unwrap();

    // An account, with its rows in Postgres and none in the SQLite files.
    let account = json!({"handle": "petra.linkjar.social", "email": "petra@example.com", "password": PASSWORD});
    let (status, made) = server.create(&account).await;
    assert_eq!(status, 200, "{made}");
    let did = text(&made, "did").to_owned();
    let access = text(&made, "accessJwt").to_owned();
    let in_files: i64 = common::open(&server.dir.config.layout.account_db)
        .query_row(
            r#"select count(*) from "actor" where "did" = ?1"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(in_files, 0);
    assert_eq!(server.create(&account).await.0, 400);

    // Sessions.
    let login = json!({"identifier": "petra.linkjar.social", "password": PASSWORD});
    let (status, session) = server
        .post("com.atproto.server.createSession", None, &login)
        .await;
    assert_eq!(status, 200, "{session}");
    let (status, refreshed) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&session, "refreshJwt")),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200, "{refreshed}");
    let (status, _) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&session, "refreshJwt")),
            &json!({}),
        )
        .await;
    assert_ne!(status, 500);
    // A token that the server never sent is refused, not an error of the
    // server: the statement compares a parameter with NULL.
    let wrong = json!({"email": "petra@example.com", "token": "aaaaa-bbbbb"});
    let (status, refused) = server
        .post("com.atproto.server.confirmEmail", Some(&access), &wrong)
        .await;
    assert_eq!(
        (status, text(&refused, "error")),
        (400, "InvalidToken"),
        "{refused}"
    );
    let reset = json!({"token": "aaaaa-bbbbb", "password": "another-password"});
    let (status, refused) = server
        .post("com.atproto.server.resetPassword", None, &reset)
        .await;
    assert_eq!(status, 400, "{refused}");
    let app = json!({"name": "phone"});
    let (status, body) = server
        .post("com.atproto.server.createAppPassword", Some(&access), &app)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        server
            .post("com.atproto.server.createAppPassword", Some(&access), &app)
            .await
            .0,
        400
    );

    // Records and the event log.
    for n in 0..3 {
        let record = json!({"repo": did, "collection": "com.example.note", "record": {"$type": "com.example.note", "n": n}});
        let (status, body) = server
            .post("com.atproto.repo.createRecord", Some(&access), &record)
            .await;
        assert_eq!(status, 200, "{body}");
    }
    let listed = server
        .http
        .get(format!("{}/xrpc/com.atproto.sync.listRepos", server.origin))
        .send()
        .await
        .unwrap();
    let repos: Value = serde_json::from_slice(&listed.bytes().await.unwrap()).unwrap();
    assert!(
        repos["repos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|repo| repo["did"] == did.as_str()),
        "{repos}"
    );
    let mut firehose = tokio_tungstenite::connect_async(format!(
        "{}/xrpc/com.atproto.sync.subscribeRepos?cursor=0",
        server.origin.replace("http://", "ws://")
    ))
    .await
    .unwrap()
    .0;
    // The identity, the account, and four commits: the first of the
    // repository and one for each record.
    for _ in 0..6 {
        assert!(matches!(
            firehose.next().await,
            Some(Ok(Message::Binary(_)))
        ));
    }

    // The admin, the operator's state, and the audit log.
    let (status, codes) = server
        .operate(
            "com.atproto.server.createInviteCodes",
            Some("a test"),
            Some(&json!({"codeCount": 2, "useCount": 1})),
        )
        .await;
    assert_eq!(status, 200, "{codes}");
    let takedown = json!({
        "subject": {"$type": "com.atproto.admin.defs#repoRef", "did": did},
        "takedown": {"applied": true, "ref": "pg-1"},
    });
    let (status, body) = server
        .operate(
            "com.atproto.admin.updateSubjectStatus",
            Some("a test"),
            Some(&takedown),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, card) = server
        .operate(&format!("internal.ops.getAccount?did={did}"), None, None)
        .await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(card["status"], "takendown");
    assert_eq!(card["email"], "petra@example.com");
    let (status, overview) = server.operate("internal.ops.getOverview", None, None).await;
    assert_eq!(status, 200, "{overview}");
    assert_eq!(overview["accounts"], 1);
    assert!(overview["sequencer"]["head"].as_i64().unwrap() >= 6);
    let (status, audit) = server
        .operate("internal.ops.listAudit?limit=3", None, None)
        .await;
    assert_eq!(status, 200, "{audit}");
    assert_eq!(
        audit["entries"][0]["action"],
        "com.atproto.admin.updateSubjectStatus"
    );
    let (status, _) = server
        .operate("internal.ops.checkpoint", Some("a test"), Some(&json!({})))
        .await;
    assert_eq!(status, 200);

    // A second server on the tier does not get the lease of the event log.
    assert!(tier.sequencer_lease().unwrap().is_some());
    let held = tier.sequencer_lease().unwrap().unwrap();
    assert!(tier.sequencer_lease().unwrap().is_none());
    drop(held);
    tier.drop_schemas().unwrap();
}

/// What a stand-in entryway was asked: the path, the credential and the
/// body of each request.
type Asked = Arc<Mutex<Vec<(String, Option<String>, String)>>>;

/// An entryway that records each request and answers with the method it
/// was asked.
async fn entryway() -> (String, Asked) {
    let asked = Asked::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .fallback(
            |State(asked): State<Asked>, request: axum::extract::Request| async move {
                let path = request.uri().path().to_owned();
                let credential = request
                    .headers()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                let body = axum::body::to_bytes(request.into_body(), 65_536)
                    .await
                    .unwrap();
                let body = String::from_utf8_lossy(&body).into_owned();
                asked.lock().unwrap().push((path.clone(), credential, body));
                let answer = json!({"answeredBy": "entryway", "method": path}).to_string();
                ([("content-type", "application/json")], answer)
            },
        )
        .with_state(Arc::clone(&asked));
    tokio::spawn(async move { axum::serve(listener, app).await });
    (origin, asked)
}

/// A token as an entryway signs it.
fn entryway_token(key: &pds_identity::Keypair, claims: &Value) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let head = URL_SAFE_NO_PAD.encode(json!({"typ": "at+jwt", "alg": "ES256K"}).to_string());
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let signature = key.sign(format!("{head}.{body}").as_bytes());
    format!("{head}.{body}.{}", URL_SAFE_NO_PAD.encode(signature))
}

#[tokio::test]
async fn a_member_host_keeps_repositories_and_leaves_accounts_to_its_entryway() {
    use pds_identity::{Algorithm, Keypair};
    let (entryway_url, asked) = entryway().await;
    let jwt_key = Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(21));
    let rotation = Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(22));
    let jwt_public = data_encoding::HEXLOWER.encode(&jwt_key.public_key().to_compressed());
    let server = Server::start(&[
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
        ("PDS_ENTRYWAY_URL", &entryway_url),
        ("PDS_ENTRYWAY_DID", "did:web:entryway.test"),
        (
            "PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX",
            &jwt_public,
        ),
        ("PDS_ENTRYWAY_PLC_ROTATION_KEY", &rotation.did_key()),
        ("PDS_ENTRYWAY_ADMIN_TOKEN", "entryway-admin"),
        // A member host has no rotation key of its own.
        ("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX", ""),
    ])
    .await;
    let create = "com.atproto.server.createAccount";

    // The entryway makes an account: it asks the host for a signing key,
    // signs the operation that creates the DID, and hands both over.
    let (status, reserved) = server
        .post("com.atproto.server.reserveSigningKey", None, &json!({}))
        .await;
    assert_eq!(status, 200, "{reserved}");
    let signing_key = text(&reserved, "signingKey").to_owned();
    let genesis = pds_identity::plc::create_op(
        &signing_key,
        &[rotation.did_key()],
        "mia.pds-users.org",
        "https://pds.test",
        &rotation,
    );
    let account =
        json!({"did": genesis.did, "handle": "mia.pds-users.org", "plcOp": genesis.operation});

    // What the host refuses: no operation, an operation that another key
    // signed, and a handle that the operation does not name.
    let plain =
        json!({"handle": "mia.pds-users.org", "email": "mia@example.com", "password": PASSWORD});
    let (status, refused) = server.post(create, None, &plain).await;
    assert_eq!(status, 400, "{refused}");
    assert_eq!(
        text(&refused, "message"),
        "non-entryway pds requires bringing a DID and plcOp"
    );
    let stranger = Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(23));
    let forged = pds_identity::plc::create_op(
        &signing_key,
        &[rotation.did_key()],
        "mia.pds-users.org",
        "https://pds.test",
        &stranger,
    );
    let forged =
        json!({"did": forged.did, "handle": "mia.pds-users.org", "plcOp": forged.operation});
    let (status, refused) = server.post(create, None, &forged).await;
    assert_eq!(
        (status, text(&refused, "error")),
        (400, "IncompatibleDidDoc"),
        "{refused}"
    );
    let mut renamed = account.clone();
    renamed["handle"] = json!("other.pds-users.org");
    let (status, refused) = server.post(create, None, &renamed).await;
    assert_eq!(
        (status, text(&refused, "error")),
        (400, "IncompatibleDidDoc"),
        "{refused}"
    );

    let before = server.ops.lock().unwrap().len();
    let (status, made) = server.post(create, None, &account).await;
    assert_eq!(status, 200, "{made}");
    assert_eq!(text(&made, "did"), genesis.did);
    // The host opens no session, sends the operation to the directory,
    // keeps the repository, and keeps no credential.
    assert_eq!(text(&made, "accessJwt"), "");
    assert_eq!(server.ops.lock().unwrap().len(), before + 1);
    let accounts = common::open(&server.dir.config.layout.account_db);
    let count = |table: &str| -> i64 {
        accounts
            .query_row(
                &format!(r#"select count(*) from "{table}" where "did" = ?1"#),
                [&genesis.did],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        (count("actor"), count("account"), count("repo_root")),
        (1, 0, 1)
    );
    // The reserved key is the account's now: a second creation finds none.
    let (status, refused) = server.post(create, None, &account).await;
    assert_eq!(status, 400, "{refused}");

    // A token of the entryway writes to the repository. A token that the
    // host's own secret signed has ended.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = entryway_token(
        &jwt_key,
        &json!({"sub": genesis.did, "aud": "did:web:pds.test", "scope": "com.atproto.access", "exp": now + 600}),
    );
    let record = json!({"repo": genesis.did, "collection": "com.example.note", "record": {"$type": "com.example.note"}});
    let (status, written) = server
        .post("com.atproto.repo.createRecord", Some(&token), &record)
        .await;
    assert_eq!(status, 200, "{written}");
    let forged = entryway_token(
        &stranger,
        &json!({"sub": genesis.did, "aud": "did:web:pds.test", "scope": "com.atproto.access", "exp": now + 600}),
    );
    let (status, _) = server
        .post("com.atproto.repo.createRecord", Some(&forged), &record)
        .await;
    assert_eq!(status, 400);

    // Sessions and credentials go to the entryway, with the credential
    // of the client.
    let login = json!({"identifier": "mia.pds-users.org", "password": PASSWORD});
    let (status, answer) = server
        .post("com.atproto.server.createSession", None, &login)
        .await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["answeredBy"], "entryway");
    let (status, _) = server
        .post(
            "com.atproto.server.refreshSession",
            Some("a-refresh-token"),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200);
    {
        let asked = asked.lock().unwrap();
        assert_eq!(asked[0].0, "/xrpc/com.atproto.server.createSession");
        assert_eq!(serde_json::from_str::<Value>(&asked[0].2).unwrap(), login);
        assert_eq!(asked[1].1.as_deref(), Some("Bearer a-refresh-token"));
    }
    // An admin method of the entryway needs the admin of this host, and
    // goes on with the admin credential of the entryway.
    let change = json!({"did": genesis.did, "password": "a-new-password"});
    let (status, _) = server
        .post(
            "com.atproto.admin.updateAccountPassword",
            Some(&token),
            &change,
        )
        .await;
    assert_eq!(status, 401);
    let passed = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.updateAccountPassword",
            server.origin
        ))
        .basic_auth("admin", Some(common::ADMIN_PASSWORD))
        .header("content-type", "application/json")
        .body(change.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(passed.status(), 200);
    {
        use base64::Engine as _;
        let basic = base64::engine::general_purpose::STANDARD.encode("admin:entryway-admin");
        let asked = asked.lock().unwrap();
        assert_eq!(
            asked.len(),
            3,
            "the refused request did not reach the entryway"
        );
        assert_eq!(
            asked[2].1.as_deref(),
            Some(format!("Basic {basic}").as_str())
        );
    }

    // The host answers for no handle and is no authorization server.
    let get = |path: &'static str| {
        let request = server
            .http
            .get(format!("{}{path}", server.origin))
            .header("host", "mia.pds-users.org");
        async move { request.send().await.unwrap() }
    };
    assert_eq!(get("/.well-known/atproto-did").await.status(), 404);
    assert_eq!(get("/oauth/authorize").await.status(), 404);
    assert_eq!(
        get("/.well-known/oauth-authorization-server")
            .await
            .status(),
        404
    );
    let resource = get("/.well-known/oauth-protected-resource").await;
    let resource: Value = serde_json::from_slice(&resource.bytes().await.unwrap()).unwrap();
    assert_eq!(resource["authorization_servers"], json!([entryway_url]));
}

#[tokio::test]
async fn an_entryway_keeps_the_accounts_of_a_member_host() {
    use pds_identity::{Algorithm, Keypair};
    let (plc_url, ops) = plc().await;
    let member_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let entryway_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let member_origin = format!("http://{}", member_listener.local_addr().unwrap());
    let entryway_origin = format!("http://{}", entryway_listener.local_addr().unwrap());
    let rotation_hex = "0707070707070707070707070707070707070707070707070707070707070707";
    let rotation = Keypair::from_secret_bytes(
        Algorithm::Secp256k1,
        &data_encoding::HEXLOWER
            .decode(rotation_hex.as_bytes())
            .unwrap(),
    )
    .unwrap();
    let signing_hex = "0808080808080808080808080808080808080808080808080808080808080808";
    let signing = Keypair::from_secret_bytes(
        Algorithm::Secp256k1,
        &data_encoding::HEXLOWER
            .decode(signing_hex.as_bytes())
            .unwrap(),
    )
    .unwrap();
    let signing_public = data_encoding::HEXLOWER.encode(&signing.public_key().to_compressed());

    // The member host is `pds.test`. The entryway is `entryway.test`.
    let member = Server::start_on(
        member_listener,
        &plc_url,
        Arc::clone(&ops),
        &[
            ("PDS_ADMIN_PASSWORD", "member-admin"),
            ("PDS_ENTRYWAY_URL", &entryway_origin),
            ("PDS_ENTRYWAY_DID", "did:web:entryway.test"),
            (
                "PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX",
                &signing_public,
            ),
            ("PDS_ENTRYWAY_PLC_ROTATION_KEY", &rotation.did_key()),
            ("PDS_ENTRYWAY_ADMIN_TOKEN", common::ADMIN_PASSWORD),
            ("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX", ""),
        ],
    )
    .await;
    let entryway = Server::start_on(
        entryway_listener,
        &plc_url,
        Arc::clone(&ops),
        &[
            ("PDS_HOSTNAME", "entryway.test"),
            ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
            ("PDS_MEMBER_URL", "https://pds.test"),
            ("PDS_MEMBER_INTERNAL_URL", &member_origin),
            ("PDS_MEMBER_ADMIN_PASSWORD", "member-admin"),
            (
                "PDS_ENTRYWAY_JWT_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
                signing_hex,
            ),
            ("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX", rotation_hex),
        ],
    )
    .await;

    // A sign-up at the entryway: the credentials stay there, the
    // repository is on the member host, and the directory hears once.
    let before = ops.lock().unwrap().len();
    let account =
        json!({"handle": "nora.pds-users.org", "email": "nora@example.com", "password": PASSWORD});
    let (status, made) = entryway.create(&account).await;
    assert_eq!(status, 200, "{made}");
    let did = text(&made, "did").to_owned();
    let access = text(&made, "accessJwt").to_owned();
    assert_eq!(ops.lock().unwrap().len(), before + 1);
    let operation = ops.lock().unwrap().last().unwrap().1.clone();
    assert_eq!(
        operation["services"]["atproto_pds"]["endpoint"],
        "https://pds.test"
    );
    let rows = |server: &Server, table: &str| -> i64 {
        common::open(&server.dir.config.layout.account_db)
            .query_row(
                &format!(r#"select count(*) from "{table}" where "did" = ?1"#),
                [&did],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        (rows(&entryway, "account"), rows(&entryway, "repo_root")),
        (1, 0)
    );
    assert_eq!(
        (rows(&member, "account"), rows(&member, "repo_root")),
        (0, 1)
    );
    assert!(member.dir.config.layout.actor_db(&did).exists());
    assert!(!entryway.dir.config.layout.actor_db(&did).exists());

    // The token of the entryway writes at the member host, and at the
    // entryway, which passes the request on.
    let record = |n: u8| json!({"repo": did, "collection": "com.example.note", "record": {"$type": "com.example.note", "n": n}});
    let (status, body) = member
        .post("com.atproto.repo.createRecord", Some(&access), &record(1))
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = entryway
        .post("com.atproto.repo.createRecord", Some(&access), &record(2))
        .await;
    assert_eq!(status, 200, "{body}");
    let store = pds_storage::ActorDb::open(&member.dir.config.layout.actor_db(&did)).unwrap();
    assert_eq!(store.counts().unwrap().0, 2);

    // The entryway serves the firehose of the member host: it has commits,
    // which the entryway alone never writes.
    let firehose = entryway.origin.replacen("http://", "ws://", 1)
        + "/xrpc/com.atproto.sync.subscribeRepos?cursor=0";
    let (mut socket, _) = tokio_tungstenite::connect_async(firehose.as_str())
        .await
        .unwrap();
    let commit = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        use futures_util::StreamExt as _;
        while let Some(Ok(frame)) = socket.next().await {
            if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = frame
                && bytes.windows(7).any(|window| window == b"#commit")
            {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(commit, Ok(true));
    drop(socket);

    // A blob larger than a buffered request goes through the entryway and
    // back as a stream.
    let blob: Vec<u8> = (0..2 * 1024 * 1024).map(|i: u32| (i % 251) as u8).collect();
    let uploaded = entryway
        .http
        .post(format!(
            "{}/xrpc/com.atproto.repo.uploadBlob",
            entryway.origin
        ))
        .bearer_auth(&access)
        .header("content-type", "application/octet-stream")
        .body(blob.clone())
        .send()
        .await
        .unwrap();
    let (status, body) = (uploaded.status(), uploaded.text().await.unwrap());
    assert_eq!(status, 200, "{body}");
    let uploaded: Value = serde_json::from_str(&body).unwrap();
    let cid = uploaded["blob"]["ref"]["$link"]
        .as_str()
        .unwrap()
        .to_owned();
    let file = json!({"repo": did, "collection": "com.example.file", "record": {"$type": "com.example.file", "file": uploaded["blob"]}});
    let (status, body) = entryway
        .post("com.atproto.repo.createRecord", Some(&access), &file)
        .await;
    assert_eq!(status, 200, "{body}");
    let fetched = entryway
        .http
        .get(format!(
            "{}/xrpc/com.atproto.sync.getBlob?did={did}&cid={cid}",
            entryway.origin
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.bytes().await.unwrap().as_ref(), blob.as_slice());

    // A client of the member host signs in and refreshes there: the
    // entryway answers.
    let login = json!({"identifier": "nora.pds-users.org", "password": PASSWORD});
    let (status, session) = member
        .post("com.atproto.server.createSession", None, &login)
        .await;
    assert_eq!(status, 200, "{session}");
    assert_eq!(text(&session, "did"), did);
    let wrong = json!({"identifier": "nora.pds-users.org", "password": "not-the-password"});
    assert_eq!(
        member
            .post("com.atproto.server.createSession", None, &wrong)
            .await
            .0,
        401
    );
    let (status, refreshed) = member
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&session, "refreshJwt")),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200, "{refreshed}");
    let access = text(&refreshed, "accessJwt").to_owned();
    let (status, body) = member
        .post("com.atproto.repo.createRecord", Some(&access), &record(3))
        .await;
    assert_eq!(status, 200, "{body}");

    // A handle change goes to the entryway, which writes the directory
    // and tells the member host.
    let rename = json!({"handle": "nora2.pds-users.org"});
    let (status, body) = member
        .post("com.atproto.identity.updateHandle", Some(&access), &rename)
        .await;
    assert_eq!(status, 200, "{body}");
    let handle = |server: &Server| -> Option<String> {
        common::open(&server.dir.config.layout.account_db)
            .query_row(
                r#"select "handle" from "actor" where "did" = ?1"#,
                [&did],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(handle(&entryway).as_deref(), Some("nora2.pds-users.org"));
    assert_eq!(handle(&member).as_deref(), Some("nora2.pds-users.org"));
    assert_eq!(
        ops.lock().unwrap().last().unwrap().1["alsoKnownAs"],
        json!(["at://nora2.pds-users.org"])
    );

    // The authorization server of the entryway protects the member host.
    let metadata: Value = entryway
        .http
        .get(format!(
            "{}/.well-known/oauth-authorization-server",
            entryway.origin
        ))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap();
    assert_eq!(
        metadata["protected_resources"],
        json!(["https://entryway.test", "https://pds.test"])
    );

    // The admin of the member host reads the account where it is kept:
    // the entryway has the email address.
    let info = member
        .http
        .get(format!(
            "{}/xrpc/com.atproto.admin.getAccountInfo?did={did}",
            member.origin
        ))
        .basic_auth("admin", Some("member-admin"))
        .send()
        .await
        .unwrap();
    assert_eq!(info.status(), 200);
    let info: Value = serde_json::from_slice(&info.bytes().await.unwrap()).unwrap();
    assert_eq!(info["email"], "nora@example.com", "{info}");

    // A deactivation at the member host goes to the entryway, which keeps
    // the status and has the member host announce it. The activation
    // checks the identity against the rotation key of the entryway.
    let deactivated = |server: &Server| -> bool {
        common::open(&server.dir.config.layout.account_db)
            .query_row(
                r#"select "deactivatedAt" is not null from "actor" where "did" = ?1"#,
                [&did],
                |row| row.get(0),
            )
            .unwrap()
    };
    let (status, body) = member
        .post(
            "com.atproto.server.deactivateAccount",
            Some(&access),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!((deactivated(&entryway), deactivated(&member)), (true, true));
    let (status, body) = member
        .post(
            "com.atproto.server.activateAccount",
            Some(&access),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (deactivated(&entryway), deactivated(&member)),
        (false, false)
    );

    // A session from before the entryway: its refresh token, signed with
    // the secret of the old server for the old server, is taken once.
    let old = pds_xrpc::auth::AuthConfig {
        jwt_secret: common::JWT_SECRET.as_bytes().to_vec(),
        admin_password: String::new(),
        service_did: "did:web:pds.test".to_owned(),
        entryway: None,
        signer: None,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let now = i64::try_from(now).unwrap();
    let (legacy, expires) = pds_xrpc::auth::mint_refresh(&old, &did, "legacy-session", now);
    common::open(&entryway.dir.config.layout.account_db)
        .execute(
            r#"insert into "refresh_token" ("id", "did", "expiresAt") values ('legacy-session', ?1, ?2)"#,
            [&did, &pds_sequencer::iso_millis(expires * 1000)],
        )
        .unwrap();
    let old_access = pds_xrpc::auth::mint_access(&old, &did, "com.atproto.access", now);
    let (status, ended) = member
        .post(
            "com.atproto.repo.createRecord",
            Some(&old_access),
            &record(4),
        )
        .await;
    assert_eq!(
        (status, text(&ended, "error")),
        (400, "ExpiredToken"),
        "{ended}"
    );
    let (status, renewed) = member
        .post(
            "com.atproto.server.refreshSession",
            Some(&legacy),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200, "{renewed}");
    let (status, body) = member
        .post(
            "com.atproto.repo.createRecord",
            Some(text(&renewed, "accessJwt")),
            &record(4),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // The admin of the member host renames the account: the entryway
    // keeps the handle and tells the member host, which does not pass
    // that call back.
    let renamed = member
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.updateAccountHandle",
            member.origin
        ))
        .basic_auth("admin", Some("member-admin"))
        .header("content-type", "application/json")
        .body(json!({"did": did, "handle": "nora3.pds-users.org"}).to_string())
        .send()
        .await
        .unwrap();
    let (status, body) = (renamed.status(), renamed.text().await.unwrap());
    assert_eq!(status, 200, "{body}");
    assert_eq!(handle(&entryway).as_deref(), Some("nora3.pds-users.org"));
    assert_eq!(handle(&member).as_deref(), Some("nora3.pds-users.org"));

    // A takedown at the member host reaches the entryway, which refuses a
    // session of the account.
    let takedown = |applied: bool| {
        json!({
            "subject": {"$type": "com.atproto.admin.defs#repoRef", "did": did},
            "takedown": {"applied": applied, "ref": "parity"},
        })
    };
    let status = member
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.updateSubjectStatus",
            member.origin
        ))
        .basic_auth("admin", Some("member-admin"))
        .header("content-type", "application/json")
        .body(takedown(true).to_string())
        .send()
        .await
        .unwrap();
    let (status, body) = (status.status(), status.text().await.unwrap());
    assert_eq!(status, 200, "{body}");
    let login = json!({"identifier": "nora3.pds-users.org", "password": PASSWORD});
    let (status, refused) = member
        .post("com.atproto.server.createSession", None, &login)
        .await;
    assert_eq!(
        (status, text(&refused, "error")),
        (401, "AccountTakedown"),
        "{refused}"
    );

    // The admin of the entryway lifts it, and the member host learns it.
    let taken = |server: &Server| -> bool {
        common::open(&server.dir.config.layout.account_db)
            .query_row(
                r#"select "takedownRef" is not null from "actor" where "did" = ?1"#,
                [&did],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!((taken(&entryway), taken(&member)), (true, true));
    let lifted = entryway
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.updateSubjectStatus",
            entryway.origin
        ))
        .basic_auth("admin", Some(common::ADMIN_PASSWORD))
        .header("content-type", "application/json")
        .body(takedown(false).to_string())
        .send()
        .await
        .unwrap();
    let (status, body) = (lifted.status(), lifted.text().await.unwrap());
    assert_eq!(status, 200, "{body}");
    assert_eq!((taken(&entryway), taken(&member)), (false, false));
    let (status, body) = member
        .post("com.atproto.server.createSession", None, &login)
        .await;
    assert_eq!(status, 200, "{body}");

    // A sign-up that stopped after the member host created the repository:
    // `reconcile` finds it, keeps it while it is young, and removes it on
    // request after that.
    let (status, orphan) = entryway
        .create(&json!({"handle": "orphan.pds-users.org", "email": "orphan@example.com", "password": PASSWORD}))
        .await;
    assert_eq!(status, 200, "{orphan}");
    let orphan = text(&orphan, "did").to_owned();
    let entryway_db = common::open(&entryway.dir.config.layout.account_db);
    for table in ["account", "actor"] {
        entryway_db
            .execute(
                &format!(r#"delete from "{table}" where "did" = ?1"#),
                [&orphan],
            )
            .unwrap();
    }
    let now = linkjar_pds::serve::now_ms().unsigned_abs() * 1000;
    let report = linkjar_pds::reconcile::reconcile_at(&entryway.dir.config, true, now)
        .await
        .unwrap();
    assert_eq!(report.only_at_member, std::slice::from_ref(&orphan));
    assert_eq!(report.young, std::slice::from_ref(&orphan));
    assert_eq!(report.removed.len(), 0);
    assert!(member.dir.config.layout.actor_db(&orphan).exists());
    let later = now + 60 * 60 * 1_000_000;
    let report = linkjar_pds::reconcile::reconcile_at(&entryway.dir.config, true, later)
        .await
        .unwrap();
    assert_eq!(report.removed, std::slice::from_ref(&orphan));
    assert_eq!(report.only_at_entryway.len(), 0, "{report}");
    assert!(!member.dir.config.layout.actor_db(&orphan).exists());
    let report = linkjar_pds::reconcile::reconcile_at(&entryway.dir.config, false, later)
        .await
        .unwrap();
    assert!(report.is_clean(), "{report}");

    // The admin of the entryway deletes the account on both servers.
    let gone = entryway
        .http
        .post(format!(
            "{}/xrpc/com.atproto.admin.deleteAccount",
            entryway.origin
        ))
        .basic_auth("admin", Some(common::ADMIN_PASSWORD))
        .header("content-type", "application/json")
        .body(json!({"did": did}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 200);
    assert_eq!(
        (rows(&entryway, "account"), rows(&member, "repo_root")),
        (0, 0)
    );
    assert!(!member.dir.config.layout.actor_db(&did).exists());
}
