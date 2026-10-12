//! The routes outside `/xrpc/<nsid>` (`docs/SPEC.md` §4.1).

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pds_storage::{AccountDb, Availability};

use crate::error::json_response;
use crate::state::AppState;

const TEXT: &str = "text/plain; charset=utf-8";

/// The Reference's banner, byte for byte.
const BANNER: &str = r"
         __                         __
        /\ \__                     /\ \__
    __  \ \ ,_\  _____   _ __   ___\ \ ,_\   ___
  /'__'\ \ \ \/ /\ '__'\/\''__\/ __'\ \ \/  / __'\
 /\ \L\.\_\ \ \_\ \ \L\ \ \ \//\ \L\ \ \ \_/\ \L\ \
 \ \__/.\_\\ \__\\ \ ,__/\ \_\\ \____/\ \__\ \____/
  \/__/\/_/ \/__/ \ \ \/  \/_/ \/___/  \/__/\/___/
                   \ \_\
                    \/_/


This is an AT Protocol Personal Data Server (aka, an atproto PDS)

Most API routes are under /xrpc/

      Code: https://github.com/bluesky-social/atproto
 Self-Host: https://github.com/bluesky-social/pds
  Protocol: https://atproto.com
";

fn text(status: StatusCode, body: impl Into<String>) -> Response {
    let mut response = (status, body.into()).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(TEXT));
    response
}

/// `GET /`.
pub fn banner() -> Response {
    text(StatusCode::OK, BANNER)
}

/// `GET /robots.txt`.
pub fn robots() -> Response {
    text(
        StatusCode::OK,
        "# Hello!\n\n# Crawling the public API is allowed\nUser-agent: *\nAllow: /",
    )
}

/// `GET /xrpc/_health`: the version, and 503 when the account database
/// does not answer.
pub async fn health(state: &AppState) -> Response {
    let version = state.config().version.clone();
    let reachable = state
        .blocking(|state| Ok(AccountDb::open(&state.layout().account_db)?.ping()?))
        .await;
    let mut body = serde_json::Map::new();
    if let Some(version) = version {
        body.insert("version".to_owned(), version.into());
    }
    if reachable.is_ok() {
        json_response(StatusCode::OK, &body.into())
    } else {
        body.insert("error".to_owned(), "Service Unavailable".into());
        json_response(StatusCode::SERVICE_UNAVAILABLE, &body.into())
    }
}

/// `GET /.well-known/atproto-did`: the DID of the hosted account whose
/// handle is the request's host.
pub async fn atproto_did(state: &AppState, headers: &HeaderMap) -> Response {
    // Express sends a string without a type as HTML.
    let not_found = || html(StatusCode::NOT_FOUND, "User not found".to_owned());
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(|host| {
            host.rsplit_once(':')
                .map_or(host, |(name, _port)| name)
                .to_owned()
        })
        .unwrap_or_default();
    if !state.config().is_hosted_handle(&host) {
        return not_found();
    }
    let found = state
        .blocking(move |state| {
            let accounts = AccountDb::open(&state.layout().account_db)?;
            Ok(accounts.actor(&host, Availability::ACTIVE)?)
        })
        .await;
    match found {
        Ok(Some(actor)) => text(StatusCode::OK, actor.did),
        Ok(None) => not_found(),
        Err(_) => html(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error".to_owned(),
        ),
    }
}

fn html(status: StatusCode, body: String) -> Response {
    let mut response = (status, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

/// Any other path, and a plain route with another method: the page Express
/// answers with.
pub fn not_found(method: &Method, path: &str) -> Response {
    let body = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>Cannot {method} {}</pre>\n</body>\n</html>\n",
        escape(path)
    );
    let mut response = html(StatusCode::NOT_FOUND, body);
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
