//! The ops listener (`docs/SPEC.md` §22.1): what an operator sees of the
//! server and the actions an operator takes, as `internal.ops.*` methods
//! on a private address. The console sidecar is its client.
//!
//! An operator is the admin credential, or an account with a role that
//! signed in to the console through this server's OAuth pages. Every
//! action carries a reason in the `X-Ops-Reason` header and writes an
//! audit entry that names the operator, the reason and the console.
//!
//! The admin methods of the public listener are served here too, for an
//! operator: the listener runs them with the admin credential, and the
//! audit entry names the operator.

use axum::body::Body;
use axum::extract::FromRequestParts as _;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use pds_storage::account::Availability;
use pds_storage::{ActorDb, BlockedEmailDomain, OperatorRole};
use serde_json::{Map, Value, json};

use crate::audit::Acting;
use crate::auth::{Credentials, optional_credentials};
use crate::error::{XrpcError, json_response};
use crate::oauth::resource;
use crate::state::AppState;

/// The header with the reason of an action.
pub const REASON_HEADER: &str = "x-ops-reason";

/// The longest reason, in bytes.
const REASON_LIMIT: usize = 500;

/// The largest body of an `internal.ops.*` action.
const BODY_LIMIT: usize = 16 * 1024;

/// How often the live stream sends the state.
const STREAM_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// What an operator may do (SPEC §12.11). Each role may do what the one
/// before it may.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Reads, and the actions on one account or one subject.
    Moderator,
    /// Every action but the grant of a role.
    Operator,
    /// The admin credential.
    Admin,
}

impl Role {
    /// The role of a name in `ops.sqlite`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "moderator" => Some(Self::Moderator),
            "operator" => Some(Self::Operator),
            _ => None,
        }
    }

    /// The name of the role.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Moderator => "moderator",
            Self::Operator => "operator",
            Self::Admin => "admin",
        }
    }
}

/// Who calls the ops listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operator {
    /// A DID, or `admin`.
    pub actor: String,
    /// What the operator may do.
    pub role: Role,
    /// The OAuth client of the session.
    pub client_id: Option<String>,
}

fn refuse(status: StatusCode, error: &str, message: &str) -> Response {
    json_response(status, &json!({ "error": error, "message": message }))
}

fn forbidden(message: &str) -> Response {
    refuse(StatusCode::FORBIDDEN, "Forbidden", message)
}

/// The operator of a request: the admin credential, or the OAuth session
/// of an account with a role, from a client the configuration names.
pub async fn authenticate(
    state: &AppState,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
) -> Result<Operator, Box<Response>> {
    if !resource::is_dpop(headers) {
        let auth = &state.config().auth;
        return match optional_credentials(headers, auth, &[], state.now_s()) {
            Ok(Credentials::Admin) => Ok(Operator {
                actor: "admin".to_owned(),
                role: Role::Admin,
                client_id: None,
            }),
            _ => Err(Box::new(
                XrpcError::auth_required("AuthenticationRequired").into_response(),
            )),
        };
    }
    let grant = resource::authenticate(state, method, path, headers).await?;
    let config = state.config();
    if !config.profile.grants_operator_roles() {
        return Err(Box::new(forbidden("this server grants no operator role")));
    }
    if !config.ops_client_ids.contains(&grant.client_id) {
        return Err(Box::new(forbidden("the session is not one of the console")));
    }
    let did = grant.did.clone();
    let role = state
        .blocking(move |state| {
            let writes = state
                .writes()
                .ok_or_else(|| XrpcError::internal("a read-only server has no operator"))?;
            Ok(writes.with_ops(|ops| ops.role(&did))?)
        })
        .await
        .map_err(|error| Box::new(error.into_response()))?;
    match role.as_deref().and_then(Role::parse) {
        Some(role) => Ok(Operator {
            actor: grant.did,
            role,
            client_id: Some(grant.client_id),
        }),
        None => Err(Box::new(forbidden("the account holds no operator role"))),
    }
}

