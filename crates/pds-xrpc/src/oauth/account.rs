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
use pds_oauth::pages::{self, DeviceLine, MethodLine, Methods, SessionLine};
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
pub(super) async fn signed_in(
    state: &AppState,
    device_id: &str,
) -> Result<Vec<(String, String)>, OAuthError> {
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

/// The sign-in methods of an account: its password, the identities that
/// are linked, and the providers it can link.
async fn methods(state: &AppState, did: &str) -> Result<Methods, OAuthError> {
    let providers = state.config().profile.providers();
    if providers.is_empty() {
        return Ok(Methods {
            has_password: true,
            ..Methods::default()
        });
    }
    let owned = did.to_owned();
    let (has_password, identities) = accounts(state, move |db| {
        Ok((db.has_password(&owned)?, db.external_identities(&owned)?))
    })
    .await?;
    let label = |id: &str| {
        providers
            .iter()
            .find(|provider| provider.id == id)
            .map(|provider| provider.label)
    };
    // A provider without credentials signs nobody in, so its identity
    // does not count as a method.
    let usable = identities
        .iter()
        .filter(|identity| label(&identity.provider).is_some())
        .count();
    let linked: Vec<MethodLine> = identities
        .iter()
        .map(|identity| MethodLine {
            label: label(&identity.provider)
                .unwrap_or(identity.provider.as_str())
                .to_owned(),
            provider: identity.provider.clone(),
            subject: identity.subject.clone(),
            email: identity.email.clone(),
        })
        .collect();
    let offered = providers
        .iter()
        .filter(|provider| !identities.iter().any(|own| own.provider == provider.id))
        .map(|provider| (provider.id.to_owned(), provider.label.to_owned()))
        .collect();
    Ok(Methods {
        can_unlink: usize::from(has_password) + usable > 1,
        has_password,
        linked,
        offered,
    })
}

async fn home(
    state: &AppState,
    device: &Device,
    did: &str,
    handle: &str,
    error: Option<&str>,
) -> Result<Response, OAuthError> {
    let owned = did.to_owned();
    let (tokens, devices) = accounts(state, move |db| {
        Ok((db.account_tokens(&owned)?, db.account_devices(&owned)?))
    })
    .await?;
    let methods = methods(state, did).await?;
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
    let page = pages::account_home(
        &nonce, &token, did, handle, &methods, &sessions, &devices, error,
    );
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    Ok(html(status, &nonce, &device.cookies, page))
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
    let providers: Vec<(&str, &str)> = state
        .config()
        .profile
        .providers()
        .iter()
        .map(|provider| (provider.id, provider.label))
        .collect();
    let page = pages::account_sign_in(
        &nonce,
        &state.config().hostname,
        &token,
        username,
        error,
        &providers,
    );
    html(status, &nonce, &device.cookies, page)
}

async fn page_inner(
    state: &AppState,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Result<Response, OAuthError> {
    let Some(device) = device(state, headers, peer, true).await? else {
        return Err(OAuthError::server("no device"));
    };
    let accounts = signed_in(state, &device.id).await?;
    // The page of one account: `/account/<did>/manage`.
    if let Some(did) = path
        .strip_prefix("/account/")
        .and_then(|rest| rest.strip_suffix("/manage"))
    {
        return match accounts.iter().find(|(own, _)| own == did) {
            Some((did, handle)) => home(state, &device, did, handle, None).await,
            None => Ok(back(&device.cookies)),
        };
    }
    let wants_sign_in = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .any(|(name, value)| name == "view" && value == "sign-in");
    if wants_sign_in {
        return Ok(sign_in_page(state, &device, StatusCode::OK, "", None));
    }
    let nonce = new_nonce();
    let page = if accounts.is_empty() {
        pages::account_welcome(&nonce, &state.config().hostname)
    } else {
        pages::account_list(&nonce, &accounts)
    };
    Ok(html(StatusCode::OK, &nonce, &device.cookies, page))
}

fn failed(error: &OAuthError) -> Response {
    error_page(error.status, &[], &error.description)
}

/// `GET /account` and `GET /account/<did>/manage`.
pub async fn page(
    state: &AppState,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Response {
    page_inner(state, path, query, headers, peer)
        .await
        .unwrap_or_else(|error| failed(&error))
}

pub(super) fn back(cookies: &[HeaderValue]) -> Response {
    to("/account", cookies)
}

/// A redirect to a page of the account pages.
pub(super) fn to(location: &str, cookies: &[HeaderValue]) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    if let Ok(location) = HeaderValue::from_str(location) {
        headers.insert(header::LOCATION, location);
    }
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
    let manage = format!("/account/{did}/manage");
    if path == "/account/unlink" {
        let usable: Vec<&'static str> = state
            .config()
            .profile
            .providers()
            .iter()
            .map(|provider| provider.id)
            .collect();
        let (provider, subject) = (field("provider").to_owned(), field("subject").to_owned());
        let now_ms = state.now_ms();
        let notice = super::new_id(pds_oauth::token::Id::Request);
        let owned = did.clone();
        let removed = accounts(state, move |db| {
            db.unlink_identity(&owned, &provider, &subject, &usable, &notice, now_ms)
        })
        .await?;
        if !removed {
            let handle = signed_in(state, &device.id)
                .await?
                .into_iter()
                .find(|(own, _)| *own == did)
                .map_or_else(|| did.clone(), |(_, handle)| handle);
            let message = "This is your only sign-in method. Add another one before you remove it.";
            return home(state, &device, &did, &handle, Some(message)).await;
        }
        return Ok(to(&manage, &device.cookies));
    }
    let (token_id, device_id) = (field("token_id").to_owned(), field("device_id").to_owned());
    let revoke = path == "/account/revoke";
    let owned = did.clone();
    accounts(state, move |db| {
        if revoke {
            // A session of another account is not this account's to end.
            if db.token(&token_id)?.is_some_and(|token| token.did == owned) {
                db.delete_token(&token_id)?;
            }
            Ok(())
        } else {
            db.remove_device_account(&device_id, &owned)
        }
    })
    .await?;
    Ok(to(&manage, &device.cookies))
}

/// `POST /account/sign-in`, `/account/revoke`, `/account/sign-out` and
/// `/account/unlink`.
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
