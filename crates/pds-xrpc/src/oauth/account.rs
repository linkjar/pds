//! The account pages (`docs/SPEC.md` §11.3): the applications that have a
//! session of the account, the devices that remember it, and the buttons
//! that end each.
//!
//! A device reads the pages of an account that signed in on it within the
//! last seven days.

use std::net::IpAddr;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pds_oauth::pages::{self, DeviceLine, SessionLine};
use pds_oauth::token::AUTHENTICATION_MAX_AGE_MS;
use pds_storage::Availability;
use subtle::ConstantTimeEq;

use super::authorize::{Device, csrf, device, error_page, html, new_nonce};
use super::{OAuthError, accounts, iso, issuer, millis};
use crate::state::AppState;

/// What the form token of the account pages is computed over, in the place
/// of a request.
const CSRF_PURPOSE: &str = "account";

/// The accounts that signed in on the device recently enough to be shown.
async fn signed_in(state: &AppState, device_id: &str) -> Result<Vec<(String, String)>, OAuthError> {
    let now_ms = state.now_ms();
    let device_id = device_id.to_owned();
    accounts(state, move |db| {
        let mut out = Vec::new();
        for remembered in db.device_accounts(&device_id)? {
            if now_ms - millis(&remembered.updated_at) > AUTHENTICATION_MAX_AGE_MS {
                continue;
            }
            if let Some(actor) = db.actor(&remembered.did, Availability::ANY)? {
                let handle = actor.handle.unwrap_or_else(|| remembered.did.clone());
                out.push((remembered.did, handle));
            }
        }
        Ok(out)
    })
    .await
}

async fn home(
    state: &AppState,
    device: &Device,
    did: &str,
    handle: &str,
) -> Result<Response, OAuthError> {
    let owned = did.to_owned();
    let (tokens, devices) = accounts(state, move |db| {
        Ok((db.account_tokens(&owned)?, db.account_devices(&owned)?))
    })
    .await?;
    let sessions: Vec<SessionLine> = tokens
        .into_iter()
        .map(|token| SessionLine {
            token_id: token.token_id,
            client_id: token.client_id,
            created_at: token.created_at,
            updated_at: token.updated_at,
        })
        .collect();
    let devices: Vec<DeviceLine> = devices
        .into_iter()
        .map(|(row, _)| DeviceLine {
            current: row.id == device.id,
            device_id: row.id,
            user_agent: row
                .user_agent
                .unwrap_or_else(|| "Unknown browser".to_owned()),
            ip_address: row.ip_address,
            last_seen_at: row.last_seen_at,
        })
        .collect();
    let nonce = new_nonce();
    let token = csrf(state, &device.id, CSRF_PURPOSE);
    let page = pages::account_home(&nonce, &token, did, handle, &sessions, &devices);
    Ok(html(StatusCode::OK, &nonce, &device.cookies, page))
}

fn sign_in_page(
    state: &AppState,
    device: &Device,
    status: StatusCode,
    username: &str,
    error: Option<&str>,
) -> Response {
    let nonce = new_nonce();
    let token = csrf(state, &device.id, CSRF_PURPOSE);
    let page = pages::account_sign_in(&nonce, &state.config().hostname, &token, username, error);
    html(status, &nonce, &device.cookies, page)
}

async fn page_inner(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Result<Response, OAuthError> {
    let Some(device) = device(state, headers, peer, true).await? else {
        return Err(OAuthError::server("no device"));
    };
    match signed_in(state, &device.id).await?.first() {
        Some((did, handle)) => home(state, &device, did, handle).await,
        None => Ok(sign_in_page(state, &device, StatusCode::OK, "", None)),
    }
}

fn failed(error: &OAuthError) -> Response {
    error_page(error.status, &[], &error.description)
}

/// `GET /account`.
pub async fn page(state: &AppState, headers: &HeaderMap, peer: Option<IpAddr>) -> Response {
    page_inner(state, headers, peer)
        .await
        .unwrap_or_else(|error| failed(&error))
}

fn back(cookies: &[HeaderValue]) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    headers.insert(header::LOCATION, HeaderValue::from_static("/account"));
    for cookie in cookies {
        headers.append(header::SET_COOKIE, cookie.clone());
    }
    response
}