/// Adds the DPoP nonce to the answer of a request with an OAuth token.
pub fn finish(state: &AppState, headers: &HeaderMap, response: Response) -> Response {
    if resource::is_dpop(headers) {
        crate::oauth::with_nonce(state, response)
    } else {
        response
    }
}

/// Whether the ops listener serves a method of the public listener with
/// the admin credential.
fn is_admin_method(nsid: &str) -> bool {
    (nsid.starts_with("com.atproto.admin.")
        || matches!(
            nsid,
            "com.atproto.server.createInviteCode" | "com.atproto.server.createInviteCodes"
        ))
        && crate::api::is_handled(nsid)
}

/// The least role that may take an action.
fn needs(nsid: &str) -> Role {
    match nsid {
        "internal.ops.grantRole" | "internal.ops.revokeRole" => Role::Admin,
        "internal.ops.revokeSessions"
        | "com.atproto.admin.updateSubjectStatus"
        | "com.atproto.admin.disableAccountInvites"
        | "com.atproto.admin.enableAccountInvites"
        | "com.atproto.admin.disableInviteCodes" => Role::Moderator,
        _ => Role::Operator,
    }
}

/// The reason of an action: required, one line, at most 500 bytes.
fn reason(headers: &HeaderMap) -> Result<String, Box<Response>> {
    let given = headers
        .get(REASON_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or_default();
    if given.is_empty() {
        return Err(Box::new(
            XrpcError::invalid_request("An action needs a reason in the X-Ops-Reason header")
                .into_response(),
        ));
    }
    if given.len() > REASON_LIMIT {
        return Err(Box::new(
            XrpcError::invalid_request("The reason is longer than 500 bytes").into_response(),
        ));
    }
    Ok(given.to_owned())
}

/// Every request to the ops listener enters here.
pub async fn serve(State(state): State<AppState>, request: axum::extract::Request) -> Response {
    let (mut parts, body) = request.into_parts();
    let headers = parts.headers.clone();
    let path = parts.uri.path().to_owned();
    let Some(nsid) = path.strip_prefix("/xrpc/") else {
        return refuse(StatusCode::NOT_FOUND, "NotFound", "no such route");
    };
    let method = parts.method.clone();
    let operator = match authenticate(&state, &method, &path, &headers).await {
        Ok(operator) => operator,
        Err(refusal) => return finish(&state, &headers, *refusal),
    };
    if nsid == "internal.ops.subscribeState"
        && let Ok(upgrade) = WebSocketUpgrade::from_request_parts(&mut parts, &()).await
    {
        let state = state.clone();
        return upgrade.on_upgrade(move |socket| stream(socket, state));
    }
    let query = parts.uri.query().map(str::to_owned);
    let response = if method == Method::GET || method == Method::HEAD {
        if is_admin_method(nsid) {
            forward(
                &state,
                &operator,
                None,
                nsid,
                &method,
                query.as_deref(),
                &headers,
                body,
            )
            .await
        } else {
            read(&state, &operator, nsid, query.as_deref()).await
        }
    } else if method == Method::POST {
        Box::pin(act(
            &state,
            &operator,
            nsid,
            query.as_deref(),
            &headers,
            body,
        ))
        .await
    } else {
        XrpcError::invalid_request("XRPC requests only supports GET and POST").into_response()
    };
    finish(&state, &headers, response)
}

/// Takes an action: checks the role and the reason, runs it, and writes
/// the audit entry.
async fn act(
    state: &AppState,
    operator: &Operator,
    nsid: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    if operator.role < needs(nsid) {
        return forbidden("the role of the operator does not allow this action");
    }
    let reason = match reason(headers) {
        Ok(reason) => reason,
        Err(refusal) => return *refusal,
    };
    if is_admin_method(nsid) {
        let reason = Some(reason);
        return forward(
            state,
            operator,
            reason,
            nsid,
            &Method::POST,
            query,
            headers,
            body,
        )
        .await;
    }
    let acting = acting(operator, Some(reason));
    let input = match axum::body::to_bytes(body, BODY_LIMIT).await {
        Ok(bytes) if bytes.is_empty() => Map::new(),
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(input)) => input,
            _ => {
                return XrpcError::invalid_request("The body is not a JSON object").into_response();
            }
        },
        Err(_) => return XrpcError::invalid_request("The body is too large").into_response(),
    };
    let name = nsid.to_owned();
    let who = acting.clone();
    let done = state
        .blocking(move |state| Ok(action(state, &who, &name, &input)))
        .await;
    let (subject, result) = match done {
        Ok(done) => done,
        Err(error) => (None, Err(error)),
    };
    let known = !matches!(&result, Err(error) if error.status == StatusCode::NOT_IMPLEMENTED);
    if known {
        let outcome = match &result {
            Ok(_) => "ok".to_owned(),
            Err(refusal) => refusal.error.clone(),
        };
        crate::audit::record(state, acting.draft(nsid, subject, outcome)).await;
    }
    match result {
        Ok(output) => json_response(StatusCode::OK, &output),
        Err(refusal) => refusal.into_response(),
    }
}

