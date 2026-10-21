//! Email changes that overlap account recovery, through the HTTP router.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::middleware::Next;
use common::{ALICE, DataDir, PASSWORD};
use futures_util::StreamExt as _;
use linkjar_pds::serve::app_state;
use serde_json::{Value, json};
use tokio::sync::Notify;

const UPDATE: &str = "com.atproto.server.updateEmail";
const NEW_PASSWORD: &str = "recovered-email-password";

#[derive(Default)]
struct BodyGate {
    armed: AtomicBool,
    started: Notify,
    resume: Notify,
}

impl BodyGate {
    async fn intercept(self: Arc<Self>, request: Request, next: Next) -> axum::response::Response {
        if !request.uri().path().ends_with(UPDATE) || !self.armed.swap(false, Ordering::SeqCst) {
            return next.run(request).await;
        }
        let (parts, body) = request.into_parts();
        // The handler polls the body only after its initial session check.
        // Keep this request there until another HTTP request finishes recovery.
        let barrier = futures_util::stream::once(async move {
            self.started.notify_one();
            self.resume.notified().await;
            Ok::<_, axum::Error>(Bytes::new())
        });
        let body = Body::from_stream(barrier.chain(body.into_data_stream()));
        next.run(Request::from_parts(parts, body)).await
    }
}

struct Server {
    _listener: common::http::Listener,
    origin: String,
    http: reqwest::Client,
    dir: DataDir,
    gate: Arc<BodyGate>,
}

impl Server {
    async fn start() -> Self {
        let dir = DataDir::new(1);
        common::open(&dir.root.join("account.sqlite"))
            .execute(
                r#"update "account" set "emailConfirmedAt" = null where "did" = ?1"#,
                [ALICE],
            )
            .unwrap();
        let state = app_state(&dir.config).unwrap();
        let gate = Arc::new(BodyGate::default());
        let middleware_gate = Arc::clone(&gate);
        let router =
            pds_xrpc::router(state).layer(axum::middleware::from_fn(move |request, next| {
                Arc::clone(&middleware_gate).intercept(request, next)
            }));
        let listener = common::http::bind().await;
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let listener = common::http::serve(listener, router);
        Self {
            _listener: listener,
            origin,
            http: reqwest::Client::new(),
            dir,
            gate,
        }
    }

    async fn post(&self, nsid: &str, input: Value) -> reqwest::Response {
        self.http
            .post(format!("{}/xrpc/{nsid}", self.origin))
            .header("content-type", "application/json")
            .body(input.to_string())
            .send()
            .await
            .unwrap()
    }

    async fn login(&self, password: &str) -> String {
        let response = self
            .post(
                "com.atproto.server.createSession",
                json!({"identifier": ALICE, "password": password}),
            )
            .await;
        assert_eq!(response.status(), 200);
        let session: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
        session["accessJwt"].as_str().unwrap().to_owned()
    }
}

#[tokio::test]
async fn recovery_prevents_an_in_flight_email_change_from_replacing_the_recovery_address() {
    let server = Server::start().await;
    let access = server.login(PASSWORD).await;
    let requested = server
        .post(
            "com.atproto.server.requestPasswordReset",
            json!({"email": "alice@example.com"}),
        )
        .await;
    assert_eq!(requested.status(), 200);
    // Read the fixture's delivered token without an external mail service.
    let reset: String = common::open(&server.dir.root.join("account.sqlite"))
        .query_row(
            r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'reset_password'"#,
            [ALICE],
            |row| row.get(0),
        )
        .unwrap();
    server.gate.armed.store(true, Ordering::SeqCst);
    let update = server
        .http
        .post(format!("{}/xrpc/{UPDATE}", server.origin))
        .bearer_auth(access)
        .header("content-type", "application/json")
        .body(json!({"email": "attacker@example.com"}).to_string());
    let attempt = tokio::spawn(async move { update.send().await.unwrap() });
    tokio::time::timeout(Duration::from_secs(5), server.gate.started.notified())
        .await
        .expect("updateEmail did not reach its request-body read");
    let recovered = server
        .post(
            "com.atproto.server.resetPassword",
            json!({"token": reset, "password": NEW_PASSWORD}),
        )
        .await;
    assert_eq!(recovered.status(), 200);
    server.gate.resume.notify_one();
    let changed = attempt.await.unwrap();
    assert_eq!(
        changed.status(),
        400,
        "a pre-recovery request replaced the recovery address"
    );
    let error: Value = serde_json::from_slice(&changed.bytes().await.unwrap()).unwrap();
    assert_eq!(error["error"], "ExpiredToken");
    let current = server.login(NEW_PASSWORD).await;
    let session = server
        .http
        .get(format!(
            "{}/xrpc/com.atproto.server.getSession",
            server.origin
        ))
        .bearer_auth(&current)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let session: Value = serde_json::from_slice(&session).unwrap();
    assert_eq!(session["email"], "alice@example.com");
    let changed = server
        .http
        .post(format!("{}/xrpc/{UPDATE}", server.origin))
        .bearer_auth(current)
        .header("content-type", "application/json")
        .body(json!({"email": "new-alice@example.com"}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(
        changed.status(),
        200,
        "a current session can change its email"
    );
}
