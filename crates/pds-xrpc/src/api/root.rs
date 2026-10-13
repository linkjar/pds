//! The routes outside `/xrpc/<nsid>` (`docs/SPEC.md` §4.1).

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pds_storage::Availability;

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
        .blocking(|state| Ok(state.layout().accounts()?.ping()?))
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
            let accounts = state.layout().accounts()?;
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

/// The path that a proxy in front of the server rewrites a request on a
/// handle host to (`legacy/staging/Caddyfile.handles`).
pub const PROFILE_HELPER: &str = "/.well-known/linkjar-profile";

fn host_of(headers: &HeaderMap) -> String {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(|host| host.rsplit_once(':').map_or(host, |(name, _port)| name))
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The page of the account that holds the host name as its handle: a
/// redirect that the profile builds from the DID alone. The path and the
/// query of the request have no part in it.
async fn profile_redirect(state: &AppState, host: String) -> Response {
    let found = {
        let host = host.clone();
        state
            .blocking(move |state| {
                let accounts = state.layout().accounts()?;
                Ok(accounts.actor(&host, Availability::ACTIVE)?)
            })
            .await
    };
    let target = match found {
        Ok(Some(actor)) => state.config().profile.profile_page(&host, &actor.did),
        _ => None,
    };
    let mut response = match target.and_then(|url| HeaderValue::from_str(&url).ok()) {
        Some(location) => {
            let mut response = StatusCode::FOUND.into_response();
            response.headers_mut().insert(header::LOCATION, location);
            response
        }
        None => html(StatusCode::NOT_FOUND, "User not found".to_owned()),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// What the host name of a hosted handle serves (SPEC §12.4, §13.2).
///
/// It serves the DID of the handle, and a redirect to the account's page.
/// It serves no other well-known document, no XRPC method and no page of
/// the server, so nothing of an account is readable under a name that the
/// account chose. `None` means the request is for the server itself.
pub async fn handle_host(
    state: &AppState,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
) -> Option<Response> {
    let config = state.config();
    if !config.profile.has_handle_hosts() {
        return None;
    }
    let reads = method == Method::GET || method == Method::HEAD;
    let host = host_of(headers);
    // A proxy that already sorted the request sends the rest here.
    if path == PROFILE_HELPER {
        return Some(if reads {
            profile_redirect(state, host).await
        } else {
            not_found(method, path)
        });
    }
    if host == config.hostname || !config.is_hosted_handle(&host) {
        return None;
    }
    // A proxy in front of the server may sort the requests of a handle
    // host and add the headers below itself (`Caddyfile.handles`). It
    // forwards the DID and the helper path, so those answers carry none
    // from here, and the others carry them.
    if path == pds_identity::handle::WELL_KNOWN_PATH && reads {
        return Some(atproto_did(state, headers).await);
    }
    let mut response = if path.starts_with("/.well-known/") || path.starts_with("/xrpc/") {
        text(
            StatusCode::NOT_FOUND,
            format!("Use the PDS service endpoint at {}", config.hostname),
        )
    } else if reads {
        profile_redirect(state, host).await
    } else {
        not_found(method, path)
    };
    let headers = response.headers_mut();
    headers.remove(header::SET_COOKIE);
    for (name, value) in [
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; sandbox",
        ),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    Some(response)
}

/// The routes outside `/xrpc/` that need no body: the handle hosts of the
/// profile, then the plain routes of the server (SPEC §4.1). `None` means
/// the path is not one of them.
pub async fn plain(
    state: &AppState,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
) -> Option<Response> {
    if let Some(response) = handle_host(state, method, path, headers).await {
        return Some(response);
    }
    if method != Method::GET && method != Method::HEAD {
        return None;
    }
    match path {
        "/" => Some(banner()),
        "/robots.txt" => Some(robots()),
        "/xrpc/_health" => Some(health(state).await),
        pds_identity::handle::WELL_KNOWN_PATH => Some(atproto_did(state, headers).await),
        _ => None,
    }
}
