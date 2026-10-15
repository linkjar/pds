//! Invite codes: `com.atproto.server.createInviteCode`,
//! `createInviteCodes`, `getAccountInviteCodes` and
//! `com.atproto.admin.getInviteCodes` (`docs/SPEC.md` §10.2), as the
//! handlers at the pin.
//!
//! Not built: the codes an account earns over time
//! (`PDS_INVITE_INTERVAL`). `getAccountInviteCodes` lists the codes an
//! admin made for the account and creates none.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use pds_storage::{Availability, InviteCode, Invites};
use serde_json::{Map, Value, json};

use super::Query;
use super::admin::require_admin;
use super::writes;
use crate::auth::{self, session};
use crate::body;
use crate::error::{XrpcError, json_response};
use crate::params::Format;
use crate::state::{AppState, WriteState};

const DEFAULT_JSON_LIMIT: usize = 100 * 1024;

/// A code as the methods answer it.
pub(crate) fn view(code: &InviteCode) -> Value {
    let uses: Vec<Value> = code
        .uses
        .iter()
        .map(|(used_by, used_at)| json!({"usedBy": used_by, "usedAt": used_at}))
        .collect();
    json!({
        "code": code.code, "available": code.available, "disabled": code.disabled,
        "forAccount": code.for_account, "createdBy": code.created_by,
        "createdAt": code.created_at, "uses": uses,
    })
}

/// A new code: the host name with dashes, then two groups of five base32
/// characters.
fn new_code(state: &AppState, writes: &WriteState) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut bits = writes.random();
    let mut group = || {
        let mut out = String::with_capacity(5);
        for _ in 0..5 {
            out.push(char::from(ALPHABET[(bits & 31) as usize]));
            bits >>= 5;
        }
        out
    };
    let (first, second) = (group(), group());
    format!(
        "{}-{first}-{second}",
        state.config().hostname.replace('.', "-")
    )
}

fn integer(
    input: &Map<String, Value>,
    key: &str,
    required: bool,
) -> Result<Option<i64>, XrpcError> {
    match input.get(key) {
        None if required => Err(XrpcError::invalid_request(format!(
            "Input must have the property \"{key}\""
        ))),
        None => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| XrpcError::invalid_request(format!("Input/{key} must be integer"))),
    }
}

async fn create(
    state: &AppState,
    accounts: Vec<String>,
    code_count: i64,
    use_count: i64,
) -> Result<Vec<(String, Vec<String>)>, XrpcError> {
    state
        .blocking(move |state| {
            let writes = writes(state)?;
            let now = pds_sequencer::iso_millis(state.now_ms());
            let made: Vec<(String, Vec<String>)> = accounts
                .into_iter()
                .map(|account| {
                    let codes = (0..code_count).map(|_| new_code(state, writes)).collect();
                    (account, codes)
                })
                .collect();
            let rows: Vec<(String, String)> = made
                .iter()
                .flat_map(|(account, codes)| {
                    codes.iter().map(|code| (code.clone(), account.clone()))
                })
                .collect();
            writes.with_accounts(|db| db.create_invite_codes(&rows, use_count, "admin", &now))?;
            Ok(made)
        })
        .await
}

/// `com.atproto.server.createInviteCode`.
pub async fn create_invite_code(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    require_admin(state, headers)?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let use_count = integer(&input, "useCount", true)?.unwrap_or(1);
    let account = body::string(&input, "forAccount", false, Format::Did, None, "$")?
        .unwrap_or("admin")
        .to_owned();
    let made = create(state, vec![account], 1, use_count).await?;
    let code = made
        .first()
        .and_then(|(_, codes)| codes.first())
        .cloned()
        .unwrap_or_default();
    Ok(json_response(StatusCode::OK, &json!({ "code": code })))
}

/// `com.atproto.server.createInviteCodes`.
pub async fn create_invite_codes(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    require_admin(state, headers)?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let code_count = integer(&input, "codeCount", false)?.unwrap_or(1);
    let use_count = integer(&input, "useCount", true)?.unwrap_or(1);
    let accounts: Vec<String> = match input.get("forAccounts") {
        None => vec!["admin".to_owned()],
        Some(Value::Array(accounts)) => accounts
            .iter()
            .filter_map(|account| account.as_str().map(str::to_owned))
            .collect(),
        Some(_) => {
            return Err(XrpcError::invalid_request(
                "Input/forAccounts must be an array",
            ));
        }
    };
    let made = create(state, accounts, code_count, use_count).await?;
    let codes: Vec<Value> = made
        .into_iter()
        .map(|(account, codes)| json!({"account": account, "codes": codes}))
        .collect();
    Ok(json_response(StatusCode::OK, &json!({ "codes": codes })))
}

