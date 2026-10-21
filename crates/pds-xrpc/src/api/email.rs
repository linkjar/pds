//! Email tokens: confirming and changing an email address and resetting a
//! password (`docs/SPEC.md` §10.1, §10.3), as the handlers and
//! `account-manager.ts` at the pin.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use data_encoding::HEXLOWER;
use pds_storage::{Account, Availability};
use serde_json::json;

use super::writes;
use crate::auth::{self, session};
use crate::body;
use crate::error::{XrpcError, json_response};
use crate::mail::Template;
use crate::params::Format;
use crate::password;
use crate::state::{AppState, WriteState};

const DEFAULT_JSON_LIMIT: usize = 100 * 1024;
/// How long an email token is valid: 15 minutes.
const TOKEN_LIFETIME_MS: i64 = 15 * 60 * 1000;

/// A new token: ten base32 characters in two groups, in upper case.
pub(crate) fn new_token(writes: &WriteState) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bits = writes.random();
    let mut out = String::with_capacity(11);
    for n in 0..10 {
        if n == 5 {
            out.push('-');
        }
        out.push(char::from(ALPHABET[(bits & 31) as usize]));
        bits >>= 5;
    }
    out
}

/// Stores a new token of `did` for `purpose` and returns it.
pub(crate) async fn create_token(
    state: &AppState,
    did: &str,
    purpose: &'static str,
) -> Result<String, XrpcError> {
    let did = did.to_owned();
    state
        .blocking(move |state| {
            let writes = writes(state)?;
            let token = new_token(writes);
            let now = pds_types::iso_millis(state.now_ms());
            writes.with_accounts(|db| db.create_email_token(&did, purpose, &token, &now))?;
            Ok(token)
        })
        .await
}

/// The account a token belongs to. With `did` the token must be that
/// account's. A token that is unknown is `InvalidToken`, and one older
/// than 15 minutes `ExpiredToken`.
pub(crate) fn valid_token(
    state: &AppState,
    purpose: &str,
    token: &str,
    did: Option<&str>,
) -> Result<String, XrpcError> {
    valid_token_in(
        &state.layout().accounts()?,
        state.now_ms(),
        purpose,
        token,
        did,
    )
}

pub(crate) fn valid_token_in(
    db: &pds_storage::AccountDb,
    now_ms: i64,
    purpose: &str,
    token: &str,
    did: Option<&str>,
) -> Result<String, XrpcError> {
    let found = db
        .email_token(purpose, &token.to_uppercase(), did)?
        .ok_or_else(|| XrpcError::bad_request("InvalidToken", "Token is invalid"))?;
    let requested =
        chrono::DateTime::parse_from_rfc3339(&found.1).map_or(0, |at| at.timestamp_millis());
    if now_ms - requested >= TOKEN_LIFETIME_MS {
        return Err(XrpcError::bad_request("ExpiredToken", "Token is expired"));
    }
    Ok(found.0)
}

/// The account of a session, whatever its status, with its email address.
async fn account_with_email(state: &AppState, did: &str) -> Result<(Account, String), XrpcError> {
    let did = did.to_owned();
    state
        .blocking(move |state| {
            let account = state
                .layout()
                .accounts()?
                .account(&did, false, Availability::ANY)?
                .ok_or_else(|| XrpcError::invalid_request("account not found"))?;
            let email = account.email.clone().ok_or_else(|| {
                XrpcError::invalid_request("account does not have an email address")
            })?;
            Ok((account, email))
        })
        .await
}

/// A session of an account that is not taken down.
async fn active_session(
    state: &AppState,
    headers: &HeaderMap,
    only: Option<&[&str]>,
) -> Result<String, XrpcError> {
    let did = session(state, headers, only)?;
    let checked = did.clone();
    state
        .blocking(move |state| {
            super::repo_write::find_account(state, &checked, true, false).map(|_| ())
        })
        .await?;
    Ok(did)
}

