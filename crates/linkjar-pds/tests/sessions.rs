//! Legacy sessions over HTTP (`docs/SPEC.md` §4.2).
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

mod common;
#[path = "sessions/random.rs"]
mod random;

use std::sync::Arc;
use std::time::Duration;

use common::{ALICE, APP_PASSWORD, DANA, DataDir, PASSWORD, TARA};
use linkjar_pds::serve::app_state;
use serde_json::{Value, json};

struct Server {
    listener: common::http::Listener,
    http: reqwest::Client,
    dir: DataDir,
    state: pds_xrpc::state::AppState,
}

impl Server {
    async fn start() -> Self {
        let dir = DataDir::new(1);
        Self::start_in(dir).await
    }

    async fn start_in(dir: DataDir) -> Self {
        let state = app_state(&dir.config).unwrap();
        Self::serve(dir, state).await
    }

    async fn gated() -> (Self, Arc<random::Random>) {
        let dir = DataDir::new(1);
        let base = app_state(&dir.config).unwrap();
        let random = Arc::new(random::Random::default());
        let source = Arc::clone(&random);
        let writes = pds_xrpc::WriteState::open(
            base.layout(),
            Arc::clone(base.writes().unwrap().sequencer()),
            pds_types::TidGenerator::new(),
            base.writes().unwrap().settings().clone(),
        )
        .unwrap()
        .with_random(move || source.next());
        let state = pds_xrpc::AppState::new(
            base.config().clone(),
            Arc::clone(base.reads()),
            None,
            base.resolver().clone(),
            base.http().clone(),
            Some(writes),
            linkjar_pds::serve::now_ms,
        );
        (Self::serve(dir, state).await, random)
    }

    async fn serve(dir: DataDir, state: pds_xrpc::AppState) -> Self {
        let listener = common::http::start(pds_xrpc::router(state.clone())).await;
        Self {
            listener,
            http: reqwest::Client::new(),
            dir,
            state,
        }
    }

    async fn post(&self, nsid: &str, token: Option<&str>, body: Option<&Value>) -> (u16, Value) {
        let response = common::http::post(&self.http, &self.listener.origin, nsid, token, body)
            .send()
            .await
            .unwrap();
        common::http::json_response(response).await
    }

    async fn get(&self, nsid: &str, token: &str) -> (u16, Value) {
        let response = self
            .http
            .get(format!("{}/xrpc/{nsid}", self.listener.origin))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        common::http::json_response(response).await
    }

    async fn login(&self, identifier: &str, password: &str) -> (u16, Value) {
        self.post(
            "com.atproto.server.createSession",
            None,
            Some(&json!({"identifier": identifier, "password": password})),
        )
        .await
    }
}