/// `com.atproto.server.getAccountInviteCodes`.
pub async fn get_account_invite_codes(
    state: &AppState,
    query: &Query<'_>,
) -> Result<Response, XrpcError> {
    let did = session(state, query.headers, Some(auth::ACCESS_FULL))?;
    let include_used = query.params.boolean("includeUsed");
    let codes = state
        .blocking(move |state| {
            super::repo_write::find_account(state, &did, true, false)?;
            let accounts = state.layout().accounts()?;
            if accounts.actor(&did, Availability::ACTIVE)?.is_none() {
                return Err(XrpcError::bad_request("NotFound", "Account not found"));
            }
            Ok(accounts.invite_codes(Invites::For(&did))?)
        })
        .await?;
    let codes: Vec<Value> = codes
        .iter()
        .filter(|code| !code.disabled)
        .filter(|code| {
            include_used || i64::try_from(code.uses.len()).unwrap_or(i64::MAX) < code.available
        })
        .map(view)
        .collect();
    Ok(json_response(StatusCode::OK, &json!({ "codes": codes })))
}

/// `com.atproto.admin.getInviteCodes`.
pub async fn get_invite_codes(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    require_admin(state, query.headers)?;
    let sort = query.params.get("sort").unwrap_or("recent").to_owned();
    let limit = query.params.limit("limit");
    let cursor = query.params.get("cursor").map(str::to_owned);
    let malformed = || XrpcError::invalid_request("Malformed cursor");
    let page = state
        .blocking(move |state| {
            let accounts = state.layout().accounts()?;
            let parts = cursor
                .as_deref()
                .map(|cursor| cursor.split_once("::").ok_or_else(malformed))
                .transpose()?;
            match sort.as_str() {
                "recent" => {
                    // The cursor names the time in milliseconds.
                    let after = parts
                        .map(|(millis, code)| {
                            let millis: i64 = millis.parse().map_err(|_| malformed())?;
                            Ok::<_, XrpcError>((pds_sequencer::iso_millis(millis), code))
                        })
                        .transpose()?;
                    let after = after.as_ref().map(|(at, code)| (at.as_str(), *code));
                    Ok(accounts.invite_codes(Invites::Recent { limit, after })?)
                }
                "usage" => {
                    let after = parts
                        .map(|(uses, code)| {
                            uses.parse::<i64>()
                                .map(|uses| (uses, code))
                                .map_err(|_| malformed())
                        })
                        .transpose()?;
                    Ok(accounts.invite_codes(Invites::Usage { limit, after })?)
                }
                other => Err(XrpcError::invalid_request(format!(
                    "unknown sort method: {other}"
                ))),
            }
        })
        .await?;
    let mut out = Map::new();
    if let Some(last) = page.last() {
        let primary = if query.params.get("sort") == Some("usage") {
            last.uses.len().to_string()
        } else {
            chrono::DateTime::parse_from_rfc3339(&last.created_at)
                .map(|at| at.timestamp_millis().to_string())
                .unwrap_or_default()
        };
        out.insert(
            "cursor".to_owned(),
            format!("{primary}::{}", last.code).into(),
        );
    }
    out.insert(
        "codes".to_owned(),
        page.iter().map(view).collect::<Vec<_>>().into(),
    );
    Ok(json_response(StatusCode::OK, &out.into()))
}

fn strings(input: &Map<String, Value>, key: &str) -> Result<Vec<String>, XrpcError> {
    match input.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_owned).ok_or_else(|| {
                    XrpcError::invalid_request(format!("Input/{key} must be an array of strings"))
                })
            })
            .collect(),
        Some(_) => Err(XrpcError::invalid_request(format!(
            "Input/{key} must be an array"
        ))),
    }
}

/// `com.atproto.admin.disableInviteCodes`: the codes named, and every
/// code of the accounts named. The admin's own codes stay.
pub async fn disable_invite_codes(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    require_admin(state, headers)?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let (codes, accounts) = (strings(&input, "codes")?, strings(&input, "accounts")?);
    if accounts.iter().any(|account| account == "admin") {
        return Err(XrpcError::invalid_request(
            "cannot disable admin invite codes",
        ));
    }
    state
        .blocking(move |state| {
            let writes = state
                .writes()
                .ok_or_else(|| XrpcError::internal("a read-only server changes no invite"))?;
            Ok(writes.with_accounts(|db| db.disable_invite_codes(&codes, &accounts))?)
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}

/// `com.atproto.admin.disableAccountInvites` and `enableAccountInvites`.
pub async fn set_account_invites(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
    disabled: bool,
) -> Result<Response, XrpcError> {
    require_admin(state, headers)?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let account = body::string(&input, "account", true, Format::Did, None, "$")?
        .unwrap_or_default()
        .to_owned();
    body::string(&input, "note", false, Format::Any, None, "$")?;
    crate::audit::set_subject(&account);
    state
        .blocking(move |state| {
            let writes = state
                .writes()
                .ok_or_else(|| XrpcError::internal("a read-only server changes no invite"))?;
            Ok(writes.with_accounts(|db| db.set_invites_disabled(&account, disabled))?)
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}
