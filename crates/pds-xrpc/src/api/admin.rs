//! Admin methods: `com.atproto.admin.getAccountInfo`, `getSubjectStatus`
//! and `updateSubjectStatus` (`docs/SPEC.md` §10.2), as the handlers at the
//! pin. The caller is the admin: HTTP Basic with `PDS_ADMIN_PASSWORD`.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pds_sequencer::account_event;
use pds_storage::{AccountDb, ActorDb, Availability, Invites};
use serde_json::{Map, Value, json};

use super::Query;
use crate::auth::{self, Credentials};
use crate::body;
use crate::error::{XrpcError, json_response};
use crate::state::{AppState, WriteState};
use crate::write::transact;

const DEFAULT_JSON_LIMIT: usize = 100 * 1024;

fn writes(state: &AppState) -> Result<&WriteState, XrpcError> {
    state
        .writes()
        .ok_or_else(|| XrpcError::internal("the server is read-only"))
}

/// Refuses a caller that is not the admin.
pub(crate) fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), XrpcError> {
    match auth::optional_credentials(headers, &state.config().auth, &[], state.now_s()) {
        Ok(Credentials::Admin) => Ok(()),
        _ => Err(XrpcError::auth_required("AuthenticationRequired")),
    }
}

fn not_found(what: &str) -> XrpcError {
    XrpcError::bad_request("NotFound", format!("{what} not found"))
}

/// An account as the admin sees it, with its invite codes.
fn account_view(accounts: &AccountDb, did: &str) -> Result<Option<Value>, XrpcError> {
    let Some(account) = accounts.account(did, false, Availability::ANY)? else {
        return Ok(None);
    };
    let mut out = Map::new();
    out.insert("did".to_owned(), account.actor.did.clone().into());
    let handle = account.actor.handle.as_deref().unwrap_or("handle.invalid");
    out.insert("handle".to_owned(), handle.into());
    if let Some(email) = &account.email {
        out.insert("email".to_owned(), email.clone().into());
    }
    out.insert(
        "indexedAt".to_owned(),
        account.actor.created_at.clone().into(),
    );
    if let Some(at) = &account.email_confirmed_at {
        out.insert("emailConfirmedAt".to_owned(), at.clone().into());
    }
    if let Some(code) = accounts.invite_codes(Invites::UsedBy(did))?.first() {
        out.insert("invitedBy".to_owned(), super::invites::view(code));
    }
    let invites: Vec<Value> = accounts
        .invite_codes(Invites::For(did))?
        .iter()
        .map(super::invites::view)
        .collect();
    out.insert("invites".to_owned(), invites.into());
    out.insert(
        "invitesDisabled".to_owned(),
        accounts.invites_disabled(did)?.unwrap_or(false).into(),
    );
    if let Some(at) = &account.actor.deactivated_at {
        out.insert("deactivatedAt".to_owned(), at.clone().into());
    }
    Ok(Some(Value::Object(out)))
}

/// `com.atproto.admin.getAccountInfo`.
pub async fn get_account_info(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    require_admin(state, query.headers)?;
    let did = query.params.text("did").to_owned();
    let info = state
        .blocking(move |state| {
            let accounts = AccountDb::open(&state.layout().account_db)?;
            account_view(&accounts, &did)?.ok_or_else(|| not_found("Account"))
        })
        .await?;
    Ok(json_response(StatusCode::OK, &info))
}

/// `com.atproto.admin.getAccountInfos`. A DID without an account is left
/// out.
pub async fn get_account_infos(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    require_admin(state, query.headers)?;
    let dids: Vec<String> = query
        .params
        .list("dids")
        .into_iter()
        .map(str::to_owned)
        .collect();
    let infos = state
        .blocking(move |state| {
            let accounts = AccountDb::open(&state.layout().account_db)?;
            let mut infos = Vec::new();
            for did in &dids {
                if let Some(info) = account_view(&accounts, did)? {
                    infos.push(info);
                }
            }
            Ok(infos)
        })
        .await?;
    Ok(json_response(StatusCode::OK, &json!({ "infos": infos })))
}

