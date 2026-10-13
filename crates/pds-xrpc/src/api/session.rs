//! Legacy sessions: `com.atproto.server.createSession`, `refreshSession`,
//! `getSession` and `deleteSession` (`docs/SPEC.md` §4.2).
//!
//! The steps and the answers are those of the handlers at the pin and of
//! `account-manager.ts`.

use std::time::{Duration, Instant};

use crate::params::Format;
use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use pds_storage::{Account, AppPassword, Availability};
use serde_json::{Map, Value};

use crate::auth::{
    self, SCOPE_ACCESS, SCOPE_APP_PASS, SCOPE_APP_PASS_PRIVILEGED, SCOPE_TAKENDOWN, session,
};
use crate::body;
use crate::error::XrpcError;
use crate::error::json_response;
use crate::password;
use crate::state::{AppState, WriteState};

/// A session opened with a wrong password takes as long as one opened with
/// the right one: 350 ms at least.
/// The longest JSON body: the Reference's default of 100 KiB.
const DEFAULT_JSON_LIMIT: usize = 100 * 1024;
const LOGIN_FLOOR: Duration = Duration::from_millis(350);
/// A refresh token that was used stays valid this long, so that a client
/// that lost the answer can ask again: 2 hours.
const REFRESH_GRACE_S: i64 = 2 * 60 * 60;

fn unauthorized(error: &str, message: &str) -> XrpcError {
    XrpcError {
        status: StatusCode::UNAUTHORIZED,
        error: error.to_owned(),
        message: message.to_owned(),
    }
}

fn writes(state: &AppState) -> Result<&WriteState, XrpcError> {
    state
        .writes()
        .ok_or_else(|| XrpcError::internal("the server is read-only"))
}

/// The id of a new refresh token: 32 random bytes in base64.
fn token_id(writes: &WriteState) -> String {
    let mut bytes = [0_u8; 32];
    for chunk in bytes.chunks_mut(8) {
        chunk.copy_from_slice(&writes.random().to_be_bytes());
    }
    STANDARD_NO_PAD.encode(bytes)
}

fn scope_of(app_password: Option<&AppPassword>, taken_down: bool) -> &'static str {
    match app_password {
        _ if taken_down => SCOPE_TAKENDOWN,
        None => SCOPE_ACCESS,
        Some(app) if app.privileged => SCOPE_APP_PASS_PRIVILEGED,
        Some(_) => SCOPE_APP_PASS,
    }
}

/// What every session answer says of the account.
fn describe(out: &mut Map<String, Value>, account: &Account) {
    let handle = account.actor.handle.as_deref().unwrap_or("handle.invalid");
    out.insert("did".to_owned(), account.actor.did.clone().into());
    out.insert("handle".to_owned(), handle.into());
    if let Some(email) = &account.email {
        out.insert("email".to_owned(), email.clone().into());
    }
    out.insert(
        "emailConfirmed".to_owned(),
        account.email_confirmed_at.is_some().into(),
    );
    out.insert("active".to_owned(), account.actor.status().is_none().into());
    if let Some(status) = account.actor.status() {
        out.insert("status".to_owned(), status.into());
    }
}

fn tokens(access: String, refresh: String) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("accessJwt".to_owned(), access.into());
    out.insert("refreshJwt".to_owned(), refresh.into());
    out
}

/// The account and, when the password is an app password, which one.
fn login(
    state: &AppState,
    identifier: &str,
    given: &str,
) -> Result<(Account, Option<AppPassword>), XrpcError> {
    let refused = || unauthorized("AuthenticationRequired", "Invalid identifier or password");
    let identifier = identifier.to_lowercase();
    let accounts = state.layout().accounts()?;
    let account = if identifier.contains('@') {
        accounts.account(&identifier, true, Availability::ANY)?
    } else if Format::AtIdentifier.check(&identifier).is_ok() {
        accounts.account(&identifier, false, Availability::ANY)?
    } else {
        None
    };
    let account = account.ok_or_else(refused)?;
    let did = account.actor.did.as_str();
    let stored = accounts.password_hash(did)?;
    if stored.is_some_and(|stored| password::verify(given, &stored)) {
        return Ok((account, None));
    }
    // An account that is taken down opens a session with its own password
    // only.
    if account.actor.takedown_ref.is_some() {
        return Err(refused());
    }
    let hash = password::hash_app_password(did, given).ok_or_else(refused)?;
    let app = accounts.app_password(did, &hash)?.ok_or_else(refused)?;
    Ok((account, Some(app)))
}

