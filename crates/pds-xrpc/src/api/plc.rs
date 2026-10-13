//! PLC operations of an account: `getRecommendedDidCredentials`,
//! `requestPlcOperationSignature`, `signPlcOperation` and
//! `submitPlcOperation` of `com.atproto.identity` (`docs/SPEC.md` §9), as
//! the handlers at the pin.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use pds_identity::{Algorithm, Keypair};
use pds_sequencer::identity_event;
use pds_storage::Availability;
use pds_types::{Data, Did};
use serde_json::{Map, Value, json};

use super::account::identity;
use super::lifecycle::full_or_takendown;
use crate::auth::session;
use crate::body;
use crate::error::{XrpcError, json_response};
use crate::mail::Template;
use crate::state::AppState;

const DEFAULT_JSON_LIMIT: usize = 100 * 1024;

fn rotation_key(state: &AppState) -> Result<Keypair, XrpcError> {
    Keypair::from_secret_bytes(Algorithm::Secp256k1, &identity(state)?.rotation_key)
        .map_err(XrpcError::internal)
}

/// The `did:key` of the account's signing key.
async fn signing_key(state: &AppState, did: &str) -> Result<String, XrpcError> {
    let did = did.to_owned();
    let key = state
        .blocking(move |state| Ok(state.layout().read_actor_key(&did)?))
        .await?;
    Ok(Keypair::from_secret_bytes(Algorithm::Secp256k1, &key)
        .map_err(XrpcError::internal)?
        .did_key())
}

/// The handle of an account that is not taken down.
async fn handle_of(state: &AppState, did: &str) -> Result<Option<String>, XrpcError> {
    let did = did.to_owned();
    state
        .blocking(move |state| {
            let availability = Availability {
                include_taken_down: false,
                include_deactivated: true,
            };
            Ok(state
                .layout()
                .accounts()?
                .actor(&did, availability)?
                .and_then(|actor| actor.handle))
        })
        .await
}

/// `com.atproto.identity.getRecommendedDidCredentials`: what the DID
/// document of the account should say for this server.
pub async fn get_recommended_did_credentials(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Response, XrpcError> {
    let did = session(state, headers, None)?;
    let config = identity(state)?.clone();
    let mut rotation_keys = vec![rotation_key(state)?.did_key()];
    if let Some(recovery) = &config.recovery_did_key {
        rotation_keys.insert(0, recovery.clone());
    }
    let mut out = Map::new();
    if let Some(handle) = handle_of(state, &did).await? {
        out.insert("alsoKnownAs".to_owned(), json!([format!("at://{handle}")]));
    }
    out.insert(
        "verificationMethods".to_owned(),
        json!({"atproto": signing_key(state, &did).await?}),
    );
    out.insert("rotationKeys".to_owned(), rotation_keys.into());
    out.insert(
        "services".to_owned(),
        json!({"atproto_pds": {"type": "AtprotoPersonalDataServer", "endpoint": config.public_url}}),
    );
    Ok(json_response(StatusCode::OK, &out.into()))
}

/// `com.atproto.identity.requestPlcOperationSignature`: mails the token
/// that `signPlcOperation` asks for.
pub async fn request_plc_operation_signature(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Response, XrpcError> {
    let did = full_or_takendown(state, headers)?;
    let email = {
        let did = did.clone();
        state
            .blocking(move |state| {
                state
                    .layout()
                    .accounts()?
                    .account(&did, false, Availability::ANY)?
                    .ok_or_else(|| XrpcError::invalid_request("account not found"))?
                    .email
                    .ok_or_else(|| {
                        XrpcError::invalid_request("account does not have an email address")
                    })
            })
            .await?
    };
    let token = super::email::create_token(state, &did, "plc_operation").await?;
    state
        .config()
        .mail
        .send(Template::PlcOperation, &email, &token, None)
        .await?;
    Ok(StatusCode::OK.into_response())
}

async fn last_operation(state: &AppState, did: &str) -> Result<Value, XrpcError> {
    let config = identity(state)?;
    let response = state
        .http()
        .get(format!("{}/{did}/log/last", config.plc_url))
        .send()
        .await
        .map_err(XrpcError::internal)?;
    if !response.status().is_success() {
        return Err(XrpcError::internal(format!(
            "the PLC directory answered {} for the last operation of {did}",
            response.status().as_u16()
        )));
    }
    serde_json::from_slice(&response.bytes().await.map_err(XrpcError::internal)?)
        .map_err(XrpcError::internal)
}

/// `com.atproto.identity.signPlcOperation`: the server signs an operation
/// that follows the last one, with the fields the caller gives.
pub async fn sign_plc_operation(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let did = full_or_takendown(state, headers)?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let Some(token) = input
        .get("token")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return Err(XrpcError::invalid_request(
            "email confirmation token required to sign PLC operations",
        ));
    };
    {
        let did = did.clone();
        state
            .blocking(move |state| {
                super::email::valid_token(state, "plc_operation", &token, Some(&did))?;
                let writes = state
                    .writes()
                    .ok_or_else(|| XrpcError::internal("the server is read-only"))?;
                writes.with_accounts(|db| db.delete_email_token(&did, "plc_operation"))?;
                Ok(())
            })
            .await?;
    }
    let last = last_operation(state, &did).await?;
    let signer = rotation_key(state)?;
    let operation = pds_identity::plc::update_op(&last, &signer, |fields| {
        for key in [
            "rotationKeys",
            "alsoKnownAs",
            "verificationMethods",
            "services",
        ] {
            if let Some(given) = input.get(key).filter(|given| !given.is_null())
                && let Ok(data) = Data::from_json_str(&given.to_string())
            {
                fields.insert(key.to_owned(), data);
            }
        }
    })
    .map_err(|error| match error {
        pds_identity::plc::UpdateError::Tombstone => {
            XrpcError::invalid_request("Did is tombstoned")
        }
        pds_identity::plc::UpdateError::Malformed => XrpcError::internal(error),
    })?;
    Ok(json_response(
        StatusCode::OK,
        &json!({ "operation": operation }),
    ))
}

/// Whether `op` has the shape of an operation of today's form.
fn is_operation(op: &Value) -> bool {
    let strings = |value: &Value| {
        value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string))
    };
    op["type"] == "plc_operation"
        && strings(&op["rotationKeys"])
        && strings(&op["alsoKnownAs"])
        && op["verificationMethods"]
            .as_object()
            .is_some_and(|keys| keys.values().all(Value::is_string))
        && op["services"].as_object().is_some_and(|services| {
            services
                .values()
                .all(|service| service["type"].is_string() && service["endpoint"].is_string())
        })
        && (op["prev"].is_null() || op["prev"].is_string())
        && op["sig"].is_string()
}

