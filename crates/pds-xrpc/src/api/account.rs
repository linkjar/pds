//! `com.atproto.server.createAccount` (`docs/SPEC.md` §10.1).
//!
//! The checks, their order and their answers are those of
//! `createAccount.ts` at the pin for a server that is not behind an
//! entryway. Not built yet: an account that brings its DID (migration,
//! unit 8), the list of disposable email domains, and the tombstone of a
//! DID whose account rows could not be written.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use data_encoding::HEXLOWER;
use pds_identity::{Algorithm, Keypair, check_handle_policy, plc, reserved};
use pds_repo::{MemoryBlockStore, Repo};
use pds_sequencer::{account_event, commit_event, identity_event, sync_event};
use pds_storage::{AccountDb, ActorDb, Availability, NewAccount, Refused, schema};
use pds_types::{Did, Handle};
use serde_json::{Map, Value};

use crate::auth::{self, SCOPE_ACCESS};
use crate::body;
use crate::error::{XrpcError, json_response};
use crate::params::Format;
use crate::password;
use crate::state::{AppState, IdentityConfig, WriteState};

const DEFAULT_JSON_LIMIT: usize = 100 * 1024;

fn refuse(error: &str, message: impl Into<String>) -> XrpcError {
    XrpcError::bad_request(error, message)
}