/// `com.atproto.server.requestEmailConfirmation`.
pub async fn request_email_confirmation(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Response, XrpcError> {
    let did = active_session(state, headers, None).await?;
    crate::ratelimit::mail_request(
        state,
        "com.atproto.server.requestEmailConfirmation-0",
        "com.atproto.server.requestEmailConfirmation-1",
        &did,
    )?;
    let (_, email) = account_with_email(state, &did).await?;
    let token = create_token(state, &did, "confirm_email").await?;
    state
        .config()
        .mail
        .send(Template::ConfirmEmail, &email, &token, None)
        .await?;
    Ok(StatusCode::OK.into_response())
}

/// `com.atproto.server.confirmEmail`.
pub async fn confirm_email(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let did = active_session(state, headers, None).await?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let email = body::string(&input, "email", true, Format::Any, None, "$")?
        .unwrap_or_default()
        .to_lowercase();
    let token = body::string(&input, "token", true, Format::Any, None, "$")?
        .unwrap_or_default()
        .to_owned();
    state
        .blocking(move |state| {
            let account = state
                .layout()
                .accounts()?
                .account(&did, false, Availability::ANY)?
                .ok_or_else(|| XrpcError::bad_request("AccountNotFound", "user not found"))?;
            if account.email.as_deref() != Some(email.as_str()) {
                return Err(XrpcError::bad_request("InvalidEmail", "invalid email"));
            }
            valid_token(state, "confirm_email", &token, Some(&did))?;
            let now = pds_types::iso_millis(state.now_ms());
            writes(state)?.with_accounts(|db| db.confirm_email(&did, &now))?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}

/// `com.atproto.server.requestEmailUpdate`. A token is needed, and sent,
/// only when the current address is confirmed.
pub async fn request_email_update(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Response, XrpcError> {
    let did = active_session(state, headers, Some(auth::ACCESS_FULL)).await?;
    crate::ratelimit::mail_request(
        state,
        "com.atproto.server.requestEmailUpdate-0",
        "com.atproto.server.requestEmailUpdate-1",
        &did,
    )?;
    let (account, email) = account_with_email(state, &did).await?;
    let token_required = account.email_confirmed_at.is_some();
    if token_required {
        let token = create_token(state, &did, "update_email").await?;
        state
            .config()
            .mail
            .send(Template::UpdateEmail, &email, &token, None)
            .await?;
    }
    Ok(json_response(
        StatusCode::OK,
        &json!({ "tokenRequired": token_required }),
    ))
}

/// `com.atproto.server.updateEmail`.
pub async fn update_email(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let did = active_session(state, headers, Some(auth::ACCESS_FULL)).await?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let email = body::string(&input, "email", true, Format::Any, None, "$")?
        .unwrap_or_default()
        .to_owned();
    body::boolean(&input, "emailAuthFactor", "$")?;
    let token = body::string(&input, "token", false, Format::Any, None, "$")?.map(str::to_owned);
    if !super::account::email_is_valid(&email) {
        return Err(XrpcError::invalid_request(
            "This email address is not supported, please use a different email.",
        ));
    }
    let headers = headers.clone();
    state
        .blocking(move |state| {
            writes(state)?.with_accounts(|db| {
                db.with_account_lock(&did, |db| {
                    auth::require_current_session(
                        db,
                        &state.config().auth,
                        &headers,
                        state.now_s(),
                    )?;
                    let account = db
                        .account(&did, false, Availability::ANY)?
                        .ok_or_else(|| XrpcError::invalid_request("account not found"))?;
                    match &token {
                        Some(token) => {
                            valid_token_in(db, state.now_ms(), "update_email", token, Some(&did))?;
                        }
                        None if account.email_confirmed_at.is_some() => {
                            return Err(XrpcError::bad_request(
                                "TokenRequired",
                                "confirmation token required",
                            ));
                        }
                        None => {}
                    }
                    if !db.update_email(&did, &email)? {
                        return Err(XrpcError::invalid_request(
                            "This email address is already in use, please use a different email.",
                        ));
                    }
                    Ok(())
                })
            })
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}

/// `com.atproto.server.requestPasswordReset`.
pub async fn request_password_reset(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    {
        use crate::ratelimit::{Key, of_route, route};
        route(
            state,
            &[
                (
                    of_route("com.atproto.server.requestPasswordReset-0", 86_400, 50),
                    Key::Ip,
                    1,
                ),
                (
                    of_route("com.atproto.server.requestPasswordReset-1", 3600, 15),
                    Key::Ip,
                    1,
                ),
            ],
        )?;
    }
    let email = body::string(&input, "email", true, Format::Any, None, "$")?
        .unwrap_or_default()
        .to_lowercase();
    let account = state
        .blocking(move |state| {
            Ok(state
                .layout()
                .accounts()?
                .account(&email, true, Availability::ANY)?)
        })
        .await?;
    let Some((account, to)) = account.and_then(|account| {
        let email = account.email.clone()?;
        Some((account, email))
    }) else {
        // The response must not reveal whether an address has an account.
        return Ok(StatusCode::OK.into_response());
    };
    let token = create_token(state, &account.actor.did, "reset_password").await?;
    let handle = account.actor.handle.as_deref().unwrap_or(&to);
    if let Err(error) = state
        .config()
        .mail
        .send(Template::ResetPassword, &to, &token, Some(handle))
        .await
    {
        tracing::warn!(error = %error.message, "password reset delivery failed");
    }
    Ok(StatusCode::OK.into_response())
}

/// `com.atproto.server.resetPassword`. The sessions of the account end.
pub async fn reset_password(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    crate::ratelimit::route(
        state,
        &[(
            crate::ratelimit::of_route("com.atproto.server.resetPassword-0", 300, 50),
            crate::ratelimit::Key::Ip,
            1,
        )],
    )?;
    let token = body::string(&input, "token", true, Format::Any, None, "$")?
        .unwrap_or_default()
        .to_owned();
    let given = body::string(&input, "password", true, Format::Any, None, "$")?
        .unwrap_or_default()
        .to_owned();
    if given.len() > password::NEW_PASSWORD_MAX_LENGTH {
        return Err(XrpcError::invalid_request("Invalid password length."));
    }
    state
        .blocking(move |state| {
            let did = valid_token(state, "reset_password", &token, None)?;
            let writes = writes(state)?;
            let mut salt = [0_u8; 16];
            for chunk in salt.chunks_mut(8) {
                chunk.copy_from_slice(&writes.random().to_be_bytes());
            }
            let hash = password::hash_with_salt(&given, &HEXLOWER.encode(&salt))
                .ok_or_else(|| XrpcError::internal("the password could not be hashed"))?;
            let notice = state
                .config()
                .profile
                .produces_notices()
                .then(|| crate::oauth::new_id(pds_oauth::token::Id::Request));
            let now_ms = state.now_ms();
            let epoch =
                auth::credential_fingerprint(&state.config().auth, &hash).ok_or_else(|| {
                    XrpcError::internal("the credential version could not be computed")
                })?;
            writes.with_accounts(|db| {
                db.with_account_lock(&did, |db| {
                    valid_token_in(db, state.now_ms(), "reset_password", &token, Some(&did))?;
                    db.update_password(
                        &did,
                        &hash,
                        &epoch,
                        notice.as_deref().map(|id| (id, now_ms)),
                    )?;
                    Ok::<_, XrpcError>(())
                })
            })?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}
