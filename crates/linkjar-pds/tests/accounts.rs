//! Account creation over HTTP (`docs/SPEC.md` §10.1).
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

mod common;

use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use common::{DataDir, PASSWORD};
use linkjar_pds::serve::app_state;
use serde_json::{Value, json};

type Ops = Arc<Mutex<Vec<(String, Value)>>>;

#[derive(Default)]
struct LastOperationGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

/// A PLC directory that keeps what it is sent and resolves nothing.
async fn plc() -> (String, Ops) {
    plc_with_last_gate(None).await
}

async fn plc_with_last_gate(gate: Option<Arc<LastOperationGate>>) -> (String, Ops) {
    let ops = Ops::default();
    let listener = common::http::bind().await;
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
            axum::routing::get(move |State(ops): State<Ops>, Path(did): Path<String>| {
                let gate = gate.clone();
                async move {
                    if let Some(gate) = gate {
                        gate.entered.notify_one();
                        gate.release.notified().await;
                    }
                    let ops = ops.lock().unwrap();
                    match ops.iter().rev().find(|(sent, _)| *sent == did) {
                        Some((_, op)) => op.to_string().into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            }),
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
    _listener: common::http::Listener,
    _ops_listener: common::http::Listener,
    origin: String,
    ops_origin: String,
    http: reqwest::Client,
    dir: DataDir,
    ops: Ops,
    state: pds_xrpc::AppState,
}

impl Server {
    async fn start(extra: &[(&str, &str)]) -> Self {
        let listener = common::http::bind().await;
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
        let public_listener = common::http::serve(listener, router);
        // The ops listener, on an address of its own (SPEC §22.1).
        let listener = common::http::bind().await;
        let ops_origin = format!("http://{}", listener.local_addr().unwrap());
        let router = linkjar_pds::serve::ops_router(state.clone(), &dir.config);
        let listener = common::http::serve(listener, router);
        Self {
            _listener: public_listener,
            _ops_listener: listener,
            origin,
            ops_origin,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            dir,
            ops,
            state,
        }
    }

    async fn post(&self, nsid: &str, token: Option<&str>, body: &Value) -> (u16, Value) {
        let response = common::http::post(&self.http, &self.origin, nsid, token, Some(body))
            .send()
            .await
            .unwrap();
        common::http::json_response(response).await
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
        common::http::json_response(response).await
    }

    async fn create(&self, body: &Value) -> (u16, Value) {
        self.post("com.atproto.server.createAccount", None, body)
            .await
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

#[path = "accounts/admin.rs"]
mod admin;
#[path = "accounts/audit.rs"]
mod audit;
#[path = "accounts/creation.rs"]
mod creation;
#[path = "accounts/entryway.rs"]
mod entryway;
#[path = "accounts/identity.rs"]
mod identity;
#[path = "accounts/lifecycle.rs"]
mod lifecycle;
#[path = "accounts/migration.rs"]
mod migration;
#[path = "accounts/operations.rs"]
mod operations;
#[path = "accounts/postgres.rs"]
mod postgres;
#[path = "accounts/profile.rs"]
mod profile;
#[path = "accounts/recovery.rs"]
mod recovery;
