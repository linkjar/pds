//! The authorization endpoint and its pages (`docs/SPEC.md` §11.3, §11.4).
//!
//! A browser arrives with a pushed request. The server knows the browser
//! by two cookies, shows the pages of `pds_oauth::pages`, and sends the
//! browser back to the client with a code or with an error.

use std::net::IpAddr;
use std::sync::{Arc, PoisonError};

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, KeyInit, Mac};
use pds_oauth::pages::{self, Consent, Flow};
use pds_oauth::token::{
    AUTHENTICATION_MAX_AGE_MS, EPHEMERAL_SESSION_MAX_AGE_S, Id, PAR_EXPIRES_IN_MS,
    REQUEST_URI_PREFIX, mint_ephemeral, verify_ephemeral,
};
use pds_storage::Availability;
use pds_storage::oauth::{DeviceRow, RequestRow};
use serde_json::{Map, Value, json};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use super::clients::{self, LoadedClient};
use super::{Fields, OAuthError, accounts, iso, issuer, millis, new_id};
use crate::state::AppState;

const DEVICE_COOKIE: &str = "dev-id";
const SESSION_COOKIE: &str = "ses-id";
/// A session identifier changes after 5 minutes.
const SESSION_ROTATION_MS: i64 = 5 * 60 * 1000;
/// For 5 seconds after a change, the identifier before it is taken too:
/// two requests of one page can cross.
const SESSION_FIXATION_MS: i64 = 5 * 1000;
/// A cookie of a device lives 10 years.
const COOKIE_MAX_AGE_S: i64 = 10 * 365 * 24 * 60 * 60;

/// The first step of a sign-up, which the server keeps until the second.
#[derive(Clone)]
pub struct PendingSignUp {
    email: String,
    password: String,
    invite_code: Option<String>,
    device_id: String,
    request_id: String,
    expires_ms: i64,
}

/// A browser, and the cookies to send it.
struct Device {
    id: String,
    cookies: Vec<HeaderValue>,
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

fn set_cookie(state: &AppState, name: &str, value: &str) -> Option<HeaderValue> {
    let secure = if issuer(state).starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={COOKIE_MAX_AGE_S}{secure}"
    ))
    .ok()
}

/// Finds the device of a request by its cookies. With `create`, a browser
/// the server does not know becomes a new device.
async fn device(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    create: bool,
) -> Result<Option<Device>, OAuthError> {
    let now_ms = state.now_ms();
    let ip_address = crate::ratelimit::client_ip(headers, peer);
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let given = cookie(headers, DEVICE_COOKIE)
        .filter(|id| Id::Device.matches(id))
        .zip(cookie(headers, SESSION_COOKIE).filter(|id| Id::Session.matches(id)))
        .map(|(device, session)| (device.to_owned(), session.to_owned()));
    let (row, changed) = accounts(state, move |db| {
        if let Some((id, session)) = given
            && let Some(mut row) = db.device(&id)?
        {
            let age = now_ms - millis(&row.last_seen_at);
            let same = bool::from(row.session_id.as_bytes().ct_eq(session.as_bytes()));
            if same || age <= SESSION_FIXATION_MS {
                let rotate = same && age > SESSION_ROTATION_MS;
                if rotate {
                    row.session_id = new_id(Id::Session);
                }
                row.user_agent = user_agent;
                row.ip_address = ip_address;
                row.last_seen_at = iso(now_ms);
                db.update_device(&row)?;
                return Ok((Some(row), rotate || !same));
            }
            // A session identifier that is neither the current one nor the
            // one just before is a copy.
            db.delete_device(&id)?;
        }
        if !create {
            return Ok((None, false));
        }
        let row = DeviceRow {
            id: new_id(Id::Device),
            session_id: new_id(Id::Session),
            user_agent,
            ip_address,
            last_seen_at: iso(now_ms),
        };
        db.create_device(&row)?;
        Ok((Some(row), true))
    })
    .await?;
    Ok(row.map(|row| Device {
        cookies: if changed {
            [
                set_cookie(state, DEVICE_COOKIE, &row.id),
                set_cookie(state, SESSION_COOKIE, &row.session_id),
            ]
            .into_iter()
            .flatten()
            .collect()
        } else {
            Vec::new()
        },
        id: row.id,
    }))
}