/// Whether the server takes this email address. The Reference asks
/// `@hapi/address`; this is the common part of what that accepts.
pub(crate) fn email_is_valid(email: &str) -> bool {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+/=?^_`{|}~.-".contains(c));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = domain.len() <= 255
        && labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    local_ok && domain_ok && email.len() <= 254
}

/// A handle an account asks for, normalised and checked as
/// `normalizeAndValidateHandle` of the Reference checks it. Returns the
/// handle and whether a hosted domain holds it. A handle under another
/// domain passes only for an account that exists, and the caller then
/// checks that the handle resolves to the account.
pub(crate) fn checked_handle(
    state: &AppState,
    given: &str,
    has_did: bool,
    allow_reserved: bool,
) -> Result<(String, bool), XrpcError> {
    let handle = Handle::parse(&given.to_lowercase())
        .map_err(|error| refuse("InvalidHandle", error.to_string()))?
        .normalized();
    // The Reference at the pin accepts a handle under `.test` in every
    // mode, and `default/11-identity` verifies one as a custom domain, so
    // the rule for development does not apply here (SPEC §9.2).
    if check_handle_policy(&handle, true).is_err() {
        return Err(refuse(
            "InvalidHandle",
            "Handle TLD is invalid or disallowed",
        ));
    }
    let handle = handle.as_str().to_owned();
    let domains = &state.config().handle_domains;
    let Some(domain) = domains
        .iter()
        .find(|domain| handle.ends_with(domain.as_str()))
    else {
        if has_did {
            return Ok((handle, false));
        }
        return Err(refuse("UnsupportedDomain", "Not a supported handle domain"));
    };
    let front = &handle[..handle.len() - domain.len()];
    // A reservation of the profile holds for the admin too.
    if state.config().profile.reserves(domain, front) {
        return Err(refuse("HandleNotAvailable", "Reserved handle"));
    }
    if front.contains('.') {
        return Err(refuse("InvalidHandle", "Invalid characters in handle"));
    }
    if front.len() < 3 {
        return Err(refuse("InvalidHandle", "Handle too short"));
    }
    if front.len() > 18 {
        return Err(refuse("InvalidHandle", "Handle too long"));
    }
    if !allow_reserved && reserved::is_reserved(front) {
        return Err(refuse("HandleNotAvailable", "Reserved handle"));
    }
    Ok((handle, true))
}

fn random_bytes<const N: usize>(writes: &WriteState) -> [u8; N] {
    let mut bytes = [0_u8; N];
    for chunk in bytes.chunks_mut(8) {
        let draw = writes.random().to_be_bytes();
        chunk.copy_from_slice(&draw[..chunk.len()]);
    }
    bytes
}

/// What a request asked for, after the checks that need no write.
struct Request {
    handle: String,
    email: String,
    password: Option<String>,
    invite_code: Option<String>,
    recovery_key: Option<String>,
}

/// The checks of `validateInputsForLocalPds`, in its order.
fn validate(
    state: &AppState,
    input: &Map<String, Value>,
) -> Result<(Request, Option<String>), XrpcError> {
    let text = |key: &str, required: bool, format: Format| {
        body::string(input, key, required, format, None, "$").map(|value| value.map(str::to_owned))
    };
    let email = text("email", false, Format::Any)?;
    let handle = text("handle", true, Format::Any)?.unwrap_or_default();
    let did = text("did", false, Format::Did)?;
    let invite_code = text("inviteCode", false, Format::Any)?;
    text("verificationCode", false, Format::Any)?;
    text("verificationPhone", false, Format::Any)?;
    let given = text("password", false, Format::Any)?;
    let recovery_key = text("recoveryKey", false, Format::Any)?;
    if input.contains_key("plcOp") {
        return Err(XrpcError::invalid_request("Unsupported input: \"plcOp\""));
    }
    if given
        .as_ref()
        .is_some_and(|given| given.len() > password::NEW_PASSWORD_MAX_LENGTH)
    {
        return Err(XrpcError::invalid_request(format!(
            "Password too long. Maximum length is {} characters.",
            password::NEW_PASSWORD_MAX_LENGTH
        )));
    }
    let required = state.config().invite_required;
    if required && invite_code.is_none() {
        return Err(refuse("InvalidInviteCode", "No invite code provided"));
    }
    let Some(email) = email else {
        return Err(XrpcError::invalid_request("Email is required"));
    };
    if !email_is_valid(&email) {
        return Err(XrpcError::invalid_request(
            "This email address is not supported, please use a different email.",
        ));
    }
    identity(state)?;
    let (handle, _) = checked_handle(state, &handle, false, false)?;
    let accounts = AccountDb::open(&state.layout().account_db)?;
    if required
        && let Some(code) = &invite_code
        && !accounts.invite_available(code)?
    {
        return Err(refuse(
            "InvalidInviteCode",
            "Provided invite code not available",
        ));
    }
    // An account that is deactivated or taken down holds its handle and
    // its email address too (SPEC §2.3, DD-16 and DD-17).
    if accounts
        .account(&handle, false, Availability::ANY)?
        .is_some()
    {
        return Err(XrpcError::invalid_request(format!(
            "Handle already taken: {handle}"
        )));
    }
    if accounts
        .account(&email.to_lowercase(), true, Availability::ANY)?
        .is_some()
    {
        return Err(XrpcError::invalid_request(format!(
            "Email already taken: {email}"
        )));
    }
    Ok((
        Request {
            handle,
            email,
            password: given,
            invite_code: invite_code.filter(|_| required),
            recovery_key,
        },
        did,
    ))
}

pub(crate) fn identity(state: &AppState) -> Result<&IdentityConfig, XrpcError> {
    state
        .config()
        .identity
        .as_ref()
        .ok_or_else(|| XrpcError::internal("the PLC rotation key is not configured"))
}

fn writes(state: &AppState) -> Result<&WriteState, XrpcError> {
    state
        .writes()
        .ok_or_else(|| XrpcError::internal("the server is read-only"))
}

/// The actor store of a new account: its directory, its signing key, and
/// a repository with an empty first commit. Returns the commit.
fn create_store(
    state: &AppState,
    did: &str,
    keypair: &Keypair,
    now: &str,
) -> Result<pds_repo::CommitData, XrpcError> {
    let layout = state.layout();
    let parsed: Did = did.parse().map_err(XrpcError::internal)?;
    let dir = layout.actor_store_dir(did);
    if layout.actor_db(did).exists() {
        return Err(XrpcError::invalid_request("Repo already exists"));
    }
    std::fs::create_dir_all(&dir).map_err(XrpcError::internal)?;
    std::fs::write(layout.actor_key(did), keypair.secret_bytes().as_slice())
        .map_err(XrpcError::internal)?;
    let rev = writes(state)?.tids().next();
    let commit = Repo::format_init_commit(MemoryBlockStore::new(), &parsed, rev, &[], |bytes| {
        Ok(keypair.sign(bytes).to_vec())
    })
    .map_err(XrpcError::internal)?;
    drop(schema::create_actor_store(&layout.actor_db(did), now)?);
    let mut store = ActorDb::open_writable(&layout.actor_db(did))?;
    let txn = store.begin()?;
    txn.update_root(did, &commit.cid, commit.rev.as_str(), now, true)?;
    txn.put_blocks(&commit.new_blocks, commit.rev.as_str())?;
    txn.commit()?;
    Ok(commit)
}

fn destroy_store(state: &AppState, did: &str) {
    let dir = state.layout().actor_store_dir(did);
    if let Err(error) = std::fs::remove_dir_all(&dir) {
        tracing::error!(%did, %error, "the actor store of a failed creation was not removed");
    }
}

/// `com.atproto.server.createAccount`.
pub async fn create_account(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    crate::ratelimit::route(
        state,
        &[(
            crate::ratelimit::of_route("com.atproto.server.createAccount-0", 300, 100),
            crate::ratelimit::Key::Ip,
            1,
        )],
    )?;
    let (request, brought_did) = {
        let input = input.clone();
        state.blocking(move |state| validate(state, &input)).await?
    };
    // Service auth is not built yet, so nobody can bring a DID.
    if let Some(did) = brought_did {
        return Err(XrpcError {
            status: StatusCode::UNAUTHORIZED,
            error: "AuthenticationRequired".to_owned(),
            message: format!("Missing auth to create account with did: {did}"),
        });
    }
    let identity = identity(state)?.clone();
    let writes = writes(state)?;
    let signer = Keypair::from_secret_bytes(Algorithm::Secp256k1, &identity.rotation_key)
        .map_err(XrpcError::internal)?;
    let keypair = {
        let mut draws = std::iter::repeat_with(|| writes.random());
        Keypair::generate(Algorithm::Secp256k1, |bytes| {
            for chunk in bytes.chunks_mut(8) {
                let draw = draws.next().unwrap_or_default().to_be_bytes();
                chunk.copy_from_slice(&draw[..chunk.len()]);
            }
        })
    };
    // The rotation keys in order of authority: the account's own recovery
    // key, the operator's, then the server's.
    let mut rotation_keys = vec![signer.did_key()];
    if let Some(recovery) = &identity.recovery_did_key {
        rotation_keys.insert(0, recovery.clone());
    }
    if let Some(recovery) = &request.recovery_key {
        rotation_keys.insert(0, recovery.clone());
    }
    let genesis = plc::create_op(
        &keypair.did_key(),
        &rotation_keys,
        &request.handle,
        &identity.public_url,
        &signer,
    );
    let did = genesis.did.clone();
    let now = pds_sequencer::iso_millis(state.now_ms());

    let commit = {
        let (did, now) = (did.clone(), now.clone());
        state
            .blocking(move |state| create_store(state, &did, &keypair, &now))
            .await?
    };
    let created = finish(state, &identity, &request, &genesis, &commit, &now).await;
    if created.is_err() {
        let did = did.clone();
        let _ = state
            .blocking(move |state| {
                destroy_store(state, &did);
                Ok(())
            })
            .await;
    }
    created
}

/// The steps after the actor store exists: the DID, the account rows, the
/// events and the answer.
async fn finish(
    state: &AppState,
    identity: &IdentityConfig,
    request: &Request,
    genesis: &plc::Genesis,
    commit: &pds_repo::CommitData,
    now: &str,
) -> Result<Response, XrpcError> {
    let did = genesis.did.as_str();
    let sent = state
        .http()
        .post(format!("{}/{did}", identity.plc_url))
        .header("content-type", "application/json")
        .body(genesis.operation.to_string())
        .send()
        .await
        .map_err(XrpcError::internal)?;
    if !sent.status().is_success() {
        return Err(XrpcError::internal(format!(
            "the PLC directory answered {} to the creation of {did}",
            sent.status().as_u16()
        )));
    }
    let parsed: Did = did.parse().map_err(XrpcError::internal)?;
    let did_doc = match state.resolver().resolve(&parsed, true).await {
        Ok(doc) => doc,
        Err(error) => {
            tracing::warn!(%did, %error, "failed to resolve did doc");
            None
        }
    };

    let writes = writes(state)?;
    let config = &state.config().auth;
    let now_s = state.now_s();
    let access = auth::mint_access(config, did, SCOPE_ACCESS, now_s);
    let id = data_encoding::BASE64_NOPAD.encode(&random_bytes::<32>(writes));
    let (refresh, expires_s) = auth::mint_refresh(config, did, &id, now_s);
    let expires_at = pds_sequencer::iso_millis(expires_s * 1000);
    let salt = HEXLOWER.encode(&random_bytes::<16>(writes));

    let stored = {
        let (did, now, handle) = (did.to_owned(), now.to_owned(), request.handle.clone());
        let (email, given) = (request.email.clone(), request.password.clone());
        let invite_code = request.invite_code.clone();
        let (cid, rev) = (commit.cid.to_string(), commit.rev.as_str().to_owned());
        state
            .blocking(move |state| {
                let hash = match &given {
                    Some(given) => {
                        Some(password::hash_with_salt(given, &salt).ok_or_else(|| {
                            XrpcError::internal("the password could not be hashed")
                        })?)
                    }
                    None => None,
                };
                let new = NewAccount {
                    did: &did,
                    handle: &handle,
                    credentials: hash.as_deref().map(|hash| (email.as_str(), hash)),
                    invite_code: invite_code.as_deref(),
                    refresh_token: Some((&id, &expires_at)),
                    repo_root: (&cid, &rev),
                    now: &now,
                    delete_after: None,
                };
                Ok(self::writes(state)?.with_accounts(|accounts| accounts.create_account(&new))?)
            })
            .await?
    };
    match stored {
        Ok(()) => {}
        Err(Refused::Invite) => {
            return Err(refuse(
                "InvalidInviteCode",
                "Provided invite code not available",
            ));
        }
        Err(Refused::Exists) => return Err(XrpcError::invalid_request("User already exists")),
    }

    let block = commit
        .new_blocks
        .get(&commit.cid)
        .ok_or_else(|| XrpcError::internal("the first commit has no block"))?;
    let events = [
        identity_event(did, Some(&request.handle), now),
        account_event(did, None, now),
        commit_event(did, commit, &[], now),
        sync_event(did, commit.rev.as_str(), &commit.cid, block, now),
    ];
    if let Err(error) = writes.sequencer().sequence(&events) {
        let did = did.to_owned();
        let _ = state
            .blocking(move |state| {
                Ok(self::writes(state)?.with_accounts(|accounts| accounts.delete_account(&did))?)
            })
            .await;
        return Err(error.into());
    }

    let mut out = Map::new();
    out.insert("handle".to_owned(), request.handle.clone().into());
    out.insert("did".to_owned(), did.into());
    if let Some(doc) = did_doc {
        out.insert("didDoc".to_owned(), doc);
    }
    out.insert("accessJwt".to_owned(), access.into());
    out.insert("refreshJwt".to_owned(), refresh.into());
    Ok(json_response(StatusCode::OK, &out.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_email_address_is_checked() {
        for good in [
            "a@b.co",
            "first.last+tag@sub.example.org",
            "x_y@ex-ample.io",
        ] {
            assert!(email_is_valid(good), "{good}");
        }
        for bad in [
            "",
            "a",
            "a@",
            "@b.co",
            "a@b",
            "a b@c.de",
            "a@b..co",
            ".a@b.co",
            "a.@b.co",
            "a..b@c.de",
            "a@-b.co",
            "a@b.co-",
            "a@@b.co",
        ] {
            assert!(!email_is_valid(bad), "{bad}");
        }
    }
}
