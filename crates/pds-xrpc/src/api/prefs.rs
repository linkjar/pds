//! Preferences: `app.bsky.actor.getPreferences` and `putPreferences`
//! (`docs/SPEC.md` §6.5). They are stored in the actor store and never
//! reach the AppView, as the handlers at the pin.

use axum::body::Body;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse as _, Response};
use chrono::Datelike as _;
use serde_json::{Value, json};

use super::{Query, proxy};
use crate::auth::{self, Credentials, SCOPE_ACCESS, SCOPE_TAKENDOWN};
use crate::body;
use crate::error::{XrpcError, json_response};
use crate::state::AppState;
use crate::write::transact;

const NAMESPACE: &str = "app.bsky";
const PERSONAL_DETAILS: &str = "app.bsky.actor.defs#personalDetailsPref";
const DECLARED_AGE: &str = "app.bsky.actor.defs#declaredAgePref";
const DEFAULT_JSON_LIMIT: usize = 100 * 1024;

fn in_namespace(name: &str) -> bool {
    name == NAMESPACE || name.starts_with("app.bsky.")
}

/// A session without the account password does not read or write the
/// personal details.
fn allowed(name: &str, full: bool) -> bool {
    full || name != PERSONAL_DETAILS
}

/// Whether the request stays here: an AppView is configured, and
/// `atproto-proxy` names it or is absent.
fn local(state: &AppState, headers: &HeaderMap) -> bool {
    let config = state.config();
    let Some(appview) = &config.appview_did else {
        return false;
    };
    match headers
        .get("atproto-proxy")
        .and_then(|value| value.to_str().ok())
    {
        None => config.appview_url.is_some(),
        Some(proxy_to) => proxy_to == format!("{appview}#bsky_appview"),
    }
}

fn session(
    state: &AppState,
    headers: &HeaderMap,
    lxm: &str,
    additional: &[&str],
) -> Result<(String, bool), XrpcError> {
    match auth::optional_credentials(headers, &state.config().auth, additional, state.now_s())? {
        Credentials::Access { did, scope } => {
            // An OAuth session reads preferences as a call to the AppView,
            // and never the personal details.
            let oauth = crate::oauth::resource::grant().is_some();
            let appview = state.config().appview_did.as_deref().unwrap_or_default();
            crate::oauth::resource::check_rpc(lxm, &format!("{appview}#bsky_appview"))?;
            Ok((did, scope == SCOPE_ACCESS && !oauth))
        }
        Credentials::None => Err(XrpcError::auth_required("AuthMissing")),
        Credentials::Admin => Err(XrpcError::bad_request(
            "InvalidToken",
            "Unexpected authorization type",
        )),
    }
}

/// Full years between a date of birth and today.
fn age(birth: &str, today: chrono::NaiveDate) -> Option<i32> {
    let birth = chrono::DateTime::parse_from_rfc3339(birth)
        .map(|at| at.date_naive())
        .or_else(|_| {
            birth
                .get(..10)
                .unwrap_or(birth)
                .parse::<chrono::NaiveDate>()
        })
        .ok()?;
    let mut age = today.year() - birth.year();
    if (today.month(), today.day()) < (birth.month(), birth.day()) {
        age -= 1;
    }
    Some(age)
}

/// `app.bsky.actor.getPreferences`.
pub async fn get_preferences(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    if !local(state, query.headers) {
        let path = match query.raw_query {
            Some(raw) => format!("/xrpc/app.bsky.actor.getPreferences?{raw}"),
            None => "/xrpc/app.bsky.actor.getPreferences".to_owned(),
        };
        return proxy::proxy(
            state,
            "app.bsky.actor.getPreferences",
            &Method::GET,
            &path,
            query.headers,
            &mut None,
        )
        .await;
    }
    let (did, full) = session(
        state,
        query.headers,
        "app.bsky.actor.getPreferences",
        &[SCOPE_TAKENDOWN],
    )?;
    let today = chrono::DateTime::from_timestamp_millis(state.now_ms())
        .unwrap_or_default()
        .date_naive();
    let preferences = state
        .blocking(move |state| {
            let stored = state.reads().store(&did)?.preferences()?;
            let mut out: Vec<Value> = stored
                .iter()
                .filter(|(_, name, _)| in_namespace(name))
                .filter_map(|(_, _, value)| serde_json::from_str(value).ok())
                .collect();
            let declared = out
                .iter()
                .find(|pref| pref["$type"] == PERSONAL_DETAILS)
                .and_then(|pref| pref["birthDate"].as_str())
                .and_then(|birth| age(birth, today));
            if let Some(age) = declared {
                out.push(json!({
                    "$type": DECLARED_AGE,
                    "isOverAge13": age >= 13,
                    "isOverAge16": age >= 16,
                    "isOverAge18": age >= 18,
                }));
            }
            out.retain(|pref| allowed(pref["$type"].as_str().unwrap_or_default(), full));
            Ok(out)
        })
        .await?;
    Ok(json_response(
        StatusCode::OK,
        &json!({ "preferences": preferences }),
    ))
}

/// `app.bsky.actor.putPreferences`: the preferences of the namespace are
/// replaced by the ones given.
pub async fn put_preferences(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    if !local(state, headers) {
        return proxy::proxy(
            state,
            "app.bsky.actor.putPreferences",
            &Method::POST,
            "/xrpc/app.bsky.actor.putPreferences",
            headers,
            body,
        )
        .await;
    }
    let (did, full) = session(state, headers, "app.bsky.actor.putPreferences", &[])?;
    {
        let did = did.clone();
        state
            .blocking(move |state| {
                super::repo_write::find_account(state, &did, true, false).map(|_| ())
            })
            .await?;
    }
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let mut put = Vec::new();
    for pref in body::array(&input, "preferences", "$")? {
        let Some(name) = pref.get("$type").and_then(Value::as_str) else {
            return Err(XrpcError::invalid_request("Preference is missing a $type"));
        };
        put.push((name.to_owned(), pref.to_string()));
    }
    if !put.iter().all(|(name, _)| in_namespace(name)) {
        return Err(XrpcError::invalid_request(format!(
            "Some preferences are not in the {NAMESPACE} namespace"
        )));
    }
    let forbidden: Vec<&str> = put
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| !allowed(name, full))
        .collect();
    if !forbidden.is_empty() {
        return Err(XrpcError::invalid_request(format!(
            "Do not have authorization to set preferences: {}",
            forbidden.join(", ")
        )));
    }
    // The declared age is computed and never stored.
    put.retain(|(name, _)| name != DECLARED_AGE);
    state
        .repo_blocking(did.clone(), move |state| {
            transact(state, &did, |session| {
                let remove: Vec<i64> = session
                    .txn()
                    .preferences()?
                    .into_iter()
                    .filter(|(_, name, _)| in_namespace(name) && allowed(name, full))
                    .map(|(id, _, _)| id)
                    .collect();
                Ok(session.txn().replace_preferences(&remove, &put)?)
            })
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_age_counts_full_years() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 13).unwrap_or_default();
        assert_eq!(age("2008-10-13T00:00:00.000Z", today), Some(18));
        assert_eq!(age("2008-10-14T00:00:00.000Z", today), Some(17));
        assert_eq!(age("2013-01-01", today), Some(13));
        assert_eq!(age("not a date", today), None);
        assert!(in_namespace("app.bsky.actor.defs#adultContentPref"));
        assert!(!in_namespace("app.bskyx.pref") && !in_namespace("com.example.pref"));
        assert!(allowed(PERSONAL_DETAILS, true) && !allowed(PERSONAL_DETAILS, false));
    }
}
