//! `com.atproto.identity` reads.

use axum::http::StatusCode;
use axum::response::Response;
use pds_storage::Availability;
use serde_json::json;

use super::Query;
use crate::error::{XrpcError, json_response};
use crate::state::AppState;

fn unresolved() -> XrpcError {
    XrpcError::invalid_request("Unable to resolve handle")
}

/// `com.atproto.identity.resolveHandle`: a hosted handle from the account
/// database, any other handle from the AppView (SPEC §9.2).
pub async fn resolve_handle(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let name = query.params.text("handle").to_ascii_lowercase();
    let local = state
        .blocking({
            let name = name.clone();
            move |state| {
                let accounts = state.reads().accounts()?;
                Ok(accounts.actor(&name, Availability::ACTIVE)?)
            }
        })
        .await?;
    if let Some(actor) = local {
        return Ok(json_response(StatusCode::OK, &json!({ "did": actor.did })));
    }
    // A handle under a hosted suffix is in the database or does not exist.
    if state.config().is_hosted_handle(&name) {
        return Err(XrpcError::bad_request(
            "HandleNotFound",
            "Unable to resolve handle",
        ));
    }
    // Without an AppView the Reference resolves the handle itself, over DNS
    // and HTTPS. That resolver arrives with `updateHandle` (docs/plan.md,
    // unit 5); until then such a handle does not resolve here.
    let appview = state
        .config()
        .appview_url
        .as_deref()
        .ok_or_else(unresolved)?;
    let response = state
        .http()
        // A valid handle holds no character that a query escapes.
        .get(format!(
            "{appview}/xrpc/com.atproto.identity.resolveHandle?handle={name}"
        ))
        .timeout(state.config().proxy_timeout)
        .send()
        .await
        .map_err(|_| unresolved())?;
    if !response.status().is_success() {
        return Err(unresolved());
    }
    let bytes = response.bytes().await.map_err(|_| unresolved())?;
    let body: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| unresolved())?;
    // The Reference passes on the `did` of the answer and does not check
    // it: an answer without one gives an empty object.
    let mut answer = serde_json::Map::new();
    if let Some(did) = body.get("did").and_then(|did| did.as_str()) {
        answer.insert("did".to_owned(), did.into());
    }
    Ok(json_response(StatusCode::OK, &answer.into()))
}
