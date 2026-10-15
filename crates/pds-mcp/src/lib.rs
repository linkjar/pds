//! The MCP server (`docs/sidecars.md` §2, `docs/SPEC.md` §22.2): a
//! sidecar that lets an AI client reach the repository of a person.
//!
//! It has two faces. For an AI client it is an MCP server and an OAuth
//! 2.1 authorization server with bearer tokens. For the PDS it is a
//! confidential OAuth client with DPoP. It holds no database connection
//! and no secret of the PDS beyond its own client key (SPEC §22.3).

pub mod auth;
pub mod clients;
pub mod config;
pub mod pds;
pub mod rpc;
pub mod store;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use pds_sidecar::client::ClientError;
use serde_json::Value;

use crate::config::{CLIENT_METADATA_PATH, Config, MCP_PATH, PDS_CALLBACK_PATH};
use crate::pds::Pds;
use crate::store::Store;

pub use pds_oauth::pages::escape;

/// The server: its connection to the PDS and what it keeps.
#[derive(Debug, Clone)]
pub struct Mcp {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    pds: Pds,
    store: Store,
    http: reqwest::Client,
}

impl Mcp {
    /// A server for the configuration.
    pub fn new(config: Config) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        Ok(Self {
            inner: Arc::new(Inner {
                pds: Pds::new(config)?,
                store: Store::default(),
                http,
            }),
        })
    }

    /// The configuration.
    pub fn config(&self) -> &Config {
        self.inner.pds.config()
    }

    /// The connection to the PDS.
    pub fn pds(&self) -> &Pds {
        &self.inner.pds
    }

    /// The tables of the server.
    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    /// The HTTP client for the metadata documents of AI clients.
    pub fn http(&self) -> &reqwest::Client {
        &self.inner.http
    }
}

/// A JSON answer that no cache keeps.
fn json_response(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body.to_string(),
    )
        .into_response()
}

/// A 303 to `to` that no cache keeps. An address that is not a header
/// value sends the browser to `/`.
fn redirect(to: &str) -> Response {
    let location = HeaderValue::from_str(to).unwrap_or_else(|_| HeaderValue::from_static("/"));
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, location),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
    )
        .into_response()
}

/// The fields of a form of at most `limit` bytes. A larger body gives no
/// fields.
async fn form(body: Body, limit: usize) -> Vec<(String, String)> {
    match axum::body::to_bytes(body, limit).await {
        Ok(bytes) => pds_sidecar::request::fields(&bytes),
        Err(_) => Vec::new(),
    }
}

/// The answer to a CORS preflight of a browser-based AI client. No
/// endpoint takes a cookie from another origin, so every origin may ask.
fn preflight() -> Response {
    (
        StatusCode::NO_CONTENT,
        [
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            (header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS"),
            (
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                "authorization, content-type, mcp-protocol-version, mcp-session-id",
            ),
            (header::ACCESS_CONTROL_MAX_AGE, "600"),
        ],
    )
        .into_response()
}

/// Adds the CORS header to the answers of the endpoints that an AI client
/// in a browser calls.
async fn cors(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if request.method() == Method::OPTIONS {
        return preflight();
    }
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        axum::http::HeaderValue::from_static("*"),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        axum::http::HeaderValue::from_static("www-authenticate"),
    );
    response
}

/// The routes of the server.
pub fn router(mcp: Mcp) -> axum::Router {
    let api = axum::Router::new()
        .route(MCP_PATH, post(rpc::post).get(rpc::get))
        .route(
            "/.well-known/oauth-protected-resource",
            get(auth::protected_resource),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(auth::protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(auth::authorization_server),
        )
        .route("/register", post(auth::register))
        .route("/token", post(auth::token))
        .route("/revoke", post(auth::revoke))
        .layer(axum::middleware::from_fn(cors));
    axum::Router::new()
        .route("/authorize", get(auth::authorize))
        .route("/authorize/decision", post(auth::decision))
        .route(PDS_CALLBACK_PATH, get(auth::pds_callback))
        .route(CLIENT_METADATA_PATH, get(auth::client_metadata))
        .route("/health", get(|| async { "ok" }))
        .route(
            "/mcp.css",
            get(|| async {
                (
                    [
                        (header::CONTENT_TYPE, "text/css; charset=utf-8"),
                        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                    ],
                    include_str!("mcp.css"),
                )
            }),
        )
        .merge(api)
        .with_state(mcp)
}