fn acting(operator: &Operator, reason: Option<String>) -> Acting {
    Acting {
        actor: operator.actor.clone(),
        credential: if operator.client_id.is_some() {
            "oauth"
        } else {
            "admin"
        },
        client_id: operator.client_id.clone(),
        reason,
    }
}

/// Runs a method of the public listener with the admin credential. The
/// audit entry names the operator.
#[allow(clippy::too_many_arguments)]
async fn forward(
    state: &AppState,
    operator: &Operator,
    reason: Option<String>,
    nsid: &str,
    method: &Method,
    query: Option<&str>,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let mut headers = headers.clone();
    headers.remove("dpop");
    let admin = format!("admin:{}", state.config().auth.admin_password);
    let Ok(value) = HeaderValue::from_str(&format!("Basic {}", STANDARD.encode(admin))) else {
        return XrpcError::internal("the admin credential is not a header value").into_response();
    };
    headers.insert(header::AUTHORIZATION, value);
    let who = acting(operator, reason);
    let mut body = Some(body);
    let run = async {
        let response = Box::pin(crate::api::call(
            state, nsid, method, query, &headers, &mut body, None,
        ))
        .await;
        if let Some(unread) = body.take() {
            crate::body::drain(unread).await;
        }
        response
    };
    let run = crate::audit::acting_as(who, run);
    Box::pin(crate::ratelimit::scope(state, &headers, None, run)).await
}

fn text<'a>(input: &'a Map<String, Value>, key: &str) -> Result<&'a str, XrpcError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            XrpcError::invalid_request(format!("Input must have the property \"{key}\""))
        })
}

fn did_of(input: &Map<String, Value>) -> Result<String, XrpcError> {
    let did = text(input, "did")?;
    if !did.starts_with("did:") {
        return Err(XrpcError::invalid_request("\"did\" is not a DID"));
    }
    Ok(did.to_owned())
}

/// A domain as the block list keeps it: lower case, with a dot, without
/// the `@` of an address.
fn domain_of(input: &Map<String, Value>) -> Result<String, XrpcError> {
    let domain = text(input, "domain")?.to_ascii_lowercase();
    let shape_ok = domain.contains('.')
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if !shape_ok {
        return Err(XrpcError::invalid_request(
            "\"domain\" is not a domain name",
        ));
    }
    Ok(domain)
}

type Done = (Option<String>, Result<Value, XrpcError>);

/// Runs an `internal.ops.*` action, and names its subject. This call
/// blocks.
fn action(state: &AppState, who: &Acting, nsid: &str, input: &Map<String, Value>) -> Done {
    let subject = input
        .get("did")
        .or_else(|| input.get("domain"))
        .or_else(|| input.get("id"))
        .map(|value| {
            value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned)
        });
    let result = match nsid {
        "internal.ops.disconnectSubscriber" => disconnect(state, input),
        "internal.ops.revokeSessions" => revoke_sessions(state, input),
        "internal.ops.grantRole" => grant_role(state, who, input),
        "internal.ops.revokeRole" => revoke_role(state, input),
        "internal.ops.blockEmailDomain" => block_domain(state, who, input),
        "internal.ops.unblockEmailDomain" => unblock_domain(state, input),
        "internal.ops.emitSync" => emit_sync(state, input),
        "internal.ops.purgeExpired" => purge(state),
        "internal.ops.checkpoint" => checkpoint(state),
        _ => Err(XrpcError::not_implemented(nsid)),
    };
    (subject, result)
}

