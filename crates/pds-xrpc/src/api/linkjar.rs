//! `io.linkjar.account.getSignupReceipt` and `acknowledgeSignupReceipt`
//! (`docs/SPEC.md` §12.8, §13.3; patch 099b).
//!
//! A session holds a receipt when its account was created in the
//! authorization that opened the session. The receipt is metadata of the
//! session, as `getSession` is: it needs no scope. Only an OAuth session
//! has one.

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use pds_storage::Availability;
use serde_json::json;

use crate::auth::session;
use crate::error::{XrpcError, json_response};
use crate::state::AppState;

const GET: &str = "io.linkjar.account.getSignupReceipt";
const ACKNOWLEDGE: &str = "io.linkjar.account.acknowledgeSignupReceipt";

async fn receipt(
    state: &AppState,
    headers: &HeaderMap,
    acknowledge: bool,
) -> Result<Response, XrpcError> {
    let did = session(state, headers, None)?;
    let grant = crate::oauth::resource::grant()
        .ok_or_else(|| XrpcError::bad_request("InvalidToken", "OAuth session is required"))?;
    let now = pds_types::iso_millis(state.now_ms());
    let handle = {
        let did = did.clone();
        state
            .blocking(move |state| {
                let writes = state
                    .writes()
                    .ok_or_else(|| XrpcError::internal("a read-only server has no sessions"))?;
                writes.with_accounts(|accounts| {
                    if acknowledge {
                        accounts.clear_token_signup(&grant.token_id, &grant.client_id, &did)?;
                        return Ok(None);
                    }
                    if !accounts.token_has_signup(&grant.token_id, &grant.client_id, &did, &now)? {
                        return Ok(None);
                    }
                    // The handle is the account's handle now, also after a rename.
                    let actor = accounts.actor(&did, Availability::ACTIVE)?;
                    Ok::<_, XrpcError>(actor.map(|actor| actor.handle))
                })
            })
            .await?
    };
    let body = match handle {
        Some(handle) => json!({ "created": true, "did": did, "handle": handle }),
        None => json!({ "created": false, "did": did }),
    };
    Ok(json_response(StatusCode::OK, &body))
}

/// Serves the methods of the profile. `None` means the method is not one,
/// or the profile does not have it.
pub async fn serve(
    state: &AppState,
    nsid: &str,
    method: &Method,
    headers: &HeaderMap,
) -> Option<Result<Response, XrpcError>> {
    if !state.config().profile.keeps_signup_receipts() {
        return None;
    }
    let outcome = match nsid {
        GET if method == Method::GET || method == Method::HEAD => {
            receipt(state, headers, false).await
        }
        ACKNOWLEDGE if method == Method::POST => receipt(state, headers, true).await,
        GET => Err(XrpcError::invalid_request(format!(
            "Incorrect HTTP method ({method}) expected GET"
        ))),
        ACKNOWLEDGE => Err(XrpcError::invalid_request(format!(
            "Incorrect HTTP method ({method}) expected POST"
        ))),
        _ => return None,
    };
    // The answer is about one session and no cache may keep it.
    Some(outcome.map(|mut response| {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }))
}
