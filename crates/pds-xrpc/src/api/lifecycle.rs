//! The status of an account: `com.atproto.server.deactivateAccount`,
//! `activateAccount` and `checkAccountStatus` (`docs/SPEC.md` §10.1,
//! §10.4), as the handlers and `account-manager.ts` at the pin.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use pds_identity::{Algorithm, Keypair};
use pds_sequencer::{account_event, identity_event, sync_event};
use pds_storage::{AccountDb, ActorDb, Availability};
use pds_types::Did;
use serde_json::{Value, json};

use crate::auth::{self, SCOPE_TAKENDOWN, session};
use crate::body;
use crate::error::{XrpcError, json_response};
use crate::params::Format;
use crate::state::{AppState, WriteState};

const DEFAULT_JSON_LIMIT: usize = 100 * 1024;

fn writes(state: &AppState) -> Result<&WriteState, XrpcError> {
    state
        .writes()
        .ok_or_else(|| XrpcError::internal("the server is read-only"))
}

/// The session of a method that an account uses while it is taken down
/// too: the account password's scope, or the scope of a taken-down
/// session.
fn full_or_takendown(state: &AppState, headers: &HeaderMap) -> Result<String, XrpcError> {
    if headers
        .get(axum::http::header::AUTHORIZATION)
        .is_none_or(axum::http::HeaderValue::is_empty)
    {
        return Err(XrpcError::auth_required("AuthMissing"));
    }
    match auth::optional_credentials(
        headers,
        &state.config().auth,
        &[SCOPE_TAKENDOWN],
        state.now_s(),
    )? {
        auth::Credentials::Access { did, scope }
            if scope == auth::SCOPE_ACCESS || scope == SCOPE_TAKENDOWN =>
        {
            Ok(did)
        }
        auth::Credentials::Access { .. } => {
            Err(XrpcError::bad_request("InvalidToken", "Bad token scope"))
        }
        _ => Err(XrpcError::bad_request(
            "InvalidToken",
            "Unexpected authorization type",
        )),
    }
}