fn status(reference: Option<&str>) -> Value {
    match reference {
        Some(reference) => json!({"applied": true, "ref": reference}),
        None => json!({"applied": false}),
    }
}

/// `com.atproto.admin.getSubjectStatus`.
pub async fn get_subject_status(
    state: &AppState,
    query: &Query<'_>,
) -> Result<Response, XrpcError> {
    require_admin(state, query.headers)?;
    let did = query.params.get("did").map(str::to_owned);
    let uri = query.params.get("uri").map(str::to_owned);
    let blob = query.params.get("blob").map(str::to_owned);
    let found = state
        .blocking(move |state| {
            let layout = state.layout();
            if let Some(blob) = blob {
                let did = did.ok_or_else(|| {
                    XrpcError::invalid_request("Must provide a did to request blob state")
                })?;
                let takedown = ActorDb::open(&layout.actor_db(&did))?.blob_takedown_ref(&blob)?;
                return Ok(takedown.map(|takedown| {
                    json!({
                        "subject": {"$type": "com.atproto.admin.defs#repoBlobRef", "did": did, "cid": blob},
                        "takedown": status(takedown.as_deref()),
                    })
                }));
            }
            if let Some(uri) = uri {
                let parsed: pds_types::AtUri = uri
                    .parse()
                    .map_err(|_| XrpcError::invalid_request("Invalid at uri"))?;
                let record = ActorDb::open(&layout.actor_db(&parsed.authority().to_string()))?
                    .record_takedown_ref(&uri)?;
                return Ok(record.map(|(cid, takedown)| {
                    json!({
                        "subject": {"$type": "com.atproto.repo.strongRef", "uri": uri, "cid": cid},
                        "takedown": status(takedown.as_deref()),
                    })
                }));
            }
            let Some(did) = did else {
                return Err(XrpcError::invalid_request("No provided subject"));
            };
            let actor = AccountDb::open(&layout.account_db)?.actor(&did, Availability::ANY)?;
            Ok(actor.map(|actor| {
                json!({
                    "subject": {"$type": "com.atproto.admin.defs#repoRef", "did": did},
                    "takedown": status(actor.takedown_ref.as_deref()),
                    "deactivated": {"applied": actor.deactivated_at.is_some()},
                })
            }))
        })
        .await?
        .ok_or_else(|| not_found("Subject"))?;
    Ok(json_response(StatusCode::OK, &found))
}

/// States the status of an account in the log.
fn sequence_status(state: &AppState, did: &str) -> Result<(), XrpcError> {
    let now = pds_sequencer::iso_millis(state.now_ms());
    let actor = AccountDb::open(&state.layout().account_db)?.actor(did, Availability::ANY)?;
    let status = match &actor {
        None => Some("deleted"),
        Some(actor) => actor.status(),
    };
    writes(state)?
        .sequencer()
        .sequence(&[account_event(did, status, &now)])?;
    Ok(())
}

/// What a status asks for: the reference of a takedown, or `None` to
/// lift it.
fn takedown_ref(state: &AppState, takedown: &Map<String, Value>) -> Option<String> {
    takedown["applied"].as_bool().unwrap_or(false).then(|| {
        takedown
            .get("ref")
            .and_then(Value::as_str)
            .map_or_else(|| pds_sequencer::iso_millis(state.now_ms()), str::to_owned)
    })
}

