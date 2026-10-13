//! The HTTP and XRPC surface (`docs/SPEC.md` §4).
//!
//! As of unit 3 the crate serves the read half: the repository and sync reads,
//! `describeServer`, `resolveHandle` and the plain routes of §4.1. A method
//! without a handler here answers as at the pin when the request has no
//! session: 401 without a credential, 400 `InvalidToken` with the admin
//! password. With a valid session it answers 501 until the proxy of §4.4
//! arrives (`docs/plan.md`).

pub mod api;
pub mod auth;
pub mod body;
pub mod error;
pub mod mail;
pub mod params;
pub mod password;
pub mod ratelimit;
pub mod service_auth;
pub mod state;
pub mod stream;
pub mod write;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};

pub use error::XrpcError;
pub use state::{AppState, ServerConfig, WriteSettings, WriteState};

/// The server's routes over `state`.
///
/// Every request goes through one entry point, because the Reference
/// answers a wrong method on a plain route with its "not found" page and a
/// malformed `/xrpc/` path with an XRPC error, which a route table does not
/// express.
pub fn router(state: AppState) -> Router {
    Router::new()
        .fallback(api::entry)
        .layer(middleware::from_fn(cors))
        .with_state(state)
}

/// CORS as the Reference's middleware sets it (SPEC §4.2, VP-11): every
/// response allows any origin, and a preflight answers 204.
async fn cors(request: Request, next: Next) -> Response {
    let preflight = request.method() == Method::OPTIONS;
    let requested = request
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .cloned();
    let mut response = if preflight {
        let mut response = StatusCode::NO_CONTENT.into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET,HEAD,PUT,PATCH,POST,DELETE"),
        );
        if let Some(requested) = requested {
            headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, requested);
        }
        headers.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("86400"),
        );
        headers.insert(
            header::VARY,
            HeaderValue::from_static("Access-Control-Request-Headers"),
        );
        response
    } else {
        next.run(request).await
    };
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}