/// `com.atproto.server.deactivateAccount`.
pub async fn deactivate_account(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let did = full_or_takendown(state, headers)?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let delete_after =
        body::string(&input, "deleteAfter", false, Format::Any, None, "$")?.map(str::to_owned);
    state
        .blocking(move |state| {
            let writes = writes(state)?;
            let now = pds_sequencer::iso_millis(state.now_ms());
            let not_found = || XrpcError::invalid_request("Account not found");
            let updated = writes.with_accounts(|accounts| {
                accounts.deactivate(&did, &now, delete_after.as_deref())
            })?;
            if !updated {
                return Err(not_found());
            }
            let actor = AccountDb::open(&state.layout().account_db)?
                .actor(&did, Availability::ANY)?
                .ok_or_else(not_found)?;
            writes
                .sequencer()
                .sequence(&[account_event(&did, actor.status(), &now)])?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}

/// What the directory says of a DID, for the checks of an activation.
struct Published {
    pds_endpoint: Option<String>,
    signing_key: Option<String>,
    /// `None` for a DID whose method has no rotation keys.
    rotation_keys: Option<Vec<String>>,
}

async fn published(state: &AppState, did: &str) -> Result<Published, XrpcError> {
    let identity = state
        .config()
        .identity
        .as_ref()
        .ok_or_else(|| XrpcError::internal("the PLC rotation key is not configured"))?;
    if did.starts_with("did:plc") {
        let response = state
            .http()
            .get(format!("{}/{did}/data", identity.plc_url))
            .send()
            .await
            .map_err(XrpcError::internal)?;
        if !response.status().is_success() {
            return Err(XrpcError::internal(format!(
                "the PLC directory answered {} for the data of {did}",
                response.status().as_u16()
            )));
        }
        let bytes = response.bytes().await.map_err(XrpcError::internal)?;
        let data: Value = serde_json::from_slice(&bytes).map_err(XrpcError::internal)?;
        let text = |value: &Value| value.as_str().map(str::to_owned);
        return Ok(Published {
            pds_endpoint: text(&data["services"]["atproto_pds"]["endpoint"]),
            signing_key: text(&data["verificationMethods"]["atproto"]),
            rotation_keys: Some(
                data["rotationKeys"]
                    .as_array()
                    .map(|keys| keys.iter().filter_map(text).collect())
                    .unwrap_or_default(),
            ),
        });
    }
    let parsed: Did = did.parse().map_err(XrpcError::internal)?;
    let doc = state
        .resolver()
        .resolve(&parsed, true)
        .await
        .map_err(XrpcError::internal)?
        .ok_or_else(|| XrpcError::invalid_request("Could not resolve DID"))?;
    let doc = pds_identity::DidDocument::from_json(doc.to_string().as_bytes(), usize::MAX)
        .map_err(XrpcError::internal)?;
    Ok(Published {
        pds_endpoint: doc.pds_endpoint().ok().map(|endpoint| endpoint.origin()),
        signing_key: doc.signing_key().ok().map(|key| key.to_did_key()),
        rotation_keys: None,
    })
}

/// The checks of `assertValidDidDocumentForService`: the published
/// identity names this server, the account's signing key and, for
/// `did:plc`, the server's rotation key.
async fn assert_valid_did(state: &AppState, did: &str) -> Result<(), XrpcError> {
    let published = published(state, did).await?;
    let identity = state
        .config()
        .identity
        .as_ref()
        .ok_or_else(|| XrpcError::internal("the PLC rotation key is not configured"))?;
    let rotation = Keypair::from_secret_bytes(Algorithm::Secp256k1, &identity.rotation_key)
        .map_err(XrpcError::internal)?
        .did_key();
    if published
        .rotation_keys
        .as_ref()
        .is_some_and(|keys| !keys.contains(&rotation))
    {
        return Err(XrpcError::invalid_request(
            "Server rotation key not included in PLC DID data",
        ));
    }
    if published.pds_endpoint.as_deref() != Some(identity.public_url.as_str()) {
        return Err(XrpcError::invalid_request(
            "DID document atproto_pds service endpoint does not match PDS public url",
        ));
    }
    let key = {
        let did = did.to_owned();
        state
            .blocking(move |state| Ok(state.layout().read_actor_key(&did)?))
            .await?
    };
    let expected = Keypair::from_secret_bytes(Algorithm::Secp256k1, &key)
        .map_err(XrpcError::internal)?
        .did_key();
    if published.signing_key.as_deref() != Some(expected.as_str()) {
        return Err(XrpcError::invalid_request(
            "DID document verification method does not match expected signing key",
        ));
    }
    Ok(())
}

/// `com.atproto.server.activateAccount`. The events are `#account`,
/// `#identity` and `#sync`, in one write.
pub async fn activate_account(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Response, XrpcError> {
    let did = session(state, headers, Some(auth::ACCESS_FULL))?;
    assert_valid_did(state, &did).await?;
    state
        .blocking(move |state| {
            let writes = writes(state)?;
            let now = pds_sequencer::iso_millis(state.now_ms());
            let not_found = || XrpcError::bad_request("AccountNotFound", "user not found");
            if !writes.with_accounts(|accounts| accounts.activate(&did))? {
                return Err(not_found());
            }
            let actor = AccountDb::open(&state.layout().account_db)?
                .actor(&did, Availability::ANY)?
                .ok_or_else(not_found)?;
            let store = ActorDb::open(&state.layout().actor_db(&did))?;
            let root = store
                .root()?
                .ok_or_else(|| XrpcError::internal("the store has no repository root"))?;
            let block = store
                .block(&root.cid)?
                .ok_or_else(|| XrpcError::internal("the commit block of the head is missing"))?;
            let rev = pds_repo::Commit::decode(&block).map_or_else(
                |_| root.rev.clone(),
                |commit| commit.rev.as_str().to_owned(),
            );
            let handle = actor.handle.as_deref().unwrap_or("handle.invalid");
            writes.sequencer().sequence(&[
                account_event(&did, actor.status(), &now),
                identity_event(&did, Some(handle), &now),
                sync_event(&did, &rev, &root.cid, &block, &now),
            ])?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}

/// `com.atproto.server.checkAccountStatus`.
pub async fn check_account_status(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Response, XrpcError> {
    let did = session(state, headers, None)?;
    let valid_did = assert_valid_did(state, &did).await.is_ok();
    let status = state
        .blocking(move |state| {
            let store = ActorDb::open(&state.layout().actor_db(&did))?;
            let root = store
                .root()?
                .ok_or_else(|| XrpcError::internal("the store has no repository root"))?;
            let (records, blobs, referenced) = store.counts()?;
            let activated = AccountDb::open(&state.layout().account_db)?
                .actor(&did, Availability::ACTIVE)?
                .is_some();
            Ok(json!({
                "activated": activated,
                "validDid": valid_did,
                "repoCommit": root.cid.to_string(),
                "repoRev": root.rev,
                "repoBlocks": store.block_count()?,
                "indexedRecords": records,
                "privateStateValues": 0,
                "expectedBlobs": referenced,
                "importedBlobs": blobs,
            }))
        })
        .await?;
    Ok(json_response(StatusCode::OK, &status))
}