fn apply_takedown(
    state: &AppState,
    subject: &Map<String, Value>,
    takedown: &Map<String, Value>,
) -> Result<(), XrpcError> {
    let reference = takedown_ref(state, takedown);
    let text = |key: &str| subject.get(key).and_then(Value::as_str).unwrap_or_default();
    match text("$type") {
        "com.atproto.admin.defs#repoRef" => {
            let did = text("did");
            writes(state)?
                .with_accounts(|accounts| accounts.set_takedown(did, reference.as_deref()))?;
            sequence_status(state, did)
        }
        "com.atproto.repo.strongRef" => {
            let uri = text("uri");
            let parsed: pds_types::AtUri = uri
                .parse()
                .map_err(|_| XrpcError::invalid_request("Invalid at uri"))?;
            transact(state, &parsed.authority().to_string(), |session| {
                Ok(session
                    .txn()
                    .set_record_takedown(uri, reference.as_deref())?)
            })
        }
        "com.atproto.admin.defs#repoBlobRef" => {
            let (did, cid) = (text("did"), text("cid"));
            transact(state, did, |session| {
                Ok(session.txn().set_blob_takedown(cid, reference.as_deref())?)
            })?;
            let Some(blobs) = state.blobs() else {
                return Ok(());
            };
            let moved = if reference.is_some() {
                blobs.quarantine(did, cid)
            } else {
                blobs.unquarantine(did, cid)
            };
            match moved {
                // The bytes are where the status says already, or the
                // store never had them.
                Ok(()) | Err(pds_blob::BlobError::NotFound) => Ok(()),
                Err(error) => {
                    tracing::error!(%did, %cid, %error, "could not update blob takedown status");
                    Err(XrpcError::internal(error))
                }
            }
        }
        other => Err(XrpcError::invalid_request(format!(
            "Invalid subject ({other})"
        ))),
    }
}

/// `com.atproto.admin.updateSubjectStatus`.
pub async fn update_subject_status(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    require_admin(state, headers)?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let object = |key: &str| input.get(key).and_then(Value::as_object).cloned();
    let Some(subject) = object("subject") else {
        return Err(XrpcError::invalid_request(
            "Input must have the property \"subject\"",
        ));
    };
    let (takedown, deactivated) = (object("takedown"), object("deactivated"));
    let applied = |status: &Option<Map<String, Value>>| {
        status
            .as_ref()
            .map(|status| status["applied"].as_bool().unwrap_or(false))
    };
    if applied(&takedown) == Some(true) && applied(&deactivated) == Some(false) {
        return Err(XrpcError::invalid_request(
            "Cannot activate and takedown an account at the same time",
        ));
    }
    let is_account =
        subject.get("$type").and_then(Value::as_str) == Some("com.atproto.admin.defs#repoRef");
    let did = subject
        .get("did")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    {
        let (subject, takedown) = (subject.clone(), takedown.clone());
        let deactivate = applied(&deactivated);
        let did = did.clone();
        state
            .blocking(move |state| {
                if let Some(takedown) = &takedown {
                    apply_takedown(state, &subject, takedown)?;
                }
                if is_account && deactivate == Some(true) {
                    let now = pds_sequencer::iso_millis(state.now_ms());
                    let updated = writes(state)?
                        .with_accounts(|accounts| accounts.deactivate(&did, &now, None))?;
                    if !updated {
                        return Err(XrpcError::invalid_request("Account not found"));
                    }
                    sequence_status(state, &did)?;
                }
                if is_account && takedown.is_none() && deactivate.is_none() {
                    sequence_status(state, &did)?;
                }
                Ok(())
            })
            .await?;
    }
    if is_account && applied(&deactivated) == Some(false) {
        super::lifecycle::activate(state, did).await?;
    }
    let mut out = Map::new();
    out.insert("subject".to_owned(), Value::Object(subject));
    if let Some(takedown) = takedown {
        out.insert("takedown".to_owned(), Value::Object(takedown));
    }
    Ok(json_response(StatusCode::OK, &out.into()))
}

/// `com.atproto.admin.deleteAccount`: as the account's own deletion,
/// without a password and a token.
pub async fn delete_account(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    require_admin(state, headers)?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let did = body::string(&input, "did", true, crate::params::Format::Did, None, "$")?
        .unwrap_or_default()
        .to_owned();
    state
        .blocking(move |state| super::lifecycle::delete_account_now(state, &did))
        .await?;
    Ok(axum::response::IntoResponse::into_response(StatusCode::OK))
}
