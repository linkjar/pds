//! The HTTP and XRPC surface (`docs/SPEC.md` §4).
//!
//! The crate holds the dispatcher and the handlers of §4, the credentials
//! of §5, the write path of §6.3, the proxy of §4.4, read-after-write, the
//! HTTP side of OAuth (§11), mail, metrics, the audit-log writer (§20) and
//! the ops listener (§22.1). The rules it applies live in the crates it
//! depends on; `README.md` maps them. A method without a handler that the
//! proxy does not forward answers as at the pin: 401 without a credential,
//! 400 `InvalidToken` with the admin password, 501 with a session.

pub mod api;
pub mod audit;
pub mod auth;
pub mod body;
pub mod error;
pub mod mail;
pub mod metrics;
pub mod oauth;
pub mod ops;
pub mod params;
pub mod password;
pub mod ratelimit;
pub mod resources;
pub mod service_auth;
pub mod state;
pub mod stream;
pub mod subscribers;
mod table;
pub mod write;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};

pub use error::XrpcError;
pub use pds_ext as ext;
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
        .layer(middleware::from_fn_with_state(state.clone(), measure))
        .with_state(state)
}

/// Counts and times every request for the metrics (SPEC §14.4).
async fn measure(
    axum::extract::State(state): axum::extract::State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let route = metrics::route_label(request.uri().path());
    let started = std::time::Instant::now();
    let response = next.run(request).await;
    state
        .metrics()
        .observe(&route, response.status().as_u16(), started.elapsed());
    response
}

/// The routes of the authorization server that a browser calls from
/// another origin. The Reference's OAuth router answers their preflight
/// itself.
const OAUTH_CORS_ROUTES: [&str; 5] = [
    "/.well-known/oauth-authorization-server",
    "/oauth/jwks",
    "/oauth/par",
    "/oauth/token",
    "/oauth/revoke",
];

/// CORS as the Reference's middleware sets it (SPEC §4.2, §11.1):
/// every response allows any origin, and a preflight answers 204. On the
/// routes of [`OAUTH_CORS_ROUTES`] a preflight answers 200, and every
/// answer names the methods and the headers.
async fn cors(request: Request, next: Next) -> Response {
    let preflight = request.method() == Method::OPTIONS;
    let oauth = OAUTH_CORS_ROUTES.contains(&request.uri().path());
    let requested = request
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .cloned();
    let mut response = if preflight && oauth {
        StatusCode::OK.into_response()
    } else if preflight {
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
    let headers = response.headers_mut();
    if oauth {
        for (name, value) in [
            (header::ACCESS_CONTROL_MAX_AGE, "86400"),
            (header::ACCESS_CONTROL_ALLOW_METHODS, "*"),
            (header::ACCESS_CONTROL_ALLOW_HEADERS, "Content-Type,DPoP"),
        ] {
            headers.insert(name, HeaderValue::from_static(value));
        }
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}