/// `com.atproto.identity.submitPlcOperation`: the server sends an
/// operation to the directory when it keeps the account on this server.
pub async fn submit_plc_operation(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let did = session(state, headers, None)?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let op = input.get("operation").cloned().unwrap_or(Value::Null);
    if !is_operation(&op) {
        return Err(XrpcError::invalid_request("Invalid operation"));
    }
    let config = identity(state)?.clone();
    let rotation = rotation_key(state)?.did_key();
    let refuse = |message: &str| Err(XrpcError::invalid_request(message));
    if !op["rotationKeys"]
        .as_array()
        .is_some_and(|keys| keys.iter().any(|key| key == rotation.as_str()))
    {
        return refuse("Rotation keys do not include server's rotation key");
    }
    let pds = &op["services"]["atproto_pds"];
    if pds["type"] != "AtprotoPersonalDataServer" {
        return refuse("Incorrect type on atproto_pds service");
    }
    if pds["endpoint"] != config.public_url.as_str() {
        return refuse("Incorrect endpoint on atproto_pds service");
    }
    if op["verificationMethods"]["atproto"] != signing_key(state, &did).await?.as_str() {
        return refuse("Incorrect signing key");
    }
    if let Some(handle) = handle_of(state, &did).await?
        && op["alsoKnownAs"][0] != format!("at://{handle}").as_str()
    {
        return refuse("Incorrect handle in alsoKnownAs");
    }
    let sent = state
        .http()
        .post(format!("{}/{did}", config.plc_url))
        .header("content-type", "application/json")
        .body(op.to_string())
        .send()
        .await
        .map_err(XrpcError::internal)?;
    if !sent.status().is_success() {
        return Err(XrpcError::internal(format!(
            "the PLC directory answered {} to an operation of {did}",
            sent.status().as_u16()
        )));
    }
    {
        let did = did.clone();
        state
            .blocking(move |state| {
                let writes = state
                    .writes()
                    .ok_or_else(|| XrpcError::internal("the server is read-only"))?;
                let now = pds_sequencer::iso_millis(state.now_ms());
                writes
                    .sequencer()
                    .sequence(&[identity_event(&did, None, &now)])?;
                Ok(())
            })
            .await?;
    }
    if let Ok(parsed) = did.parse::<Did>()
        && let Err(error) = state.resolver().resolve(&parsed, true).await
    {
        tracing::error!(%did, %error, "failed to refresh did after plc update");
    }
    Ok(StatusCode::OK.into_response())
}

/// `com.atproto.server.reserveSigningKey`: a signing key for an account
/// that will come here. A DID gets the same key each time.
pub async fn reserve_signing_key(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let did = body::string(&input, "did", false, crate::params::Format::Did, None, "$")?
        .map(str::to_owned);
    // The DID names a file. A DID has no path separator, and this one is
    // not taken on trust.
    if did
        .as_deref()
        .is_some_and(|did| did.starts_with('.') || did.contains(['/', '\\']))
    {
        return Err(XrpcError::invalid_request("unsafe path part"));
    }
    let writes = state
        .writes()
        .ok_or_else(|| XrpcError::internal("a read-only server reserves no key"))?;
    let keypair = {
        let mut draws = std::iter::repeat_with(|| writes.random());
        Keypair::generate(Algorithm::Secp256k1, |bytes| {
            for chunk in bytes.chunks_mut(8) {
                let draw = draws.next().unwrap_or_default().to_be_bytes();
                chunk.copy_from_slice(&draw[..chunk.len()]);
            }
        })
    };
    let signing_key = state
        .blocking(move |state| {
            let layout = state.layout();
            if let Some(did) = &did
                && let Ok(stored) = std::fs::read(layout.reserved_key(did))
            {
                return Ok(Keypair::from_secret_bytes(Algorithm::Secp256k1, &stored)
                    .map_err(XrpcError::internal)?
                    .did_key());
            }
            let key = keypair.did_key();
            let path = layout.reserved_key(did.as_deref().unwrap_or(&key));
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(XrpcError::internal)?;
            }
            std::fs::write(&path, keypair.secret_bytes().as_slice())
                .map_err(XrpcError::internal)?;
            Ok(key)
        })
        .await?;
    Ok(json_response(
        StatusCode::OK,
        &json!({ "signingKey": signing_key }),
    ))
}
