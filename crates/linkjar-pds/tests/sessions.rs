//! Legacy sessions over HTTP (`docs/SPEC.md` §4.2).
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

mod common;

use common::{ALICE, APP_PASSWORD, DANA, DataDir, PASSWORD, TARA};
use linkjar_pds::serve::app_state;
use serde_json::{Value, json};

struct Server {
    origin: String,
    http: reqwest::Client,
    _dir: DataDir,
}

impl Server {
    async fn start() -> Self {
        let dir = DataDir::new(1);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = pds_xrpc::router(app_state(&dir.config).unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await });
        Self {
            origin,
            http: reqwest::Client::new(),
            _dir: dir,
        }
    }

    async fn post(&self, nsid: &str, token: Option<&str>, body: Option<&Value>) -> (u16, Value) {
        let mut request = self.http.post(format!("{}/xrpc/{nsid}", self.origin));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let bytes = response.bytes().await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn get(&self, nsid: &str, token: &str) -> (u16, Value) {
        let response = self
            .http
            .get(format!("{}/xrpc/{nsid}", self.origin))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = response.bytes().await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
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
    // A name in use is a client error (DD-3).
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
