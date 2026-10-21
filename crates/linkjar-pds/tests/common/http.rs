//! Shared HTTP transport for integration tests; protocol fixtures stay in their suites.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test diagnostics

use axum::Router;
use reqwest::{Client, RequestBuilder, Response};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Owns a listener task so dropping a fixture stops its server.
pub struct Listener {
    pub origin: String,
    task: JoinHandle<()>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn bind() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}

pub fn serve(listener: TcpListener, router: Router) -> Listener {
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Listener { origin, task }
}

pub async fn start(router: Router) -> Listener {
    serve(bind().await, router)
}

/// Empty responses have no JSON value. Invalid JSON must fail the test.
pub fn json(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(bytes).unwrap_or_else(|error| {
        panic!(
            "invalid JSON response: {error}\nbody: {}",
            String::from_utf8_lossy(bytes)
        )
    })
}

pub async fn json_response(response: Response) -> (u16, Value) {
    let status = response.status().as_u16();
    let bytes = response.bytes().await.unwrap();
    (status, json(&bytes))
}

pub fn post(
    client: &Client,
    origin: &str,
    nsid: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> RequestBuilder {
    let mut request = client.post(format!("{origin}/xrpc/{nsid}"));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_string());
    }
    request
}