/// A sign-in that takes at least [`LOGIN_FLOOR`], so that its duration says
/// little of why it failed.
pub(crate) async fn timed_login(
    state: &AppState,
    identifier: &str,
    given: &str,
) -> Result<(Account, Option<AppPassword>), XrpcError> {
    if given.len() > password::OLD_PASSWORD_MAX_LENGTH {
        return Err(unauthorized(
            "AuthenticationRequired",
            "Password too long. Consider resetting your password.",
        ));
    }
    let started = Instant::now();
    let (identifier, given) = (identifier.to_owned(), given.to_owned());
    let found = state
        .blocking(move |state| login(state, &identifier, &given))
        .await;
    tokio::time::sleep(LOGIN_FLOOR.saturating_sub(started.elapsed())).await;
    found
}

/// `com.atproto.server.createSession`.
pub async fn create_session(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let identifier = body::string(&input, "identifier", true, Format::Any, None, "$")?
        .unwrap_or_default()
        .to_owned();
    let given = body::string(&input, "password", true, Format::Any, None, "$")?
        .unwrap_or_default()
        .to_owned();
    body::string(&input, "authFactorToken", false, Format::Any, None, "$")?;
    let allow_takendown = body::boolean(&input, "allowTakendown", "$")?.unwrap_or(false);
    {
        use crate::ratelimit::{Key, of_route, route};
        route(
            state,
            &[
                (
                    of_route("com.atproto.server.createSession-0", 86_400, 300),
                    Key::WithIp(&identifier),
                    1,
                ),
                (
                    of_route("com.atproto.server.createSession-1", 300, 30),
                    Key::WithIp(&identifier),
                    1,
                ),
            ],
        )?;
    }
    let found = timed_login(state, &identifier, &given).await;
    let (account, app_password) = found?;
    let taken_down = account.actor.takedown_ref.is_some();
    if taken_down && !allow_takendown {
        return Err(unauthorized(
            "AccountTakedown",
            "Account has been taken down",
        ));
    }

    let writes = writes(state)?;
    let now_s = state.now_s();
    let config = &state.config().auth;
    let did = account.actor.did.as_str();
    let scope = scope_of(app_password.as_ref(), taken_down);
    let id = token_id(writes);
    let access = auth::mint_access(config, did, scope, now_s);
    let (refresh, expires_s) = auth::mint_refresh(config, did, &id, now_s);
    // A session of an account that is taken down cannot be refreshed.
    if !taken_down {
        let expires_at = pds_sequencer::iso_millis(expires_s * 1000);
        let name = app_password.as_ref().map(|app| app.name.as_str());
        writes
            .with_accounts(|accounts| accounts.store_refresh_token(&id, did, &expires_at, name))?;
    }
    let mut out = tokens(access, refresh);
    describe(&mut out, &account);
    Ok(json_response(StatusCode::OK, &out.into()))
}

/// `com.atproto.server.refreshSession`.
pub async fn refresh_session(state: &AppState, headers: &HeaderMap) -> Result<Response, XrpcError> {
    let now_s = state.now_s();
    let (did, id) = auth::refresh(headers, &state.config().auth, now_s, false)?;
    let rotated = state
        .blocking(move |state| {
            let account = state
                .layout()
                .accounts()?
                .account(&did, false, Availability::ANY)?
                .ok_or_else(|| {
                    XrpcError::invalid_request(format!(
                        "Could not find user info for account: {did}"
                    ))
                })?;
            if account.actor.takedown_ref.is_some() {
                return Err(unauthorized(
                    "AccountTakedown",
                    "Account has been taken down",
                ));
            }
            let rotated = rotate(state, &id, now_s)?
                .ok_or_else(|| XrpcError::bad_request("ExpiredToken", "Token has been revoked"))?;
            Ok((account, rotated))
        })
        .await?;
    let (account, (access, refresh)) = rotated;
    let mut out = tokens(access, refresh);
    describe(&mut out, &account);
    Ok(json_response(StatusCode::OK, &out.into()))
}

/// Replaces the refresh token `id`, as `rotateRefreshToken` of the
/// Reference: the old token lives for the grace period more, and a second
/// use of it gives the same successor.
fn rotate(state: &AppState, id: &str, now_s: i64) -> Result<Option<(String, String)>, XrpcError> {
    let writes = writes(state)?;
    let config = &state.config().auth;
    let now = pds_sequencer::iso_millis(state.now_ms());
    // Another request can give the token a successor between the read and
    // the write; the second pass reads that successor.
    for _ in 0..3 {
        let Some(token) = writes.with_accounts(|accounts| accounts.refresh_token(id))? else {
            return Ok(None);
        };
        writes
            .with_accounts(|accounts| accounts.delete_expired_refresh_tokens(&token.did, &now))?;
        let grace_ends = pds_sequencer::iso_millis((now_s + REFRESH_GRACE_S) * 1000);
        let expires_at = if grace_ends < token.expires_at {
            grace_ends
        } else {
            token.expires_at.clone()
        };
        if expires_at <= now {
            return Ok(None);
        }
        let next = token.next_id.clone().unwrap_or_else(|| token_id(writes));
        let scope = scope_of(token.app_password.as_ref(), false);
        let access = auth::mint_access(config, &token.did, scope, now_s);
        let (refresh, next_expires_s) = auth::mint_refresh(config, &token.did, &next, now_s);
        let next_expires_at = pds_sequencer::iso_millis(next_expires_s * 1000);
        let rotated = writes.with_accounts(|accounts| {
            accounts.rotate_refresh_token(id, &expires_at, &next, &next_expires_at)
        })?;
        if rotated {
            return Ok(Some((access, refresh)));
        }
    }
    Err(XrpcError::internal(
        "a refresh token changed three times while it was rotated",
    ))
}