fn writes(state: &AppState) -> Result<&crate::state::WriteState, XrpcError> {
    state
        .writes()
        .ok_or_else(|| XrpcError::internal("a read-only server takes no action"))
}

fn disconnect(state: &AppState, input: &Map<String, Value>) -> Result<Value, XrpcError> {
    let id = input
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| XrpcError::invalid_request("Input must have the property \"id\""))?;
    if !state.subscribers().disconnect(id) {
        return Err(XrpcError::bad_request(
            "SubscriberNotFound",
            "No subscriber with this id is connected",
        ));
    }
    Ok(json!({}))
}

fn revoke_sessions(state: &AppState, input: &Map<String, Value>) -> Result<Value, XrpcError> {
    let did = did_of(input)?;
    let (legacy, oauth) = writes(state)?.with_accounts(|accounts| {
        let oauth = accounts.account_tokens(&did)?.len();
        let legacy = accounts.revoke_refresh_tokens(&did)?;
        accounts.delete_account_tokens(&did)?;
        for (device, _) in accounts.account_devices(&did)? {
            accounts.remove_device_account(&device.id, &did)?;
        }
        Ok::<_, pds_storage::StorageError>((legacy, oauth))
    })?;
    Ok(json!({ "legacySessions": legacy, "oauthSessions": oauth }))
}

fn grant_role(
    state: &AppState,
    who: &Acting,
    input: &Map<String, Value>,
) -> Result<Value, XrpcError> {
    let did = did_of(input)?;
    let role = text(input, "role")?;
    if Role::parse(role).is_none() {
        return Err(XrpcError::invalid_request(
            "\"role\" must be \"operator\" or \"moderator\"",
        ));
    }
    let writes = writes(state)?;
    let hosted = writes
        .with_accounts(|accounts| accounts.actor(&did, Availability::ANY))?
        .is_some();
    if !hosted {
        return Err(XrpcError::bad_request(
            "AccountNotFound",
            "The server hosts no account with this DID",
        ));
    }
    let row = OperatorRole {
        did,
        role: role.to_owned(),
        granted_by: who.actor.clone(),
        granted_at: pds_types::iso_millis(state.now_ms()),
    };
    writes.with_ops(|ops| ops.grant_role(&row))?;
    Ok(json!({}))
}

fn revoke_role(state: &AppState, input: &Map<String, Value>) -> Result<Value, XrpcError> {
    let did = did_of(input)?;
    if !writes(state)?.with_ops(|ops| ops.revoke_role(&did))? {
        return Err(XrpcError::bad_request(
            "RoleNotFound",
            "The account holds no role",
        ));
    }
    Ok(json!({}))
}

fn block_domain(
    state: &AppState,
    who: &Acting,
    input: &Map<String, Value>,
) -> Result<Value, XrpcError> {
    let row = BlockedEmailDomain {
        domain: domain_of(input)?,
        reason: who.reason.clone().unwrap_or_default(),
        created_by: who.actor.clone(),
        created_at: pds_types::iso_millis(state.now_ms()),
    };
    writes(state)?.with_ops(|ops| ops.block_email_domain(&row))?;
    Ok(json!({}))
}

fn unblock_domain(state: &AppState, input: &Map<String, Value>) -> Result<Value, XrpcError> {
    let domain = domain_of(input)?;
    if !writes(state)?.with_ops(|ops| ops.unblock_email_domain(&domain))? {
        return Err(XrpcError::bad_request(
            "DomainNotFound",
            "The domain is not blocked",
        ));
    }
    Ok(json!({}))
}

