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
pub(crate) fn full_or_takendown(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<String, XrpcError> {
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
    activate(state, did).await?;
    Ok(StatusCode::OK.into_response())
}

/// Activates the account `did`, as `activateAccount` of the account
/// manager: the published identity is checked first.
pub(crate) async fn activate(state: &AppState, did: String) -> Result<(), XrpcError> {
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
    Ok(())
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

/// Sends the PLC directory the operation that gives `did` the handle.
async fn publish_handle(
    state: &AppState,
    identity: &crate::state::IdentityConfig,
    did: &str,
    handle: &str,
) -> Result<(), XrpcError> {
    let signer = Keypair::from_secret_bytes(Algorithm::Secp256k1, &identity.rotation_key)
        .map_err(XrpcError::internal)?;
    let last = state
        .http()
        .get(format!("{}/{did}/log/last", identity.plc_url))
        .send()
        .await
        .map_err(XrpcError::internal)?;
    if !last.status().is_success() {
        return Err(XrpcError::internal(format!(
            "the PLC directory answered {} for the last operation of {did}",
            last.status().as_u16()
        )));
    }
    let last: Value = serde_json::from_slice(&last.bytes().await.map_err(XrpcError::internal)?)
        .map_err(XrpcError::internal)?;
    let operation =
        pds_identity::plc::update_handle_op(&last, &signer, handle).map_err(XrpcError::internal)?;
    let sent = state
        .http()
        .post(format!("{}/{did}", identity.plc_url))
        .header("content-type", "application/json")
        .body(operation.to_string())
        .send()
        .await
        .map_err(XrpcError::internal)?;
    if !sent.status().is_success() {
        return Err(XrpcError::internal(format!(
            "the PLC directory answered {} to the handle change of {did}",
            sent.status().as_u16()
        )));
    }
    Ok(())
}

/// `com.atproto.identity.updateHandle`: the PLC directory first, then the
/// account, then `#identity`.
pub async fn update_handle(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let did = session(state, headers, None)?;
    {
        let did = did.clone();
        state
            .blocking(move |state| {
                super::repo_write::find_account(state, &did, true, false).map(|_| ())
            })
            .await?;
    }
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let given = body::string(&input, "handle", true, Format::Handle, None, "$")?
        .unwrap_or_default()
        .to_owned();
    change_handle(state, did, given, false).await?;
    Ok(StatusCode::OK.into_response())
}

/// Gives the account `did` the handle `given`, as `updateHandle` of the
/// account manager. The admin may give a reserved name.
pub(crate) async fn change_handle(
    state: &AppState,
    did: String,
    given: String,
    allow_reserved: bool,
) -> Result<(), XrpcError> {
    let identity = super::account::identity(state)?.clone();
    let (handle, hosted) = {
        let (did, identity) = (did.clone(), identity.clone());
        state
            .blocking(move |state| {
                let accounts = AccountDb::open(&state.layout().account_db)?;
                let availability = Availability {
                    include_taken_down: false,
                    include_deactivated: true,
                };
                if accounts.actor(&did, availability)?.is_none() {
                    return Err(XrpcError::invalid_request("Account not found"));
                }
                let (handle, hosted) =
                    super::account::checked_handle(state, &given, &identity, true, allow_reserved)?;
                if accounts
                    .actor(&handle, Availability::ANY)?
                    .is_some_and(|other| other.did != did)
                {
                    return Err(XrpcError::bad_request(
                        "HandleNotAvailable",
                        format!("Handle already taken: {handle}"),
                    ));
                }
                Ok((handle, hosted))
            })
            .await?
    };
    // A handle under another domain must name the account already: its
    // well-known file is read over HTTPS. The DNS record is not read yet.
    if !hosted {
        let named = state
            .http()
            .get(format!("https://{handle}/.well-known/atproto-did"))
            .send()
            .await
            .ok()
            .filter(|response| response.status().is_success());
        let text = match named {
            Some(response) => response.text().await.unwrap_or_default(),
            None => String::new(),
        };
        let resolved = pds_identity::handle::did_from_well_known(&text);
        if resolved.as_ref().map(Did::as_str) != Some(did.as_str()) {
            return Err(XrpcError::invalid_request(
                "External handle did not resolve to DID",
            ));
        }
    }
    if !did.starts_with("did:plc:") {
        return Err(XrpcError::invalid_request(
            "DID is not properly configured for handle",
        ));
    }
    publish_handle(state, &identity, &did, &handle).await?;
    state
        .blocking(move |state| {
            let writes = writes(state)?;
            if !writes.with_accounts(|accounts| accounts.update_handle(&did, &handle))? {
                return Err(XrpcError::invalid_request(
                    "Handle is already in use, please choose a different handle.",
                ));
            }
            let now = pds_sequencer::iso_millis(state.now_ms());
            if let Err(error) =
                writes
                    .sequencer()
                    .sequence(&[identity_event(&did, Some(&handle), &now)])
            {
                tracing::error!(%did, %handle, %error, "failed to sequence handle update");
            }
            Ok(())
        })
        .await?;
    Ok(())
}

/// Removes an account for good: its rows, every event of it but the one
/// that says it is deleted, its blobs and its actor store. The steps are
/// safe to repeat.
pub(crate) fn delete_account_now(state: &AppState, did: &str) -> Result<(), XrpcError> {
    let writes = writes(state)?;
    writes.with_accounts(|accounts| accounts.delete_account(did))?;
    let now = pds_sequencer::iso_millis(state.now_ms());
    let sequenced =
        writes
            .sequencer()
            .sequence_deletion(&account_event(did, Some("deleted"), &now));
    // The store goes whether the event was written or not, as at the pin.
    let layout = state.layout();
    if let (Some(blobs), Ok(store)) = (state.blobs(), ActorDb::open(&layout.actor_db(did))) {
        for (cid, staged) in store.all_blob_rows().unwrap_or_default() {
            let removed = match staged {
                Some(key) => blobs.delete_staged(did, &key),
                None => blobs.delete(did, &cid),
            };
            if let Err(error) = removed {
                tracing::error!(%did, %cid, %error, "a blob of a deleted account was not removed");
            }
        }
    }
    writes.forget_store(did);
    match std::fs::remove_dir_all(layout.actor_store_dir(did)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            tracing::error!(%did, %error, "the actor store of a deleted account was not removed");
        }
        _ => {}
    }
    sequenced?;
    Ok(())
}

/// `com.atproto.server.requestAccountDelete`.
pub async fn request_account_delete(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Response, XrpcError> {
    let did = session(state, headers, Some(auth::ACCESS_FULL))?;
    let email = {
        let did = did.clone();
        state
            .blocking(move |state| {
                super::repo_write::find_account(state, &did, true, false)?;
                AccountDb::open(&state.layout().account_db)?
                    .account(&did, false, Availability::ANY)?
                    .ok_or_else(|| XrpcError::invalid_request("account not found"))?
                    .email
                    .ok_or_else(|| {
                        XrpcError::invalid_request("account does not have an email address")
                    })
            })
            .await?
    };
    let token = super::email::create_token(state, &did, "delete_account").await?;
    state
        .config()
        .mail
        .send(crate::mail::Template::DeleteAccount, &email, &token, None)
        .await?;
    Ok(StatusCode::OK.into_response())
}

/// `com.atproto.server.deleteAccount`: the DID, the account password and
/// the token of the mail.
pub async fn delete_account(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let text = |key: &str, format: Format| {
        body::string(&input, key, true, format, None, "$")
            .map(|value| value.unwrap_or_default().to_owned())
    };
    let did = text("did", Format::Did)?;
    let given = text("password", Format::Any)?;
    let token = text("token", Format::Any)?;
    let denied = |message: &str| XrpcError {
        status: StatusCode::UNAUTHORIZED,
        error: "AuthenticationRequired".to_owned(),
        message: message.to_owned(),
    };
    if given.len() > crate::password::OLD_PASSWORD_MAX_LENGTH {
        return Err(denied(
            "Password too long. Consider resetting your password.",
        ));
    }
    state
        .blocking(move |state| {
            let accounts = AccountDb::open(&state.layout().account_db)?;
            if accounts.actor(&did, Availability::ANY)?.is_none() {
                return Err(XrpcError::invalid_request("account not found"));
            }
            let valid = accounts
                .password_hash(&did)?
                .is_some_and(|stored| crate::password::verify(&given, &stored));
            if !valid {
                return Err(denied("Invalid did or password"));
            }
            super::email::valid_token(state, "delete_account", &token, Some(&did))?;
            drop(accounts);
            delete_account_now(state, &did)
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}