fn html(status: StatusCode, nonce: &str, cookies: &[HeaderValue], body: String) -> Response {
    let mut response = (status, body).into_response();
    let headers = response.headers_mut();
    let csp = format!(
        "default-src 'none'; style-src 'nonce-{nonce}'; base-uri 'none'; frame-ancestors 'none'"
    );
    if let Ok(csp) = HeaderValue::from_str(&csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    }
    for (name, value) in [
        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
        (header::CACHE_CONTROL, "no-store"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "same-origin"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    for cookie in cookies {
        headers.append(header::SET_COOKIE, cookie.clone());
    }
    response
}

fn new_nonce() -> String {
    Id::Request.format(&super::random::<16>())[4..].to_owned()
}

fn error_page(status: StatusCode, cookies: &[HeaderValue], message: &str) -> Response {
    let nonce = new_nonce();
    html(
        status,
        &nonce,
        cookies,
        pages::error(&nonce, "Authorization failed", message),
    )
}

/// Sends the browser back to the client with `outcome`, in the response
/// mode of the request.
fn to_client(
    state: &AppState,
    parameters: &Map<String, Value>,
    cookies: &[HeaderValue],
    outcome: &[(&str, &str)],
) -> Response {
    let text = |name: &str| parameters.get(name).and_then(Value::as_str);
    let issuer = issuer(state);
    let mut fields: Vec<(&str, &str)> = vec![("iss", issuer.as_str())];
    if let Some(given) = text("state") {
        fields.push(("state", given));
    }
    fields.extend_from_slice(outcome);
    let Some(mut target) = text("redirect_uri").and_then(|uri| url::Url::parse(uri).ok()) else {
        return error_page(
            StatusCode::BAD_REQUEST,
            cookies,
            "The request has no redirect URI",
        );
    };
    match text("response_mode") {
        Some("form_post") => {
            let nonce = new_nonce();
            let page = pages::form_post(&nonce, target.as_str(), &fields);
            return html(StatusCode::OK, &nonce, cookies, page);
        }
        Some("fragment") => {
            let fragment = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(&fields)
                .finish();
            target.set_fragment(Some(&fragment));
        }
        _ => {
            target.query_pairs_mut().extend_pairs(&fields);
        }
    }
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    if let Ok(location) = HeaderValue::from_str(target.as_str()) {
        headers.insert(header::LOCATION, location);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    for cookie in cookies {
        headers.append(header::SET_COOKIE, cookie.clone());
    }
    response
}

fn denied(
    state: &AppState,
    parameters: &Map<String, Value>,
    cookies: &[HeaderValue],
    error: &str,
    description: &str,
) -> Response {
    to_client(
        state,
        parameters,
        cookies,
        &[("error", error), ("error_description", description)],
    )
}

/// The token that ties a form to the device and the request. Another site
/// cannot read the device cookie, so it cannot compute the token.
fn csrf(state: &AppState, device_id: &str, request_id: &str) -> String {
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&state.config().auth.jwt_secret) else {
        return String::new();
    };
    mac.update(b"oauth-csrf\0");
    mac.update(device_id.as_bytes());
    mac.update(b"\0");
    mac.update(request_id.as_bytes());
    data_encoding::HEXLOWER.encode(&mac.finalize().into_bytes())
}

/// A pushed request as a page works with it.
struct Pending {
    row: RequestRow,
    parameters: Map<String, Value>,
}

impl Pending {
    fn text(&self, name: &str) -> Option<&str> {
        self.parameters.get(name).and_then(Value::as_str)
    }

    fn scopes(&self) -> Vec<&str> {
        self.text("scope")
            .map(|scope| scope.split(' ').collect())
            .unwrap_or_default()
    }
}

enum Loaded {
    Ready(Box<Pending>),
    /// The request is over. The client hears why.
    Denied(Map<String, Value>, &'static str),
    Unknown,
}

/// Reads the pushed request for this device and this client, binds it to
/// the device and gives it five more minutes, as `RequestManager.get` of
/// the Reference.
async fn load(
    state: &AppState,
    request_uri: &str,
    client_id: &str,
    device_id: &str,
) -> Result<Loaded, OAuthError> {
    let Some(id) = request_uri
        .strip_prefix(REQUEST_URI_PREFIX)
        .filter(|id| Id::Request.matches(id))
    else {
        return Ok(Loaded::Unknown);
    };
    let now_ms = state.now_ms();
    let (id, client_id, device_id) = (id.to_owned(), client_id.to_owned(), device_id.to_owned());
    accounts(state, move |db| {
        let Some(mut row) = db.request(&id)? else {
            return Ok(Loaded::Unknown);
        };
        let parameters: Map<String, Value> =
            serde_json::from_str(&row.parameters).unwrap_or_default();
        let refusal = if row.did.is_some() || row.code.is_some() {
            Some("This request was already authorized")
        } else if millis(&row.expires_at) < now_ms {
            Some("This request has expired")
        } else if row.client_id != client_id {
            Some("This request was initiated for another client")
        } else if row.device_id.as_ref().is_some_and(|own| *own != device_id) {
            Some("This request was initiated from another device")
        } else {
            None
        };
        if let Some(refusal) = refusal {
            db.delete_request(&id)?;
            return Ok(Loaded::Denied(parameters, refusal));
        }
        row.device_id = Some(device_id);
        row.expires_at = iso(now_ms + PAR_EXPIRES_IN_MS);
        db.update_request(&row)?;
        Ok(Loaded::Ready(Box::new(Pending { row, parameters })))
    })
    .await
}

/// An account the device remembers.
struct Session {
    did: String,
    handle: String,
    deactivated: bool,
    login_required: bool,
    consent_required: bool,
}

fn matches_hint(hint: Option<&str>, did: &str, handle: &str) -> bool {
    hint.is_some_and(|hint| hint == did || hint == handle)
}

/// Whether the account must be asked: it never authorized the client, the
/// client asks for consent, or the request has a scope the account did not
/// grant before.
fn consent_required(pending: &Pending, authorized: Option<&str>) -> bool {
    let Some(data) = authorized.and_then(|data| serde_json::from_str::<Value>(data).ok()) else {
        return true;
    };
    if pending.text("prompt") == Some("consent") {
        return true;
    }
    let granted: Vec<&str> = data
        .get("authorizedScopes")
        .and_then(Value::as_array)
        .map(|scopes| scopes.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    !pending.scopes().iter().all(|scope| granted.contains(scope))
}

async fn sessions(
    state: &AppState,
    device_id: &str,
    pending: &Pending,
) -> Result<Vec<Session>, OAuthError> {
    let now_ms = state.now_ms();
    let (device_id, client_id) = (device_id.to_owned(), pending.row.client_id.clone());
    let rows = accounts(state, move |db| {
        let mut out = Vec::new();
        for remembered in db.device_accounts(&device_id)? {
            let Some(actor) = db.actor(&remembered.did, Availability::ANY)? else {
                continue;
            };
            let authorized = db.authorized_client(&remembered.did, &client_id)?;
            out.push((remembered, actor, authorized));
        }
        Ok(out)
    })
    .await?;
    Ok(rows
        .into_iter()
        .map(|(remembered, actor, authorized)| Session {
            handle: actor.handle.unwrap_or_else(|| remembered.did.clone()),
            deactivated: actor.deactivated_at.is_some(),
            login_required: pending.text("prompt") == Some("login")
                || now_ms - millis(&remembered.updated_at) > AUTHENTICATION_MAX_AGE_MS,
            consent_required: consent_required(pending, authorized.as_deref()),
            did: remembered.did,
        })
        .collect())
}

/// Authorizes the request for the account: the code, the record of what
/// the account granted, and the way back to the client.
async fn authorize(
    state: &AppState,
    pending: &Pending,
    did: &str,
    cookies: &[HeaderValue],
) -> Result<Response, OAuthError> {
    let now_ms = state.now_ms();
    let code = new_id(Id::Code);
    let mut row = pending.row.clone();
    let scopes: Vec<String> = pending.scopes().iter().map(|s| (*s).to_owned()).collect();
    let (did, stored_code) = (did.to_owned(), code.clone());
    let outcome = accounts(state, move |db| {
        let available = Availability {
            include_taken_down: false,
            include_deactivated: true,
        };
        let Some(actor) = db.actor(&did, available)? else {
            db.delete_request(&row.id)?;
            return Ok(Err("This account is not available"));
        };
        if actor.deactivated_at.is_some() {
            db.delete_request(&row.id)?;
            return Ok(Err("This account has been deactivated"));
        }
        row.did = Some(did.clone());
        row.code = Some(stored_code);
        row.expires_at = iso(now_ms + PAR_EXPIRES_IN_MS);
        db.update_request(&row)?;
        let mut granted: Vec<String> = db
            .authorized_client(&did, &row.client_id)?
            .and_then(|data| serde_json::from_str::<Value>(&data).ok())
            .and_then(|data| {
                data.get("authorizedScopes")
                    .and_then(Value::as_array)
                    .map(|scopes| {
                        scopes
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
            })
            .unwrap_or_default();
        for scope in scopes {
            if !granted.contains(&scope) {
                granted.push(scope);
            }
        }
        let data = json!({ "authorizedScopes": granted }).to_string();
        db.set_authorized_client(&did, &row.client_id, &data, &iso(now_ms))?;
        Ok(Ok(()))
    })
    .await?;
    Ok(match outcome {
        Ok(()) => to_client(state, &pending.parameters, cookies, &[("code", &code)]),
        Err(why) => denied(state, &pending.parameters, cookies, "access_denied", why),
    })
}

/// What a page of one flow is built from.
struct Page<'a> {
    state: &'a AppState,
    client: &'a LoadedClient,
    pending: &'a Pending,
    device: &'a Device,
}

impl Page<'_> {
    fn render(&self, status: StatusCode, build: impl FnOnce(&Flow<'_>) -> String) -> Response {
        let nonce = new_nonce();
        let request_uri = format!("{REQUEST_URI_PREFIX}{}", self.pending.row.id);
        let csrf = csrf(self.state, &self.device.id, &self.pending.row.id);
        let flow = Flow {
            service: &self.state.config().hostname,
            nonce: &nonce,
            client_id: &self.client.client.id,
            request_uri: &request_uri,
            csrf: &csrf,
        };
        html(status, &nonce, &self.device.cookies, build(&flow))
    }

    fn sign_in(&self, status: StatusCode, username: &str, error: Option<&str>) -> Response {
        let locked = self.pending.text("login_hint").is_some();
        self.render(status, |flow| pages::sign_in(flow, username, locked, error))
    }

    fn sign_up(
        &self,
        status: StatusCode,
        email: &str,
        invite: &str,
        error: Option<&str>,
    ) -> Response {
        let required = self.state.config().invite_required;
        self.render(status, |flow| {
            pages::sign_up_credentials(flow, required, email, invite, error)
        })
    }

    fn consent(&self, did: &str, handle: &str, ephemeral: Option<&str>) -> Response {
        let scopes = self.pending.scopes();
        // A client the operator does not trust is shown by its identifier only.
        let client_name = self.client.client.name().filter(|_| self.client.trusted);
        self.render(StatusCode::OK, |flow| {
            pages::consent(
                flow,
                &Consent {
                    did,
                    handle,
                    client_name,
                    scopes: &scopes,
                    ephemeral,
                },
            )
        })
    }
}

/// The handle to show for a login hint: the hint itself, or the handle of
/// the account when the hint is a DID.
async fn hint_handle(state: &AppState, hint: &str) -> Result<String, OAuthError> {
    if !hint.starts_with("did:") {
        return Ok(hint.to_owned());
    }
    let did = hint.to_owned();
    let handle = accounts(state, move |db| {
        Ok(db
            .actor(&did, Availability::ANY)?
            .and_then(|actor| actor.handle))
    })
    .await?;
    Ok(handle.unwrap_or_else(|| hint.to_owned()))
}

async fn page_inner(
    state: &AppState,
    query: Option<&str>,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Result<Response, OAuthError> {
    let query: Fields = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    let bad = |message: &str| Ok(error_page(StatusCode::BAD_REQUEST, &[], message));
    let Some(client_id) = query.get("client_id") else {
        return bad("The request names no client");
    };
    // PAR is mandatory: the endpoint does not take the parameters themselves.
    let Some(request_uri) = query.get("request_uri") else {
        return bad("This server requires a pushed authorization request");
    };
    let client = match clients::get(state, client_id).await {
        Ok(client) => client,
        Err(error) => return bad(&error.description),
    };
    let Some(device) = device(state, headers, peer, true).await? else {
        return Err(OAuthError::server("no device"));
    };
    let pending = match load(state, request_uri, client_id, &device.id).await? {
        Loaded::Ready(pending) => pending,
        Loaded::Denied(parameters, why) => {
            return Ok(denied(
                state,
                &parameters,
                &device.cookies,
                "access_denied",
                why,
            ));
        }
        Loaded::Unknown => {
            return Ok(error_page(
                StatusCode::BAD_REQUEST,
                &device.cookies,
                "Unknown request_uri",
            ));
        }
    };
    let sessions = sessions(state, &device.id, &pending).await?;
    let prompt = pending.text("prompt");
    let hint = pending.text("login_hint");
    let sso: Vec<&Session> = sessions
        .iter()
        .filter(|session| !session.deactivated && matches_hint(hint, &session.did, &session.handle))
        .collect();
    let refuse = |error: &str, description: &str| {
        Ok(denied(
            state,
            &pending.parameters,
            &device.cookies,
            error,
            description,
        ))
    };
    if prompt == Some("none") {
        return match sso.as_slice() {
            [] => refuse("login_required", "Login is required"),
            [only] if only.login_required => refuse("login_required", "Login is required"),
            [only] if only.consent_required => refuse("consent_required", "Consent is required"),
            [only] => authorize(state, &pending, &only.did, &device.cookies).await,
            _ => refuse(
                "account_selection_required",
                "Account selection is required",
            ),
        };
    }
    if prompt == Some("select_account") && sessions.is_empty() {
        return refuse(
            "account_selection_required",
            "Account selection is required",
        );
    }
    if let (None, Some(_), [only]) = (prompt, hint, sso.as_slice())
        && !only.login_required
        && !only.consent_required
    {
        return authorize(state, &pending, &only.did, &device.cookies).await;
    }

    let page = Page {
        state,
        client: &client,
        pending: &pending,
        device: &device,
    };
    view(&page, &sessions, &query).await
}

/// The page for the state of the flow: what the request asks, what the
/// device remembers and what the user chose on the page before.
async fn view(
    page: &Page<'_>,
    sessions: &[Session],
    query: &Fields,
) -> Result<Response, OAuthError> {
    let prompt = page.pending.text("prompt");
    let hint = page.pending.text("login_hint");
    let hinted = match hint {
        Some(hint) => hint_handle(page.state, hint).await?,
        None => String::new(),
    };
    let chosen = query
        .get("did")
        .and_then(|did| sessions.iter().find(|session| session.did == *did));
    let selected = chosen.or_else(|| {
        sessions
            .iter()
            .filter(|_| matches!(prompt, None | Some("login" | "consent")))
            .find(|session| matches_hint(hint, &session.did, &session.handle))
    });
    Ok(match (query.get("view").map(String::as_str), selected) {
        (Some("sign-in"), _) => page.sign_in(StatusCode::OK, &hinted, None),
        (Some("sign-up"), _) => page.sign_up(StatusCode::OK, "", "", None),
        (Some("welcome"), _) => page.render(StatusCode::OK, pages::welcome),
        (_, _) if prompt == Some("create") => page.sign_up(StatusCode::OK, "", "", None),
        (_, Some(session)) if !session.login_required => {
            page.consent(&session.did, &session.handle, None)
        }
        (_, Some(session)) => page.sign_in(StatusCode::OK, &session.handle, None),
        (_, None) if hint.is_some() || prompt == Some("login") => {
            page.sign_in(StatusCode::OK, &hinted, None)
        }
        (_, None) if !sessions.is_empty() => {
            let list: Vec<(String, String)> = sessions
                .iter()
                .map(|session| (session.did.clone(), session.handle.clone()))
                .collect();
            page.render(StatusCode::OK, |flow| pages::accounts(flow, &list))
        }
        (_, None) => page.render(StatusCode::OK, pages::welcome),
    })
}

fn failed(error: &OAuthError) -> Response {
    error_page(error.status, &[], &error.description)
}

/// `GET /oauth/authorize`.
pub async fn page(
    state: &AppState,
    query: Option<&str>,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Response {
    page_inner(state, query, headers, peer)
        .await
        .unwrap_or_else(|error| failed(&error))
}

/// What a form of a page carries, after the checks every form shares.
struct Posted {
    fields: Fields,
    client: Arc<LoadedClient>,
    device: Device,
    pending: Box<Pending>,
}

/// Reads a form of a page. `Err` is the answer to send in its place.
async fn posted(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    body: Body,
) -> Result<Result<Posted, Response>, OAuthError> {
    let stop = |status: StatusCode, message: &str| Ok(Err(error_page(status, &[], message)));
    // A form of these pages comes from these pages.
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
    let (Some(client_id), Some(request_uri), Some(given)) = (
        fields.get("client_id"),
        fields.get("request_uri"),
        fields.get("csrf"),
    ) else {
        return stop(StatusCode::BAD_REQUEST, "The form is incomplete");
    };
    let Some(device) = device(state, headers, peer, false).await? else {
        return stop(
            StatusCode::BAD_REQUEST,
            "Your browser sent no session. Start again from the application.",
        );
    };
    let client = match clients::get(state, client_id).await {
        Ok(client) => client,
        Err(error) => return stop(StatusCode::BAD_REQUEST, &error.description),
    };
    let pending = match load(state, request_uri, client_id, &device.id).await? {
        Loaded::Ready(pending) => pending,
        Loaded::Denied(parameters, why) => {
            return Ok(Err(denied(
                state,
                &parameters,
                &device.cookies,
                "access_denied",
                why,
            )));
        }
        Loaded::Unknown => return stop(StatusCode::BAD_REQUEST, "Unknown request_uri"),
    };
    let expected = csrf(state, &device.id, &pending.row.id);
    if !bool::from(expected.as_bytes().ct_eq(given.as_bytes())) {
        return stop(
            StatusCode::FORBIDDEN,
            "The form does not belong to this session",
        );
    }
    Ok(Ok(Posted {
        fields,
        client,
        device,
        pending,
    }))
}

impl Posted {
    fn page<'a>(&'a self, state: &'a AppState) -> Page<'a> {
        Page {
            state,
            client: &self.client,
            pending: &self.pending,
            device: &self.device,
        }
    }

    fn field(&self, name: &str) -> &str {
        self.fields.get(name).map_or("", String::as_str)
    }
}

/// After a sign-in or a sign-up: the consent page, or straight back to the
/// client when the account has nothing to decide.
async fn after_authentication(
    state: &AppState,
    posted: &Posted,
    did: &str,
    handle: &str,
    remember: bool,
    ask: Option<bool>,
) -> Result<Response, OAuthError> {
    let now_ms = state.now_ms();
    let authorized = {
        let (device, did) = (posted.device.id.clone(), did.to_owned());
        let client_id = posted.pending.row.client_id.clone();
        accounts(state, move |db| {
            if remember {
                db.upsert_device_account(&device, &did, &iso(now_ms))?;
            } else {
                // A sign-in without "remember" ends what the device remembered of the account.
                db.remove_device_account(&device, &did)?;
            }
            db.authorized_client(&did, &client_id)
        })
        .await?
    };
    let ephemeral = (!remember).then(|| {
        mint_ephemeral(
            &state.config().auth.jwt_secret,
            did,
            &posted.device.id,
            &posted.pending.row.id,
            now_ms / 1000,
        )
    });
    if ask.unwrap_or_else(|| consent_required(&posted.pending, authorized.as_deref())) {
        Ok(posted
            .page(state)
            .consent(did, handle, ephemeral.as_deref()))
    } else {
        authorize(state, &posted.pending, did, &posted.device.cookies).await
    }
}

async fn sign_in_inner(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    body: Body,
) -> Result<Response, OAuthError> {
    let posted = match posted(state, headers, peer, body).await? {
        Ok(posted) => posted,
        Err(response) => return Ok(response),
    };
    let page = posted.page(state);
    let username = posted.field("username").trim().to_owned();
    let password = posted.field("password").to_owned();
    let refused =
        |status: StatusCode, message: &str| Ok(page.sign_in(status, &username, Some(message)));
    if username.is_empty() || password.is_empty() {
        return refused(
            StatusCode::BAD_REQUEST,
            "Enter your handle and your password",
        );
    }
    let found = crate::api::session::timed_login(state, &username, &password).await;
    let (account, app_password) = match found {
        Ok(found) => found,
        Err(error) if error.status == StatusCode::TOO_MANY_REQUESTS => {
            return refused(error.status, "Too many attempts. Try again later.");
        }
        Err(error) if error.status.is_server_error() => {
            return Err(OAuthError::server(error.message));
        }
        Err(_) => return refused(StatusCode::BAD_REQUEST, "Invalid identifier or password"),
    };
    let did = account.actor.did.clone();
    let handle = account.actor.handle.clone().unwrap_or_else(|| did.clone());
    // An app password opens no browser session, and a request that names an account is for that account only.
    let hint = posted.pending.text("login_hint");
    if app_password.is_some() || (hint.is_some() && !matches_hint(hint, &did, &handle)) {
        return refused(StatusCode::BAD_REQUEST, "Invalid identifier or password");
    }
    if account.actor.takedown_ref.is_some() {
        return refused(StatusCode::BAD_REQUEST, "This account has been taken down");
    }
    let remember = posted.fields.contains_key("remember");
    after_authentication(state, &posted, &did, &handle, remember, None).await
}

/// `POST /oauth/authorize/sign-in`.
pub async fn sign_in(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    body: Body,
) -> Response {
    sign_in_inner(state, headers, peer, body)
        .await
        .unwrap_or_else(|error| failed(&error))
}

async fn accept_inner(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    body: Body,
) -> Result<Response, OAuthError> {
    let posted = match posted(state, headers, peer, body).await? {
        Ok(posted) => posted,
        Err(response) => return Ok(response),
    };
    let did = posted.field("did").to_owned();
    let now_ms = state.now_ms();
    let by_proof = posted.fields.get("ephemeral").is_some_and(|proof| {
        verify_ephemeral(
            &state.config().auth.jwt_secret,
            proof,
            &posted.device.id,
            &posted.pending.row.id,
            now_ms / 1000,
        )
        .as_deref()
            == Some(did.as_str())
    });
    let by_device = sessions(state, &posted.device.id, &posted.pending)
        .await?
        .iter()
        .any(|session| session.did == did && !session.login_required);
    if !by_proof && !by_device {
        // The sign-in is over. The flow starts again at the first page.
        let mut response = StatusCode::SEE_OTHER.into_response();
        let location = format!(
            "/oauth/authorize?{}",
            url::form_urlencoded::Serializer::new(String::new())
                .append_pair("client_id", posted.field("client_id"))
                .append_pair("request_uri", posted.field("request_uri"))
                .finish()
        );
        if let Ok(location) = HeaderValue::from_str(&location) {
            response.headers_mut().insert(header::LOCATION, location);
        }
        return Ok(response);
    }
    authorize(state, &posted.pending, &did, &posted.device.cookies).await
}

/// `POST /oauth/authorize/accept`: the "Authorize" button.
pub async fn accept(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    body: Body,
) -> Response {
    accept_inner(state, headers, peer, body)
        .await
        .unwrap_or_else(|error| failed(&error))
}

async fn reject_inner(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    body: Body,
) -> Result<Response, OAuthError> {
    let posted = match posted(state, headers, peer, body).await? {
        Ok(posted) => posted,
        Err(response) => return Ok(response),
    };
    let id = posted.pending.row.id.clone();
    accounts(state, move |db| db.delete_request(&id)).await?;
    Ok(denied(
        state,
        &posted.pending.parameters,
        &posted.device.cookies,
        "access_denied",
        "The user rejected the request",
    ))
}

/// `POST /oauth/authorize/reject`: the "Deny access" button.
pub async fn reject(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    body: Body,
) -> Response {
    reject_inner(state, headers, peer, body)
        .await
        .unwrap_or_else(|error| failed(&error))
}

/// The first step of a sign-up: the checks that need no handle.
async fn sign_up_credentials(state: &AppState, posted: &Posted) -> Result<Response, OAuthError> {
    let page = posted.page(state);
    let email = posted.field("email").trim().to_owned();
    let password = posted.field("password").to_owned();
    let invite = posted.field("inviteCode").trim().to_owned();
    let required = state.config().invite_required;
    let refused =
        |message: &str| Ok(page.sign_up(StatusCode::BAD_REQUEST, &email, &invite, Some(message)));
    if !crate::api::account::email_is_valid(&email) {
        return refused("This email address is not supported, please use a different email.");
    }
    if password.len() < 8 {
        return refused("The password must have at least 8 characters");
    }
    if password.len() > crate::password::NEW_PASSWORD_MAX_LENGTH {
        return refused("The password is too long");
    }
    if required && invite.is_empty() {
        return refused("An invite code is required");
    }
    let problem = {
        let (email, invite) = (email.to_lowercase(), invite.clone());
        accounts(state, move |db| {
            if required && !db.invite_available(&invite)? {
                return Ok(Some("This invite code is not available"));
            }
            if db.account(&email, true, Availability::ANY)?.is_some() {
                return Ok(Some("This email address is already in use"));
            }
            Ok(None)
        })
        .await?
    };
    if let Some(problem) = problem {
        return refused(problem);
    }
    let now_ms = state.now_ms();
    let key = new_id(Id::Code);
    {
        let mut pending = state
            .oauth()
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if pending.len() >= super::TABLE_SWEEP_AT {
            pending.retain(|_, kept| kept.expires_ms > now_ms);
        }
        pending.insert(
            key.clone(),
            PendingSignUp {
                email,
                password,
                invite_code: Some(invite).filter(|invite| required && !invite.is_empty()),
                device_id: posted.device.id.clone(),
                request_id: posted.pending.row.id.clone(),
                expires_ms: now_ms + EPHEMERAL_SESSION_MAX_AGE_S * 1000,
            },
        );
    }
    let domain = state
        .config()
        .handle_domains
        .first()
        .cloned()
        .unwrap_or_default();
    Ok(page.render(StatusCode::OK, |flow| {
        pages::sign_up_handle(flow, &key, "", &domain, None)
    }))
}

/// The second step of a sign-up: the handle, and the account.
async fn sign_up_handle(
    state: &AppState,
    posted: &Posted,
    key: &str,
) -> Result<Response, OAuthError> {
    let page = posted.page(state);
    let now_ms = state.now_ms();
    let first = state
        .oauth()
        .pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .filter(|kept| {
            kept.expires_ms > now_ms
                && kept.device_id == posted.device.id
                && kept.request_id == posted.pending.row.id
        })
        .cloned();
    let Some(first) = first else {
        return Ok(page.sign_up(
            StatusCode::BAD_REQUEST,
            "",
            "",
            Some("The sign-up took too long. Enter your email address again."),
        ));
    };
    let domain = state
        .config()
        .handle_domains
        .first()
        .cloned()
        .unwrap_or_default();
    let typed = posted.field("handle").trim().to_lowercase();
    let handle = if state.config().is_hosted_handle(&typed) {
        typed.clone()
    } else {
        format!("{typed}{domain}")
    };
    let mut input = json!({ "email": first.email, "handle": handle, "password": first.password });
    if let Some(code) = &first.invite_code {
        input["inviteCode"] = Value::String(code.clone());
    }
    let mut json_headers = HeaderMap::new();
    json_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let mut body = Some(Body::from(input.to_string()));
    let created = crate::api::account::create_account(state, &json_headers, &mut body).await;
    let response = match created {
        Ok(response) => response,
        Err(error) if error.status.is_server_error() => {
            return Err(OAuthError::server(error.message));
        }
        Err(error) => {
            return Ok(page.render(StatusCode::BAD_REQUEST, |flow| {
                pages::sign_up_handle(flow, key, &typed, &domain, Some(&error.message))
            }));
        }
    };
    state
        .oauth()
        .pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(key);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .map_err(OAuthError::server)?;
    let session: Value = serde_json::from_slice(&bytes).map_err(OAuthError::server)?;
    let did = session
        .get("did")
        .and_then(Value::as_str)
        .ok_or_else(|| OAuthError::server("the new account has no DID"))?;
    // A new account that came through a trusted client has authorized it by signing up there.
    let ask = !posted.client.trusted;
    after_authentication(state, posted, did, &handle, false, Some(ask)).await
}

/// `POST /oauth/authorize/sign-up`: both steps of the sign-up.
pub async fn sign_up(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    body: Body,
) -> Response {
    let outcome = async {
        let posted = match posted(state, headers, peer, body).await? {
            Ok(posted) => posted,
            Err(response) => return Ok(response),
        };
        match posted.fields.get("pending").cloned() {
            Some(key) => sign_up_handle(state, &posted, &key).await,
            None => sign_up_credentials(state, &posted).await,
        }
    };
    outcome.await.unwrap_or_else(|error| failed(&error))
}