async fn sign_in(
    state: &AppState,
    device: &Device,
    username: &str,
    password: &str,
) -> Result<Response, OAuthError> {
    let refused = |status: StatusCode, message: &str| {
        Ok(sign_in_page(state, device, status, username, Some(message)))
    };
    if username.is_empty() || password.is_empty() {
        return refused(
            StatusCode::BAD_REQUEST,
            "Enter your handle and your password",
        );
    }
    {
        use crate::ratelimit::{Key, consume, of_route};
        let limits = [
            (
                of_route("com.atproto.server.createSession-0", 86_400, 300),
                Key::WithIp(username),
                1,
            ),
            (
                of_route("com.atproto.server.createSession-1", 300, 30),
                Key::WithIp(username),
                1,
            ),
        ];
        if consume(state, &limits).is_err() {
            return refused(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many attempts. Try again later.",
            );
        }
    }
    let (account, app_password) =
        match crate::api::session::timed_login(state, username, password).await {
            Ok(found) => found,
            Err(error) if error.status.is_server_error() => {
                return Err(OAuthError::server(error.message));
            }
            Err(_) => return refused(StatusCode::BAD_REQUEST, "Invalid identifier or password"),
        };
    if app_password.is_some() || account.actor.takedown_ref.is_some() {
        return refused(StatusCode::BAD_REQUEST, "Invalid identifier or password");
    }
    let now_ms = state.now_ms();
    let (device_id, did) = (device.id.clone(), account.actor.did);
    accounts(state, move |db| {
        db.upsert_device_account(&device_id, &did, &iso(now_ms))
    })
    .await?;
    Ok(back(&device.cookies))
}

async fn post_inner(
    state: &AppState,
    path: &str,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    body: Body,
) -> Result<Response, OAuthError> {
    let stop = |status: StatusCode, message: &str| Ok(error_page(status, &[], message));
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    let site = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok());
    if origin.is_some_and(|origin| origin != issuer(state))
        || site.is_some_and(|site| site != "same-origin")
    {
        return stop(StatusCode::FORBIDDEN, "The form came from another site");
    }
    let fields = match super::read_fields(headers, body).await {
        Ok(fields) => fields,
        Err(error) => return stop(StatusCode::BAD_REQUEST, &error.description),
    };
    let Some(device) = device(state, headers, peer, false).await? else {
        return stop(
            StatusCode::BAD_REQUEST,
            "Your browser sent no session. Open the account page again.",
        );
    };
    let expected = csrf(state, &device.id, CSRF_PURPOSE);
    let given = fields.get("csrf").map_or("", String::as_str);
    if !bool::from(expected.as_bytes().ct_eq(given.as_bytes())) {
        return stop(
            StatusCode::FORBIDDEN,
            "The form does not belong to this session",
        );
    }
    let field = |name: &str| fields.get(name).map_or("", String::as_str);
    if path == "/account/sign-in" {
        return sign_in(state, &device, field("username").trim(), field("password")).await;
    }
    // The other forms act on an account that is signed in on this device.
    let did = field("did").to_owned();
    if !signed_in(state, &device.id)
        .await?
        .iter()
        .any(|(own, _)| *own == did)
    {
        return Ok(back(&device.cookies));
    }
    let (token_id, device_id) = (field("token_id").to_owned(), field("device_id").to_owned());
    let revoke = path == "/account/revoke";
    accounts(state, move |db| {
        if revoke {
            // A session of another account is not this account's to end.
            if db.token(&token_id)?.is_some_and(|token| token.did == did) {
                db.delete_token(&token_id)?;
            }
            Ok(())
        } else {
            db.remove_device_account(&device_id, &did)
        }
    })
    .await?;
    Ok(back(&device.cookies))
}

/// `POST /account/sign-in`, `/account/revoke` and `/account/sign-out`.
pub async fn post(
    state: &AppState,
    path: &str,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    body: Body,
) -> Response {
    post_inner(state, path, headers, peer, body)
        .await
        .unwrap_or_else(|error| failed(&error))
}