const CREATE: &str = "com.atproto.server.createSession";
const REFRESH: &str = "com.atproto.server.refreshSession";
const DELETE: &str = "com.atproto.server.deleteSession";
const GET: &str = "com.atproto.server.getSession";

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovery_prevents_in_flight_password_logins_from_resurrecting_refresh_tokens() {
    let (server, random) = Server::gated().await;
    let (_, active) = server.login(ALICE, PASSWORD).await;
    let mut paused = random.pause(3);
    let main_login = server
        .http
        .post(format!("{}/xrpc/{CREATE}", server.listener.origin))
        .header("content-type", "application/json")
        .body(json!({"identifier": ALICE, "password": PASSWORD}).to_string());
    let app_login = server
        .http
        .post(format!("{}/xrpc/{CREATE}", server.listener.origin))
        .header("content-type", "application/json")
        .body(json!({"identifier": ALICE, "password": APP_PASSWORD}).to_string());
    let create_app = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.server.createAppPassword",
            server.listener.origin
        ))
        .bearer_auth(text(&active, "accessJwt"))
        .header("content-type", "application/json")
        .body(json!({"name": "racing-app-password"}).to_string());
    let attempts = tokio::spawn(async move {
        tokio::join!(main_login.send(), app_login.send(), create_app.send())
    });
    // Each request has validated its original credential before asking for
    // the randomness of its new credential. Pause all three at that seam.
    paused.entered().await;
    server.state.writes().unwrap().with_accounts(|db| {
        db.update_password(ALICE, "!recovered-password", "recovered-epoch", None)
            .unwrap();
    });
    drop(paused);
    let (main, app, created) = tokio::time::timeout(Duration::from_secs(10), attempts)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        created.unwrap().status(),
        400,
        "a pre-recovery bearer created a new persistent app password"
    );
    assert_eq!(
        main.unwrap().status(),
        401,
        "a stale password check reopened a session"
    );
    assert_eq!(
        app.unwrap().status(),
        401,
        "a revoked app password reopened a session"
    );
    assert_eq!(
        server
            .state
            .writes()
            .unwrap()
            .with_accounts(|db| db.refresh_tokens_of(ALICE))
            .unwrap(),
        vec![]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recovery_token_is_consumed_once_even_when_two_resets_are_in_flight() {
    let (server, random) = Server::gated().await;
    assert_eq!(
        server
            .post(
                "com.atproto.server.requestPasswordReset",
                None,
                Some(&json!({"email": "alice@example.com"}))
            )
            .await
            .0,
        200
    );
    let token: String = common::open(&server.dir.root.join("account.sqlite")).query_row(
        r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'reset_password'"#,
        [ALICE], |row| row.get(0),
    ).unwrap();
    let mut paused = random.pause(2);
    let reset = |password| {
        server
            .http
            .post(format!(
                "{}/xrpc/com.atproto.server.resetPassword",
                server.listener.origin
            ))
            .header("content-type", "application/json")
            .body(json!({"token": token, "password": password}).to_string())
    };
    let (first, second) = (
        reset("first-recovery-password"),
        reset("second-recovery-password"),
    );
    let attempts = tokio::spawn(async move { tokio::join!(first.send(), second.send()) });
    // Salt generation follows token validation. Both reset requests must
    // reach that boundary before either can change the password.
    paused.entered().await;
    drop(paused);
    let (first, second) = tokio::time::timeout(Duration::from_secs(10), attempts)
        .await
        .unwrap()
        .unwrap();
    let mut statuses = [
        first.unwrap().status().as_u16(),
        second.unwrap().status().as_u16(),
    ];
    statuses.sort_unstable();
    assert_eq!(
        statuses,
        [200, 400],
        "a one-time recovery token changed the password twice"
    );
}

#[tokio::test]
async fn password_recovery_revokes_legacy_access_across_restart_and_accepts_new_login() {
    let server = Server::start().await;
    // A token minted before this extension has no credential version.
    let unversioned = pds_xrpc::auth::mint_access(
        &server.state.config().auth,
        ALICE,
        pds_xrpc::auth::SCOPE_ACCESS,
        server.state.now_s(),
    );
    assert_eq!(server.get(GET, &unversioned).await.0, 200);
    let (_, before) = server.login(ALICE, PASSWORD).await;
    let old = text(&before, "accessJwt").to_owned();
    assert_eq!(server.get(GET, &old).await.0, 200);
    assert_eq!(
        server
            .post(
                "com.atproto.server.requestPasswordReset",
                None,
                Some(&json!({"email": "alice@example.com"}))
            )
            .await
            .0,
        200
    );
    let reset: String = common::open(&server.dir.root.join("account.sqlite"))
        .query_row(r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'reset_password'"#, [ALICE], |row| row.get(0)).unwrap();
    assert_eq!(
        server
            .post(
                "com.atproto.server.resetPassword",
                None,
                Some(&json!({"token": reset, "password": "new-recovery-password"}))
            )
            .await
            .0,
        200
    );
    let denied = server.get(GET, &old).await;
    assert_eq!(
        denied.1["error"], "ExpiredToken",
        "pre-recovery bearer token still grants access"
    );
    assert_eq!(
        server.get(GET, &unversioned).await.1["error"],
        "ExpiredToken"
    );
    let (_, after) = server.login(ALICE, "new-recovery-password").await;
    let current = text(&after, "accessJwt").to_owned();
    assert_eq!(server.get(GET, &current).await.0, 200);
    let Server { dir, .. } = server;
    let restarted = Server::start_in(dir).await;
    assert_eq!(restarted.get(GET, &old).await.1["error"], "ExpiredToken");
    assert_eq!(
        restarted.get(GET, &unversioned).await.1["error"],
        "ExpiredToken"
    );
    assert_eq!(restarted.get(GET, &current).await.0, 200);
    let (_, refreshed) = restarted
        .post(REFRESH, Some(text(&after, "refreshJwt")), None)
        .await;
    assert_eq!(
        restarted.get(GET, text(&refreshed, "accessJwt")).await.0,
        200
    );
    // The stored recovery version also survives a signing-secret change.
    let Server { mut dir, .. } = restarted;
    dir.config.jwt_secret = Some("rotated-test-signing-secret".to_owned());
    let rotated = Server::start_in(dir).await;
    assert_eq!(rotated.get(GET, &current).await.0, 400);
    let (status, signed_in) = rotated.login(ALICE, "new-recovery-password").await;
    assert_eq!(status, 200);
    assert_eq!(
        rotated.get(GET, text(&signed_in, "accessJwt")).await.0,
        200,
        "signing-key rotation stranded a recovered account"
    );
    let (_, refreshed) = rotated
        .post(REFRESH, Some(text(&signed_in, "refreshJwt")), None)
        .await;
    assert_eq!(rotated.get(GET, text(&refreshed, "accessJwt")).await.0, 200);
}

#[tokio::test]
async fn a_session_is_opened_read_refreshed_and_closed() {
    let server = Server::start().await;
    let started = std::time::Instant::now();
    let (status, session) = server.login("alice.pds.test", PASSWORD).await;
    assert_eq!(status, 200, "{session}");
    assert!(started.elapsed() >= std::time::Duration::from_millis(350));
    assert_eq!(
        session.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "accessJwt",
            "refreshJwt",
            "did",
            "handle",
            "email",
            "emailConfirmed",
            "active"
        ]
    );
    assert_eq!(session["did"], ALICE);
    assert_eq!(session["handle"], "alice.pds.test");
    assert_eq!(session["email"], "alice@example.com");
    assert_eq!(session["emailConfirmed"], true);
    assert_eq!(session["active"], true);
    let (access, refresh) = (text(&session, "accessJwt"), text(&session, "refreshJwt"));

    // The access token is a session for the reads and the writes.
    let (status, own) = server.get(GET, access).await;
    assert_eq!(status, 200);
    assert_eq!(
        own,
        json!({"did": ALICE, "handle": "alice.pds.test", "email": "alice@example.com",
               "emailConfirmed": true, "active": true})
    );
    // Each token has its own use.
    assert_eq!(server.get(GET, refresh).await.1["error"], "InvalidToken");
    assert_eq!(
        server.post(REFRESH, Some(access), None).await.1["error"],
        "InvalidToken"
    );
    assert_eq!(
        server.post(REFRESH, None, None).await.1["error"],
        "AuthMissing"
    );

    let (status, next) = server.post(REFRESH, Some(refresh), None).await;
    assert_eq!(status, 200, "{next}");
    assert_eq!(next["did"], ALICE);
    assert_ne!(text(&next, "refreshJwt"), refresh);
    // A second use within the grace period gives the same successor.
    let (status, again) = server.post(REFRESH, Some(refresh), None).await;
    assert_eq!(status, 200);
    let id = |token: &str| -> Value {
        use base64::Engine as _;
        let body = token.split('.').nth(1).unwrap();
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(body)
            .unwrap();
        serde_json::from_slice::<Value>(&bytes).unwrap()["jti"].clone()
    };
    assert_eq!(
        id(text(&again, "refreshJwt")),
        id(text(&next, "refreshJwt"))
    );
    assert_ne!(id(text(&next, "refreshJwt")), id(refresh));

    // Closing the session revokes its refresh token and no other.
    let successor = text(&next, "refreshJwt");
    assert_eq!(server.post(DELETE, Some(successor), None).await.0, 200);
    let (status, revoked) = server.post(REFRESH, Some(successor), None).await;
    assert_eq!((status, text(&revoked, "error")), (400, "ExpiredToken"));
    assert_eq!(revoked["message"], "Token has been revoked");
    assert_eq!(server.post(DELETE, Some(successor), None).await.0, 200);
}

#[tokio::test]
async fn a_session_is_refused_as_the_reference_refuses_it() {
    let server = Server::start().await;
    let refused =
        json!({"error": "AuthenticationRequired", "message": "Invalid identifier or password"});
    for (identifier, password) in [
        ("alice.pds.test", "wrong"),
        ("nobody.pds.test", PASSWORD),
        ("nobody@example.com", PASSWORD),
        ("not an identifier", PASSWORD),
    ] {
        let (status, body) = server.login(identifier, password).await;
        assert_eq!((status, &body), (401, &refused), "{identifier}");
    }
    let (status, body) = server.login("alice.pds.test", &"x".repeat(513)).await;
    assert_eq!(status, 401);
    assert_eq!(
        body["message"],
        "Password too long. Consider resetting your password."
    );
    let (status, body) = server
        .post(CREATE, None, Some(&json!({"identifier": "a"})))
        .await;
    assert_eq!((status, text(&body, "error")), (400, "InvalidRequest"));

    // The identifier is a handle, a DID or an email address, in any case.
    for identifier in ["ALICE.pds.test", ALICE, "Alice@Example.com"] {
        let (status, body) = server.login(identifier, PASSWORD).await;
        assert_eq!((status, &body["did"]), (200, &json!(ALICE)), "{identifier}");
    }

    // An app password opens a session of its own scope.
    let (status, session) = server.login(ALICE, APP_PASSWORD).await;
    assert_eq!(status, 200, "{session}");
    let (status, next) = server
        .post(REFRESH, Some(text(&session, "refreshJwt")), None)
        .await;
    assert_eq!(status, 200, "{next}");
    for token in [text(&session, "accessJwt"), text(&next, "accessJwt")] {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(token.split('.').nth(1).unwrap())
            .unwrap();
        let claims: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(claims["scope"], "com.atproto.appPass");
        assert_eq!(claims["aud"], "did:web:pds.test");
        assert_eq!(
            claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
            7200
        );
    }

    // A deactivated account opens a session and reads it.
    let (status, session) = server.login(DANA, PASSWORD).await;
    assert_eq!(status, 200);
    assert_eq!(
        (&session["active"], &session["status"]),
        (&json!(false), &json!("deactivated"))
    );
    assert_eq!(server.get(GET, text(&session, "accessJwt")).await.0, 200);

    // An account that is taken down opens one only when it asks for it,
    // and that session cannot be refreshed.
    let (status, body) = server.login(TARA, PASSWORD).await;
    assert_eq!((status, text(&body, "error")), (401, "AccountTakedown"));
    let (status, session) = server
        .post(
            CREATE,
            None,
            Some(&json!({"identifier": TARA, "password": PASSWORD, "allowTakendown": true})),
        )
        .await;
    assert_eq!(status, 200, "{session}");
    assert_eq!(session["status"], "takendown");
    let (status, body) = server
        .post(REFRESH, Some(text(&session, "refreshJwt")), None)
        .await;
    assert_eq!((status, text(&body, "error")), (401, "AccountTakedown"));
}

#[tokio::test]
async fn app_passwords_are_created_listed_and_revoked() {
    let server = Server::start().await;
    let (_, session) = server.login(ALICE, PASSWORD).await;
    let access = text(&session, "accessJwt");
    let create = "com.atproto.server.createAppPassword";
    let (status, made) = server
        .post(create, Some(access), Some(&json!({"name": "reader"})))
        .await;
    assert_eq!(status, 200, "{made}");
    assert_eq!(
        made.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["name", "password", "createdAt", "privileged"]
    );
    let given = text(&made, "password").to_owned();
    assert_eq!(given.len(), 19);
    assert!(given.split('-').all(|part| part.len() == 4));
    // A name in use is a client error, a deliberate difference (SPEC §2.3).
    let (status, body) = server
        .post(create, Some(access), Some(&json!({"name": "reader"})))
        .await;
    assert_eq!((status, text(&body, "error")), (400, "InvalidRequest"));
    let (_, privileged) = server
        .post(
            create,
            Some(access),
            Some(&json!({"name": "chat", "privileged": true})),
        )
        .await;
    assert_eq!(privileged["privileged"], true);

    let (status, app) = server.login(ALICE, &given).await;
    assert_eq!(status, 200, "{app}");
    // An app-password session does not manage app passwords.
    let (status, body) = server
        .post(
            create,
            Some(text(&app, "accessJwt")),
            Some(&json!({"name": "nested"})),
        )
        .await;
    assert_eq!((status, text(&body, "error")), (400, "InvalidToken"));

    let (_, listed) = server
        .get("com.atproto.server.listAppPasswords", access)
        .await;
    let names: Vec<&str> = listed["passwords"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| text(entry, "name"))
        .collect();
    assert_eq!(names.len(), 3);
    assert!(names.contains(&"reader") && names.contains(&"chat") && names.contains(&"phone"));

    let revoke = "com.atproto.server.revokeAppPassword";
    assert_eq!(
        server
            .post(revoke, Some(access), Some(&json!({"name": "reader"})))
            .await
            .0,
        200
    );
    assert_eq!(server.login(ALICE, &given).await.0, 401);
    let (status, body) = server
        .post(REFRESH, Some(text(&app, "refreshJwt")), None)
        .await;
    assert_eq!((status, text(&body, "error")), (400, "ExpiredToken"));
}
