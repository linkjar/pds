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

/// An identity at an identity provider that a new account is created for
/// (SPEC §12.3).
#[derive(Debug, Clone)]
pub(crate) struct External {
    pub(crate) provider: String,
    pub(crate) subject: String,
    pub(crate) display_name: Option<String>,
}

/// What a request asked for, after the checks that need no write.
#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub(crate) handle: String,
    pub(crate) email: String,
    pub(crate) password: Option<String>,
    pub(crate) invite_code: Option<String>,
    pub(crate) recovery_key: Option<String>,
    /// The identity that proved the email address. The account then has
    /// no password, a confirmed address and, in a profile that keeps one,
    /// a handle policy row. No request field sets this (EX-3).
    pub(crate) external: Option<External>,
    /// Whether the creation opens a legacy session. A sign-up on the
    /// authorization pages opens none.
    pub(crate) legacy_session: bool,
    /// The authorization the account is created in: the pushed request,
    /// its client and the device. The server passes it; no request field
    /// sets it (EX-3).
    pub(crate) signup: Option<(String, String, String)>,
    /// The DID of an account that moves here (SPEC §10.3). Its service
    /// token proved that the caller controls it. The account starts
    /// deactivated, the server writes nothing to the PLC directory, and
    /// no event goes out until the account is activated.
    pub(crate) brought_did: Option<String>,
}

/// A new account.
#[derive(Debug, Clone)]
pub(crate) struct Created {
    pub(crate) did: String,
    pub(crate) handle: String,
    pub(crate) did_doc: Option<Value>,
    /// The access and the refresh token of the legacy session.
    pub(crate) session: Option<(String, String)>,
}

/// The checks of `validateInputsForLocalPds`, in its order.
pub(crate) fn validate(
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
    // The operator's list of blocked email domains (SPEC §12.2, §22.1).
    if let Some(writes) = state.writes()
        && writes.with_ops(|ops| ops.is_email_blocked(&email))?
    {
        return Err(XrpcError::invalid_request(
            "This email address is not supported, please use a different email.",
        ));
    }
    identity(state)?;
    let (handle, _) = checked_handle(state, &handle, did.is_some(), false)?;
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
            external: None,
            legacy_session: true,
            signup: None,
            brought_did: None,
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
    // A service token of an account names the DID that the caller may
    // bring: the account moves here from another server (SPEC §10.3).
    let requester = if crate::service_auth::is_service_token(headers) {
        Some(crate::service_auth::verify(state, headers, "com.atproto.server.createAccount").await?)
    } else {
        None
    };
    let named = input.get("did").and_then(Value::as_str);
    // The legacy method cannot carry the proof of a CAPTCHA. With one
    // configured, a new account comes through the sign-up page (SPEC
    // §12.2, patch 099c). An account that moves here proved control of
    // its DID and passes.
    if state.config().profile.guards_signup()
        && state.config().oauth.hcaptcha.is_some()
        && (named.is_none() || named != requester.as_deref())
    {
        return Err(XrpcError::invalid_request(
            "Create an account through the OAuth signup page",
        ));
    }
    let (mut request, brought_did) = {
        let input = input.clone();
        state.blocking(move |state| validate(state, &input)).await?
    };
    if let Some(did) = brought_did {
        if requester.as_deref() != Some(did.as_str()) {
            return Err(XrpcError {
                status: StatusCode::UNAUTHORIZED,
                error: "AuthenticationRequired".to_owned(),
                message: format!("Missing auth to create account with did: {did}"),
            });
        }
        // A handle under another domain must name the DID already.
        if !state.config().is_hosted_handle(&request.handle) {
            super::lifecycle::assert_handle_resolves(state, &request.handle, &did).await?;
        }
        request.brought_did = Some(did);
    }
    let created = create(state, request).await?;
    crate::audit::set_subject(&created.did);
    let mut out = Map::new();
    out.insert("handle".to_owned(), created.handle.into());
    out.insert("did".to_owned(), created.did.into());
    if let Some(doc) = created.did_doc {
        out.insert("didDoc".to_owned(), doc);
    }
    if let Some((access, refresh)) = created.session {
        out.insert("accessJwt".to_owned(), access.into());
        out.insert("refreshJwt".to_owned(), refresh.into());
    }
    Ok(json_response(StatusCode::OK, &out.into()))
}

