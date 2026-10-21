//! OAuth access tokens on the XRPC routes (`docs/SPEC.md` §5.5, §11.3).
//!
//! A request with `Authorization: DPoP` is checked once, when it arrives:
//! the proof, the token, its row and the account. What the token grants
//! then travels with the task, and the handlers read it from there.

use std::cell::RefCell;
use std::future::Future;

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pds_oauth::dpop::DpopError;
use pds_oauth::scope::Permissions;
use pds_storage::Availability;
use serde_json::Value;

use super::{accounts, dpop_proof, millis};
use crate::error::XrpcError;
use crate::state::AppState;

/// What an OAuth access token grants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// The account.
    pub did: String,
    /// The client the token was issued to.
    pub client_id: String,
    /// The session: the `jti` of the token.
    pub token_id: String,
    /// What the scope allows.
    pub permissions: Permissions,
}

tokio::task_local! {
    static GRANT: RefCell<Option<Grant>>;
}

/// The grant of the request that this task serves, when it came with an
/// OAuth access token.
pub fn grant() -> Option<Grant> {
    GRANT
        .try_with(|grant| grant.borrow().clone())
        .ok()
        .flatten()
}

/// Serves a request with its grant in reach of the handlers.
pub async fn with_grant<F: Future>(grant: Option<Grant>, serve: F) -> F::Output {
    GRANT.scope(RefCell::new(grant), serve).await
}

/// Whether the request presents an OAuth access token.
pub fn is_dpop(headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(' ').next())
        .is_some_and(|kind| kind.eq_ignore_ascii_case("dpop"))
}

/// A 401 that names the scheme and the error, as RFC 9449 asks.
// Every caller returns it as the boxed error of `authenticate`.
#[allow(clippy::unnecessary_box_returns)]
fn unauthorized(error: &str, message: &str) -> Box<Response> {
    let mut response = XrpcError {
        status: StatusCode::UNAUTHORIZED,
        error: error.to_owned(),
        message: message.to_owned(),
    }
    .into_response();
    let challenge = format!(
        "DPoP error=\"{error}\", error_description=\"{}\"",
        message.replace(['"', '\\'], "")
    );
    if let Ok(value) = HeaderValue::from_str(&challenge) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    Box::new(response)
}

fn forbidden(message: &str) -> XrpcError {
    XrpcError {
        status: StatusCode::FORBIDDEN,
        error: "Forbidden".to_owned(),
        message: message.to_owned(),
    }
}

/// The refusal of a call that the token's scope does not cover.
pub fn scope_missing(scope: &str) -> XrpcError {
    XrpcError {
        status: StatusCode::FORBIDDEN,
        error: "ScopeMissingError".to_owned(),
        message: format!("Missing required scope \"{scope}\""),
    }
}

/// What the server holds for the `jti` of a token.
enum Found {
    Missing,
    /// The row is bound to another key. It goes, and the token is invalid.
    OtherKey(String),
    /// The row ended. It goes, and the token is expired.
    Expired(String),
    TakenDown,
    Row(Box<pds_storage::oauth::TokenRow>),
}

/// Deletes the row of a token that is refused.
async fn forget_token(state: &AppState, token_id: String) -> Result<(), Box<Response>> {
    accounts(state, move |db| db.delete_token(&token_id))
        .await
        .map_err(|_| Box::new(XrpcError::internal("the token store failed").into_response()))
}

/// What the server holds for the token `jti`, whose proof is bound to the
/// key `jkt`, at `now_ms`. The row is read on a kept read connection; a
/// row that is refused is deleted on the write connection after.
async fn find_token(
    state: &AppState,
    jti: String,
    jkt: String,
    now_ms: i64,
) -> Result<Found, Box<Response>> {
    state
        .blocking(move |state| {
            let db = state.reads().accounts()?;
            let Some(row) = db.token(&jti)? else {
                return Ok(Found::Missing);
            };
            let parameters: Value = serde_json::from_str(&row.parameters).unwrap_or(Value::Null);
            if parameters.get("dpop_jkt").and_then(Value::as_str) != Some(jkt.as_str()) {
                return Ok(Found::OtherKey(row.token_id));
            }
            if millis(&row.expires_at) < now_ms {
                return Ok(Found::Expired(row.token_id));
            }
            Ok(match db.actor(&row.did, Availability::ANY)? {
                None => Found::Missing,
                Some(actor) if actor.takedown_ref.is_some() => Found::TakenDown,
                Some(_) => Found::Row(Box::new(row)),
            })
        })
        .await
        .map_err(|_| Box::new(XrpcError::internal("the token store failed").into_response()))
}