fn emit_sync(state: &AppState, input: &Map<String, Value>) -> Result<Value, XrpcError> {
    let did = did_of(input)?;
    match crate::write::emit_sync(state, &did)? {
        Some(seq) => Ok(json!({ "seq": seq })),
        None => Err(XrpcError::bad_request(
            "RepoNotFound",
            "The server holds no repository of this DID",
        )),
    }
}

fn purge(state: &AppState) -> Result<Value, XrpcError> {
    let now = pds_types::iso_millis(state.now_ms());
    let removed = writes(state)?.with_accounts(|accounts| accounts.purge_expired(&now))?;
    Ok(json!({ "removed": removed }))
}

fn checkpoint(state: &AppState) -> Result<Value, XrpcError> {
    let log = state
        .audit()
        .ok_or_else(|| XrpcError::internal("the server has no audit log"))?;
    let time = pds_types::iso_millis(state.now_ms());
    let made = log.checkpoint(&time)?;
    Ok(json!({ "last": made.map(|checkpoint| checkpoint.last) }))
}

/// A parameter of the query.
fn param(query: Option<&str>, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(key, value)| key == name && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

fn limit(query: Option<&str>, default: u32, most: u32) -> u32 {
    param(query, "limit")
        .and_then(|value| value.parse().ok())
        .map_or(default, |value: u32| value.clamp(1, most))
}

/// Answers a read. Every role may read.
async fn read(state: &AppState, operator: &Operator, nsid: &str, query: Option<&str>) -> Response {
    if nsid == "internal.ops.getOperator" {
        let body = json!({ "actor": operator.actor, "role": operator.role.name() });
        return json_response(StatusCode::OK, &body);
    }
    let name = nsid.to_owned();
    let query = query.map(str::to_owned);
    let result = state
        .blocking(move |state| {
            let query = query.as_deref();
            match name.as_str() {
                "internal.ops.getOverview" => overview(state),
                "internal.ops.listSubscribers" => Ok(json!({ "subscribers": subscribers(state) })),
                "internal.ops.listAccounts" => list_accounts(state, query),
                "internal.ops.getAccount" => account(state, query),
                "internal.ops.listAudit" => list_audit(state, query),
                "internal.ops.listRoles" => list_roles(state),
                "internal.ops.listBlockedEmailDomains" => list_domains(state),
                _ => Err(XrpcError::not_implemented(&name)),
            }
        })
        .await;
    match result {
        Ok(output) => json_response(StatusCode::OK, &output),
        Err(refusal) => refusal.into_response(),
    }
}

fn subscribers(state: &AppState) -> Vec<Value> {
    let head = state.writes().map_or(0, |writes| writes.sequencer().head());
    state.subscribers().list(head)
}

/// The state of the server at a glance. This call blocks.
pub fn overview(state: &AppState) -> Result<Value, XrpcError> {
    let config = state.config();
    let writes = writes(state)?;
    let accounts = writes.with_accounts(|accounts| accounts.count_actors())?;
    let pending = writes.with_ops(pds_storage::OpsDb::pending_commits)?.len();
    let notices = if config.profile.produces_notices() {
        Some(writes.with_accounts(|accounts| accounts.notice_count())?)
    } else {
        None
    };
    let audit = match state.audit() {
        Some(log) => json!({
            "entries": log.len()?,
            "unsigned": log.unsigned()?,
            "checkpointAgeSeconds": log.checkpoint_age_s(state.now_ms())?,
        }),
        None => Value::Null,
    };
    Ok(json!({
        "version": config.version,
        "profile": config.profile.name(),
        "hostname": config.hostname,
        "time": pds_types::iso_millis(state.now_ms()),
        "accounts": accounts,
        "sequencer": { "head": writes.sequencer().head() },
        "subscribers": state.subscribers().len(),
        "commitsInFlight": pending,
        "noticeOutboxDepth": notices,
        "audit": audit,
    }))
}

fn list_accounts(state: &AppState, query: Option<&str>) -> Result<Value, XrpcError> {
    let limit = limit(query, 50, 500);
    let cursor = param(query, "cursor");
    let after = cursor
        .as_deref()
        .map(|cursor| {
            cursor
                .split_once("::")
                .ok_or_else(|| XrpcError::invalid_request("Malformed cursor"))
        })
        .transpose()?;
    let page = writes(state)?.with_accounts(|accounts| accounts.list_repos(limit, after))?;
    let next = (page.len() == limit as usize)
        .then(|| page.last())
        .flatten()
        .map(|last| format!("{}::{}", last.actor.created_at, last.actor.did));
    let accounts: Vec<Value> = page
        .iter()
        .map(|listing| {
            json!({
                "did": listing.actor.did,
                "handle": listing.actor.handle,
                "createdAt": listing.actor.created_at,
                "status": listing.actor.status().unwrap_or("active"),
                "head": listing.head,
                "rev": listing.rev,
            })
        })
        .collect();
    Ok(json!({ "accounts": accounts, "cursor": next }))
}

/// One account: the repository card and the sessions.
fn account(state: &AppState, query: Option<&str>) -> Result<Value, XrpcError> {
    let did = param(query, "did")
        .ok_or_else(|| XrpcError::invalid_request("Params must have the property \"did\""))?;
    let writes = writes(state)?;
    let found = writes.with_accounts(|accounts| {
        let Some(account) = accounts.account(&did, false, Availability::ANY)? else {
            return Ok(None);
        };
        let root = accounts.repo_root(&did)?;
        let legacy = accounts.refresh_tokens_of(&did)?;
        let tokens = accounts.account_tokens(&did)?;
        let devices = accounts.account_devices(&did)?;
        Ok::<_, pds_storage::StorageError>(Some((account, root, legacy, tokens, devices)))
    })?;
    let Some((account, root, legacy, tokens, devices)) = found else {
        return Err(XrpcError::bad_request(
            "AccountNotFound",
            "The server hosts no account with this DID",
        ));
    };
    let role = writes.with_ops(|ops| ops.role(&did))?;
    let path = state.layout().actor_db(&did);
    let repo = if path.exists() {
        let store = ActorDb::open(&path)?;
        let (records, blobs, referenced) = store.counts()?;
        json!({
            "records": records,
            "blobs": blobs,
            "referencedBlobs": referenced,
            "blocks": store.block_count()?,
        })
    } else {
        Value::Null
    };
    let legacy: Vec<Value> = legacy
        .iter()
        .map(|(app_password, expires)| json!({ "appPassword": app_password, "expiresAt": expires }))
        .collect();
    let oauth: Vec<Value> = tokens
        .iter()
        .map(|token| {
            json!({
                "clientId": token.client_id,
                "createdAt": token.created_at,
                "updatedAt": token.updated_at,
                "expiresAt": token.expires_at,
                "scope": token.scope,
            })
        })
        .collect();
    let devices: Vec<Value> = devices
        .iter()
        .map(|(device, signed_in)| {
            json!({
                "userAgent": device.user_agent,
                "ipAddress": device.ip_address,
                "lastSeenAt": device.last_seen_at,
                "signedInAt": signed_in,
            })
        })
        .collect();
    Ok(json!({
        "did": account.actor.did,
        "handle": account.actor.handle,
        "email": account.email,
        "emailConfirmedAt": account.email_confirmed_at,
        "createdAt": account.actor.created_at,
        "status": account.actor.status().unwrap_or("active"),
        "takedownRef": account.actor.takedown_ref,
        "deactivatedAt": account.actor.deactivated_at,
        "role": role,
        "head": root.as_ref().map(|(cid, _)| cid),
        "rev": root.as_ref().map(|(_, rev)| rev),
        "repo": repo,
        "legacySessions": legacy,
        "oauthSessions": oauth,
        "devices": devices,
    }))
}

fn list_audit(state: &AppState, query: Option<&str>) -> Result<Value, XrpcError> {
    let log = state
        .audit()
        .ok_or_else(|| XrpcError::internal("the server has no audit log"))?;
    let limit = u64::from(limit(query, 50, 500));
    let before = param(query, "before").and_then(|value| value.parse::<u64>().ok());
    let entries = log.page(before, limit)?;
    let next = entries
        .last()
        .and_then(|entry| entry["seq"].as_u64())
        .filter(|seq| *seq > 1);
    Ok(json!({ "entries": entries, "before": next }))
}

fn list_roles(state: &AppState) -> Result<Value, XrpcError> {
    let roles: Vec<Value> = writes(state)?
        .with_ops(pds_storage::OpsDb::roles)?
        .iter()
        .map(|role| {
            json!({
                "did": role.did,
                "role": role.role,
                "grantedBy": role.granted_by,
                "grantedAt": role.granted_at,
            })
        })
        .collect();
    Ok(json!({ "roles": roles }))
}

fn list_domains(state: &AppState) -> Result<Value, XrpcError> {
    let domains: Vec<Value> = writes(state)?
        .with_ops(pds_storage::OpsDb::blocked_email_domains)?
        .iter()
        .map(|block| {
            json!({
                "domain": block.domain,
                "reason": block.reason,
                "createdBy": block.created_by,
                "createdAt": block.created_at,
            })
        })
        .collect();
    Ok(json!({ "domains": domains }))
}

/// The live stream: the overview and the subscribers, every two seconds,
/// as JSON text frames. It ends when the client goes.
async fn stream(mut socket: WebSocket, state: AppState) {
    let mut ticks = tokio::time::interval(STREAM_INTERVAL);
    loop {
        tokio::select! {
            received = socket.recv() => match received {
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
            _ = ticks.tick() => {
                let live = state
                    .blocking(|state| {
                        Ok(json!({
                            "overview": overview(state)?,
                            "subscribers": subscribers(state),
                        }))
                    })
                    .await;
                let Ok(live) = live else { return };
                if socket.send(Message::Text(live.to_string().into())).await.is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_may_do_what_the_one_before_it_may() {
        assert!(Role::Moderator < Role::Operator && Role::Operator < Role::Admin);
        assert_eq!(needs("internal.ops.grantRole"), Role::Admin);
        assert_eq!(needs("internal.ops.revokeSessions"), Role::Moderator);
        assert_eq!(
            needs("com.atproto.admin.updateSubjectStatus"),
            Role::Moderator
        );
        assert_eq!(needs("com.atproto.admin.deleteAccount"), Role::Operator);
        assert_eq!(needs("internal.ops.emitSync"), Role::Operator);
        assert_eq!(Role::parse("admin"), None);
        assert_eq!(Role::parse("operator"), Some(Role::Operator));
    }

    #[test]
    fn an_action_needs_a_reason() {
        let mut headers = HeaderMap::new();
        assert!(reason(&headers).is_err());
        headers.insert(REASON_HEADER, HeaderValue::from_static("   "));
        assert!(reason(&headers).is_err());
        headers.insert(REASON_HEADER, HeaderValue::from_static(" spam wave "));
        assert_eq!(reason(&headers).unwrap(), "spam wave");
        let long = "a".repeat(REASON_LIMIT + 1);
        headers.insert(REASON_HEADER, HeaderValue::from_str(&long).unwrap());
        assert!(reason(&headers).is_err());
    }

    #[test]
    fn a_blocked_domain_is_a_domain_name() {
        let input = |domain: &str| {
            let mut map = Map::new();
            map.insert("domain".to_owned(), json!(domain));
            map
        };
        assert_eq!(domain_of(&input("Spam.Example")).unwrap(), "spam.example");
        for bad in [
            "example",
            "a@spam.example",
            "spam..example",
            ".example",
            "sp am.example",
        ] {
            assert!(domain_of(&input(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_admin_methods_are_served_and_others_are_not() {
        assert!(is_admin_method("com.atproto.admin.updateSubjectStatus"));
        assert!(is_admin_method("com.atproto.server.createInviteCodes"));
        assert!(!is_admin_method("com.atproto.admin.noSuchMethod"));
        assert!(!is_admin_method("com.atproto.repo.createRecord"));
        assert!(!is_admin_method("com.atproto.server.createSession"));
    }
}