/// `com.atproto.server.getSession`.
pub async fn get_session(state: &AppState, headers: &HeaderMap) -> Result<Response, XrpcError> {
    let did = session(state, headers, None)?;
    let account = state
        .blocking(move |state| {
            let availability = Availability {
                include_taken_down: false,
                include_deactivated: true,
            };
            state
                .layout()
                .accounts()?
                .account(&did, false, availability)?
                .ok_or_else(|| {
                    XrpcError::invalid_request(format!(
                        "Could not find user info for account: {did}"
                    ))
                })
        })
        .await?;
    let mut out = Map::new();
    describe(&mut out, &account);
    // An OAuth session reads the email address with its own permission.
    if !crate::oauth::resource::may_read_email() {
        for field in ["email", "emailConfirmed", "emailAuthFactor"] {
            out.remove(field);
        }
    }
    Ok(json_response(StatusCode::OK, &out.into()))
}

/// `com.atproto.server.deleteSession`. The token may be past its expiry.
pub async fn delete_session(state: &AppState, headers: &HeaderMap) -> Result<Response, XrpcError> {
    let (_, id) = auth::refresh(headers, &state.config().auth, state.now_s(), true)?;
    state
        .blocking(move |state| {
            writes(state)?.with_accounts(|accounts| accounts.revoke_refresh_token(&id))?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}

/// A new app password: 16 characters of base32 in four groups.
fn app_password(writes: &WriteState) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity(19);
    let mut bits = 0_u64;
    for n in 0..16 {
        // Twelve characters of five bits from one draw of 64.
        if n % 12 == 0 {
            bits = writes.random();
        }
        if n > 0 && n % 4 == 0 {
            out.push('-');
        }
        out.push(char::from(ALPHABET[(bits & 31) as usize]));
        bits >>= 5;
    }
    out
}

/// `com.atproto.server.createAppPassword`. Only a session of the account
/// password creates one.
pub async fn create_app_password(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let did = session(state, headers, Some(auth::ACCESS_FULL))?;
    {
        let did = did.clone();
        state
            .blocking(move |state| {
                super::repo_write::find_account(state, &did, true, false).map(|_| ())
            })
            .await?;
    }
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let name = body::string(&input, "name", true, Format::Any, None, "$")?
        .unwrap_or_default()
        .to_owned();
    let privileged = body::boolean(&input, "privileged", "$")?.unwrap_or(false);
    let created = state
        .blocking(move |state| {
            let writes = writes(state)?;
            let given = app_password(writes);
            let hash = password::hash_app_password(&did, &given)
                .ok_or_else(|| XrpcError::internal("the app password could not be hashed"))?;
            let created_at = pds_sequencer::iso_millis(state.now_ms());
            let stored = writes.with_accounts(|accounts| {
                accounts.create_app_password(&did, &name, &hash, &created_at, privileged)
            })?;
            // The Reference answers 500 here (SPEC §2.3, DD-3).
            if !stored {
                return Err(XrpcError::invalid_request(format!(
                    "An app password named \"{name}\" already exists"
                )));
            }
            Ok(serde_json::json!({
                "name": name, "password": given, "createdAt": created_at,
                "privileged": privileged,
            }))
        })
        .await?;
    Ok(json_response(StatusCode::OK, &created))
}

/// `com.atproto.server.listAppPasswords`.
pub async fn list_app_passwords(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Response, XrpcError> {
    let did = session(state, headers, None)?;
    let listed = state
        .blocking(move |state| Ok(state.layout().accounts()?.list_app_passwords(&did)?))
        .await?;
    let passwords: Vec<Value> = listed
        .into_iter()
        .map(|(name, created_at, privileged)| {
            serde_json::json!({"name": name, "createdAt": created_at, "privileged": privileged})
        })
        .collect();
    Ok(json_response(
        StatusCode::OK,
        &serde_json::json!({ "passwords": passwords }),
    ))
}

/// `com.atproto.server.revokeAppPassword`. The sessions that the password
/// opened end with it.
pub async fn revoke_app_password(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let did = session(state, headers, None)?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let name = body::string(&input, "name", true, Format::Any, None, "$")?
        .unwrap_or_default()
        .to_owned();
    state
        .blocking(move |state| {
            writes(state)?.with_accounts(|accounts| accounts.revoke_app_password(&did, &name))?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}