/// The grant of a token on a member host (SPEC §21.3). The host has no
/// row of the session: the entryway has it. The token says the scope, and
/// the host knows the account. The entryway confirms the row is current.
async fn member_grant(
    state: &AppState,
    claims: pds_oauth::token::AccessClaims,
    scope: Option<String>,
) -> Result<Grant, Box<Response>> {
    let did = claims.sub.clone();
    let actor = state
        .blocking(move |state| Ok(state.reads().accounts()?.actor(&did, Availability::ANY)?))
        .await
        .map_err(|_| Box::new(XrpcError::internal("the account store failed").into_response()))?;
    match actor {
        None => Err(unauthorized("invalid_token", "Invalid token")),
        Some(actor) if actor.takedown_ref.is_some() => Err(Box::new(
            XrpcError {
                status: StatusCode::UNAUTHORIZED,
                error: "AccountTakedown".to_owned(),
                message: "Account has been taken down".to_owned(),
            }
            .into_response(),
        )),
        Some(_) => Ok(Grant {
            did: claims.sub,
            client_id: claims.client_id,
            token_id: claims.jti,
            permissions: Permissions::new(&scope.unwrap_or_default()),
        }),
    }
}

/// Checks the token of a request with `Authorization: DPoP`.
pub async fn authenticate(
    state: &AppState,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
) -> Result<Grant, Box<Response>> {
    let invalid = |message: &str| unauthorized("invalid_token", message);
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let parts: Vec<&str> = value.split(' ').collect();
    let [_, token] = parts.as_slice() else {
        return Err(Box::new(
            XrpcError::bad_request("InvalidToken", "Malformed authorization header")
                .into_response(),
        ));
    };
    if state.writes().is_none() {
        return Err(invalid("Invalid token"));
    }
    // HEAD is served as GET, and the proof names the method the client sent.
    let proof = match dpop_proof(state, headers, method, path, Some(token)) {
        Ok(Some(proof)) => proof,
        Ok(None) => return Err(unauthorized("invalid_dpop_proof", "DPoP proof required")),
        Err(DpopError::UseNonce(message)) => return Err(unauthorized("use_dpop_nonce", &message)),
        Err(DpopError::Invalid(message)) => {
            return Err(unauthorized("invalid_dpop_proof", &message));
        }
    };
    let (claims, carried) =
        super::read_access_token(state, token).ok_or_else(|| invalid("Invalid token"))?;
    if claims.jkt.as_deref() != Some(proof.jkt.as_str()) {
        return Err(invalid("Invalid DPoP key binding"));
    }
    let now_ms = state.now_ms();
    if claims.exp * 1000 <= now_ms {
        return Err(invalid("Token expired"));
    }
    if state.config().entryway.is_some() {
        let active =
            crate::auth::member_token_active(state, token, "oauth", &claims.sub, Some(&claims.jti))
                .await
                .map_err(|error| Box::new(error.into_response()))?;
        if !active {
            return Err(invalid("This session has ended"));
        }
        return member_grant(state, claims, carried).await;
    }

    let found = find_token(state, claims.jti.clone(), proof.jkt.clone(), now_ms).await?;
    let row = match found {
        Found::Missing => return Err(invalid("Invalid token")),
        Found::OtherKey(token_id) => {
            forget_token(state, token_id).await?;
            return Err(invalid("Invalid token"));
        }
        Found::Expired(token_id) => {
            forget_token(state, token_id).await?;
            return Err(invalid("Token expired"));
        }
        Found::TakenDown => {
            return Err(Box::new(
                XrpcError {
                    status: StatusCode::UNAUTHORIZED,
                    error: "AccountTakedown".to_owned(),
                    message: "Account has been taken down".to_owned(),
                }
                .into_response(),
            ));
        }
        Found::Row(row) => row,
    };
    if row.did != claims.sub {
        return Err(invalid("Invalid token"));
    }
    let scope = row.scope.clone().unwrap_or_else(|| {
        serde_json::from_str::<Value>(&row.parameters)
            .ok()
            .and_then(|parameters| {
                parameters
                    .get("scope")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default()
    });
    Ok(Grant {
        did: row.did,
        client_id: row.client_id,
        token_id: row.token_id,
        permissions: Permissions::new(&scope),
    })
}

/// The methods an OAuth token may call on this server, each with the check
/// of its scope that needs no input. A method with a handler that is not
/// here refuses every OAuth token, as the Reference's account management
/// does. A method without a handler goes to the proxy, which checks the
/// `rpc` permission for its service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// Any token.
    Any,
    /// The handler checks the collection, the type or the service.
    Handler,
    /// `identity:handle`.
    Handle,
    /// `account:repo?action=manage`.
    ManageRepo,
    /// `account:status?action=manage`.
    ManageStatus,
}

fn rule(nsid: &str) -> Option<Rule> {
    Some(match nsid {
        "com.atproto.server.getSession"
        | "com.atproto.server.describeServer"
        | "com.atproto.identity.resolveHandle"
        | "com.atproto.repo.describeRepo"
        | "com.atproto.repo.getRecord"
        | "com.atproto.repo.listRecords"
        | "com.atproto.repo.listMissingBlobs"
        | "com.atproto.server.checkAccountStatus"
        | "com.atproto.server.reserveSigningKey" => Rule::Any,
        "com.atproto.repo.createRecord"
        | "com.atproto.repo.putRecord"
        | "com.atproto.repo.deleteRecord"
        | "com.atproto.repo.applyWrites"
        | "com.atproto.repo.uploadBlob"
        | "com.atproto.server.getServiceAuth"
        | "app.bsky.actor.getPreferences"
        | "app.bsky.actor.putPreferences" => Rule::Handler,
        "com.atproto.identity.updateHandle" => Rule::Handle,
        "com.atproto.repo.importRepo" => Rule::ManageRepo,
        "com.atproto.server.activateAccount" | "com.atproto.server.deactivateAccount" => {
            Rule::ManageStatus
        }
        nsid if nsid.starts_with("com.atproto.sync.") => Rule::Any,
        _ => return None,
    })
}

/// Whether the grant may call `nsid` at all. `handled` says the server has
/// a handler for the method.
pub fn check_method(grant: &Grant, nsid: &str, handled: bool) -> Result<(), XrpcError> {
    let permissions = &grant.permissions;
    match rule(nsid) {
        None if handled => Err(forbidden(
            "OAuth credentials are not supported for this endpoint",
        )),
        Some(Rule::Any | Rule::Handler) | None => Ok(()),
        Some(Rule::Handle) if permissions.allows_identity("handle") => Ok(()),
        Some(Rule::Handle) => Err(scope_missing("identity:handle")),
        Some(Rule::ManageRepo) if permissions.allows_account("repo", true) => Ok(()),
        Some(Rule::ManageRepo) => Err(scope_missing("account:repo?action=manage")),
        Some(Rule::ManageStatus) if permissions.allows_account("status", true) => Ok(()),
        Some(Rule::ManageStatus) => Err(scope_missing("account:status?action=manage")),
    }
}

/// Refuses a repository write that the grant of this request does not
/// cover. A request without an OAuth token passes.
pub fn check_repo(collection: &str, action: &str) -> Result<(), XrpcError> {
    match grant() {
        Some(grant) if !grant.permissions.allows_repo(collection, action) => {
            Err(scope_missing(&format!("repo:{collection}?action={action}")))
        }
        _ => Ok(()),
    }
}

/// Refuses an upload of a type that the grant of this request does not
/// cover.
pub fn check_blob(mime: &str) -> Result<(), XrpcError> {
    match grant() {
        Some(grant) if !grant.permissions.allows_blob(mime) => {
            Err(scope_missing(&format!("blob:{mime}")))
        }
        _ => Ok(()),
    }
}

/// Refuses a call to a service that the grant of this request does not
/// cover. `aud` is the service: a DID with its fragment.
pub fn check_rpc(lxm: &str, aud: &str) -> Result<(), XrpcError> {
    match grant() {
        Some(grant) if !grant.permissions.allows_rpc(lxm, aud) => {
            Err(scope_missing(&format!("rpc:{lxm}?aud={aud}")))
        }
        _ => Ok(()),
    }
}

/// Whether the request may read the account's email address. A legacy
/// session may.
pub fn may_read_email() -> bool {
    grant().is_none_or(|grant| grant.permissions.allows_account("email", false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(scope: &str) -> Grant {
        Grant {
            did: "did:plc:abc".to_owned(),
            client_id: "https://app.example.org/client-metadata.json".to_owned(),
            token_id: "tok-0".to_owned(),
            permissions: Permissions::new(scope),
        }
    }

    #[test]
    fn account_management_refuses_every_oauth_token() {
        let generic = of("atproto transition:generic");
        for nsid in [
            "com.atproto.server.createAppPassword",
            "com.atproto.server.deleteSession",
            "com.atproto.server.requestAccountDelete",
            "com.atproto.admin.getAccountInfo",
            "com.atproto.identity.signPlcOperation",
        ] {
            let refusal = check_method(&generic, nsid, true).unwrap_err();
            assert_eq!(
                (refusal.status, refusal.error.as_str()),
                (StatusCode::FORBIDDEN, "Forbidden"),
                "{nsid}"
            );
        }
        assert!(check_method(&generic, "com.atproto.server.deactivateAccount", true).is_err());
        assert!(check_method(&generic, "com.atproto.identity.updateHandle", true).is_err());
        assert!(
            check_method(
                &of("atproto identity:handle"),
                "com.atproto.identity.updateHandle",
                true
            )
            .is_ok()
        );
        assert!(check_method(&generic, "com.atproto.repo.createRecord", true).is_ok());
        assert!(check_method(&generic, "com.atproto.sync.getRepo", true).is_ok());
        // A method without a handler goes to the proxy, which checks the service.
        assert!(check_method(&generic, "app.bsky.feed.getTimeline", false).is_ok());
    }
}