/// Creates the account of a checked request: the identity, the store, the
/// rows and the events.
pub(crate) async fn create(state: &AppState, request: Request) -> Result<Created, XrpcError> {
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
    // An account that brings its DID gets no operation from here: the
    // account writes its own when it is ready to move.
    let genesis = request.brought_did.is_none().then(|| {
        plc::create_op(
            &keypair.did_key(),
            &rotation_keys,
            &request.handle,
            &identity.public_url,
            &signer,
        )
    });
    let did = match (&genesis, &request.brought_did) {
        (Some(genesis), _) => genesis.did.clone(),
        (None, Some(did)) => did.clone(),
        (None, None) => return Err(XrpcError::internal("an account without a DID")),
    };
    let now = pds_sequencer::iso_millis(state.now_ms());

    let commit = {
        let (did, now) = (did.clone(), now.clone());
        state
            .blocking(move |state| create_store(state, &did, &keypair, &now))
            .await?
    };
    let created = finish(
        state,
        &identity,
        &request,
        &did,
        genesis.as_ref(),
        &commit,
        &now,
    )
    .await;
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

/// Writes the rows of a new account in one transaction. `root` is the CID
/// and the revision of the first commit; `session` is the id of the first
/// refresh token and when it ends.
fn store_rows(
    state: &AppState,
    request: &Request,
    did: &str,
    now: &str,
    root: &(String, String),
    session: &(String, String),
    salt: &str,
) -> Result<Result<(), Refused>, XrpcError> {
    let external = request.external.as_ref();
    // An account that an identity created has no password to match.
    let hash = match (&request.password, external) {
        (_, Some(_)) => Some(password::EXTERNAL_IDENTITY.to_owned()),
        (Some(given), None) => Some(
            password::hash_with_salt(given, salt)
                .ok_or_else(|| XrpcError::internal("the password could not be hashed"))?,
        ),
        (None, None) => None,
    };
    let email = request.email.as_str();
    let new = NewAccount {
        did,
        handle: &request.handle,
        credentials: hash.as_deref().map(|hash| (email, hash)),
        external_identity: external.map(|external| pds_storage::linkjar::NewIdentity {
            provider: &external.provider,
            subject: &external.subject,
            email: Some(email),
            email_verified: true,
            display_name: external.display_name.as_deref(),
        }),
        handle_policy: external.is_some() && state.config().profile.keeps_handle_policy(),
        signup: request
            .signup
            .as_ref()
            .filter(|_| state.config().profile.keeps_signup_receipts())
            .map(|(request, client, device)| (request.as_str(), client.as_str(), device.as_str())),
        invite_code: request.invite_code.as_deref(),
        refresh_token: request
            .legacy_session
            .then_some((session.0.as_str(), session.1.as_str())),
        repo_root: (&root.0, &root.1),
        now,
        delete_after: None,
        deactivated: request.brought_did.is_some(),
    };
    Ok(writes(state)?.with_accounts(|accounts| accounts.create_account(&new))?)
}

/// The steps after the actor store exists: the DID, the account rows, the
/// events and the answer.
async fn finish(
    state: &AppState,
    identity: &IdentityConfig,
    request: &Request,
    did: &str,
    genesis: Option<&plc::Genesis>,
    commit: &pds_repo::CommitData,
    now: &str,
) -> Result<Created, XrpcError> {
    if let Some(genesis) = genesis {
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
        let request = request.clone();
        let (did, now) = (did.to_owned(), now.to_owned());
        let root = (commit.cid.to_string(), commit.rev.as_str().to_owned());
        let session = (id, expires_at);
        state
            .blocking(move |state| store_rows(state, &request, &did, &now, &root, &session, &salt))
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
        Err(Refused::Authorization) => {
            return Err(XrpcError::invalid_request(
                "Account creation authorization is no longer available",
            ));
        }
    }

    let block = commit
        .new_blocks
        .get(&commit.cid)
        .ok_or_else(|| XrpcError::internal("the first commit has no block"))?;
    // An account that moved here says nothing on the firehose until it is
    // activated.
    if request.brought_did.is_none() {
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
                    Ok(self::writes(state)?
                        .with_accounts(|accounts| accounts.delete_account(&did))?)
                })
                .await;
            return Err(error.into());
        }
    }

    Ok(Created {
        did: did.to_owned(),
        handle: request.handle.clone(),
        did_doc,
        session: request.legacy_session.then_some((access, refresh)),
    })
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
