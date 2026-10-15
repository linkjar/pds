//! The handlers, by namespace, and the way from a request to one.

pub mod account;
pub mod admin;
pub mod email;
pub mod entryway;
pub mod identity;
pub mod import;
pub mod invites;
pub mod lifecycle;
mod linkjar;
pub mod plc;
pub mod prefs;
pub mod proxy;
mod read_after_write;
pub mod repo;
pub mod repo_write;
pub mod root;
pub mod server;
pub mod session;
pub mod subscribe;
pub mod sync;

use axum::extract::State;
use axum::http::{HeaderMap, Method};
use axum::response::{IntoResponse, Response};
use pds_storage::{Actor, Availability};

use crate::auth::{Credentials, optional_credentials};
use crate::error::XrpcError;
use crate::params::{Format, Params, Spec};
use crate::state::{AppState, WriteState};
use axum::body::Body;

/// The write path, or the internal error of a read-only server.
pub(crate) fn writes(state: &AppState) -> Result<&WriteState, XrpcError> {
    state
        .writes()
        .ok_or_else(|| XrpcError::internal("the server is read-only"))
}

/// One query as a handler sees it.
#[derive(Debug)]
pub struct Query<'a> {
    /// The validated parameters.
    pub params: Params,
    /// The request headers.
    pub headers: &'a HeaderMap,
    /// The raw query string, for a request that is forwarded.
    pub raw_query: Option<&'a str>,
}

impl Query<'_> {
    /// The optional credential of the request. `additional` names scopes
    /// the method accepts besides the standard access scopes.
    pub fn credentials(
        &self,
        state: &AppState,
        additional: &[&str],
    ) -> Result<Credentials, XrpcError> {
        optional_credentials(
            self.headers,
            &state.config().auth,
            additional,
            state.now_s(),
        )
    }
}

/// A query the crate serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Served {
    SyncGetRepo,
    SyncGetCheckout,
    SyncGetRecord,
    SyncGetBlocks,
    SyncGetLatestCommit,
    SyncGetHead,
    SyncGetRepoStatus,
    SyncListRepos,
    SyncListBlobs,
    SyncGetBlob,
    RepoGetRecord,
    RepoListRecords,
    RepoDescribeRepo,
    ServerDescribeServer,
    ServerGetSession,
    ServerListAppPasswords,
    ServerCheckAccountStatus,
    AdminGetAccountInfo,
    ServerGetServiceAuth,
    ActorGetPreferences,
    IdentityGetRecommendedDidCredentials,
    AdminGetAccountInfos,
    AdminGetInviteCodes,
    ServerGetAccountInviteCodes,
    AdminGetSubjectStatus,
    IdentityResolveHandle,
    RepoListMissingBlobs,
}

/// A procedure the crate serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Procedure {
    CreateRecord,
    PutRecord,
    DeleteRecord,
    ApplyWrites,
    UploadBlob,
    ImportRepo,
    CreateSession,
    RefreshSession,
    DeleteSession,
    CreateAppPassword,
    RevokeAppPassword,
    CreateAccount,
    DeactivateAccount,
    ActivateAccount,
    ReserveSigningKey,
    UpdateHandle,
    AdminUpdateSubjectStatus,
    PutPreferences,
    CreateInviteCode,
    CreateInviteCodes,
    DisableInviteCodes,
    DisableAccountInvites,
    EnableAccountInvites,
    RequestEmailConfirmation,
    ConfirmEmail,
    RequestEmailUpdate,
    UpdateEmail,
    RequestPasswordReset,
    ResetPassword,
    RequestAccountDelete,
    DeleteAccount,
    AdminDeleteAccount,
    AdminUpdateAccountEmail,
    AdminUpdateAccountHandle,
    AdminUpdateAccountPassword,
    AdminSendEmail,
    RequestPlcOperationSignature,
    SignPlcOperation,
    SubmitPlcOperation,
}

/// The procedure that `nsid` names. `None` for a method without a handler.
fn procedure(nsid: &str) -> Option<Procedure> {
    Some(match nsid {
        "com.atproto.repo.createRecord" => Procedure::CreateRecord,
        "com.atproto.repo.putRecord" => Procedure::PutRecord,
        "com.atproto.repo.deleteRecord" => Procedure::DeleteRecord,
        "com.atproto.repo.applyWrites" => Procedure::ApplyWrites,
        "com.atproto.repo.uploadBlob" => Procedure::UploadBlob,
        "com.atproto.repo.importRepo" => Procedure::ImportRepo,
        "com.atproto.server.createSession" => Procedure::CreateSession,
        "com.atproto.server.refreshSession" => Procedure::RefreshSession,
        "com.atproto.server.deleteSession" => Procedure::DeleteSession,
        "com.atproto.server.createAccount" => Procedure::CreateAccount,
        "com.atproto.server.deactivateAccount" => Procedure::DeactivateAccount,
        "com.atproto.server.activateAccount" => Procedure::ActivateAccount,
        "com.atproto.server.reserveSigningKey" => Procedure::ReserveSigningKey,
        "com.atproto.identity.updateHandle" => Procedure::UpdateHandle,
        "com.atproto.admin.updateSubjectStatus" => Procedure::AdminUpdateSubjectStatus,
        "app.bsky.actor.putPreferences" => Procedure::PutPreferences,
        "com.atproto.server.createInviteCode" => Procedure::CreateInviteCode,
        "com.atproto.server.createInviteCodes" => Procedure::CreateInviteCodes,
        "com.atproto.admin.disableInviteCodes" => Procedure::DisableInviteCodes,
        "com.atproto.admin.disableAccountInvites" => Procedure::DisableAccountInvites,
        "com.atproto.admin.enableAccountInvites" => Procedure::EnableAccountInvites,
        "com.atproto.server.requestEmailConfirmation" => Procedure::RequestEmailConfirmation,
        "com.atproto.server.confirmEmail" => Procedure::ConfirmEmail,
        "com.atproto.server.requestEmailUpdate" => Procedure::RequestEmailUpdate,
        "com.atproto.server.updateEmail" => Procedure::UpdateEmail,
        "com.atproto.server.requestPasswordReset" => Procedure::RequestPasswordReset,
        "com.atproto.server.resetPassword" => Procedure::ResetPassword,
        "com.atproto.server.requestAccountDelete" => Procedure::RequestAccountDelete,
        "com.atproto.server.deleteAccount" => Procedure::DeleteAccount,
        "com.atproto.admin.deleteAccount" => Procedure::AdminDeleteAccount,
        "com.atproto.admin.sendEmail" => Procedure::AdminSendEmail,
        "com.atproto.admin.updateAccountEmail" => Procedure::AdminUpdateAccountEmail,
        "com.atproto.admin.updateAccountHandle" => Procedure::AdminUpdateAccountHandle,
        "com.atproto.admin.updateAccountPassword" => Procedure::AdminUpdateAccountPassword,
        "com.atproto.identity.requestPlcOperationSignature" => {
            Procedure::RequestPlcOperationSignature
        }
        "com.atproto.identity.signPlcOperation" => Procedure::SignPlcOperation,
        "com.atproto.identity.submitPlcOperation" => Procedure::SubmitPlcOperation,
        "com.atproto.server.createAppPassword" => Procedure::CreateAppPassword,
        "com.atproto.server.revokeAppPassword" => Procedure::RevokeAppPassword,
        _ => return None,
    })
}

/// The query that `nsid` names and its parameters, in the order its lexicon
/// declares them. `None` for a method without a handler.
// A table: one arm for each query the crate serves.
#[allow(clippy::too_many_lines)]
fn served(nsid: &str) -> Option<(Served, &'static [Spec])> {
    const DID: Spec = Spec::required("did", Format::Did);
    const ONLY_DID: &[Spec] = &[DID];
    const REPO_AND_SINCE: &[Spec] = &[DID, Spec::optional("since", Format::Tid)];
    Some(match nsid {
        "com.atproto.sync.getRepo" => (Served::SyncGetRepo, REPO_AND_SINCE),
        "com.atproto.sync.getCheckout" => (Served::SyncGetCheckout, ONLY_DID),
        "com.atproto.sync.getLatestCommit" => (Served::SyncGetLatestCommit, ONLY_DID),
        "com.atproto.sync.getHead" => (Served::SyncGetHead, ONLY_DID),
        "com.atproto.sync.getRepoStatus" => (Served::SyncGetRepoStatus, ONLY_DID),
        "com.atproto.sync.getRecord" => (
            Served::SyncGetRecord,
            const {
                &[
                    DID,
                    Spec::required("collection", Format::Nsid),
                    Spec::required("rkey", Format::RecordKey),
                ]
            },
        ),
        "com.atproto.sync.getBlocks" => (
            Served::SyncGetBlocks,
            const { &[DID, Spec::list("cids", Format::Cid)] },
        ),
        "com.atproto.sync.listRepos" => (
            Served::SyncListRepos,
            const {
                &[
                    Spec::integer("limit", 1, 1000, 500),
                    Spec::optional("cursor", Format::Any),
                ]
            },
        ),
        "com.atproto.sync.listBlobs" => (
            Served::SyncListBlobs,
            const {
                &[
                    DID,
                    Spec::optional("since", Format::Tid),
                    Spec::integer("limit", 1, 1000, 500),
                    Spec::optional("cursor", Format::Any),
                ]
            },
        ),
        "com.atproto.sync.getBlob" => (
            Served::SyncGetBlob,
            const { &[DID, Spec::required("cid", Format::Cid)] },
        ),
        "com.atproto.repo.getRecord" => (
            Served::RepoGetRecord,
            const {
                &[
                    Spec::required("repo", Format::AtIdentifier),
                    Spec::required("collection", Format::Nsid),
                    Spec::required("rkey", Format::RecordKey),
                    Spec::optional("cid", Format::Cid),
                ]
            },
        ),
        "com.atproto.repo.listRecords" => (
            Served::RepoListRecords,
            const {
                &[
                    Spec::required("repo", Format::AtIdentifier),
                    Spec::required("collection", Format::Nsid),
                    Spec::integer("limit", 1, 100, 50),
                    Spec::optional("cursor", Format::Any),
                    Spec::boolean("reverse", false),
                ]
            },
        ),
        "com.atproto.repo.describeRepo" => (
            Served::RepoDescribeRepo,
            const { &[Spec::required("repo", Format::AtIdentifier)] },
        ),
        "com.atproto.repo.listMissingBlobs" => (
            Served::RepoListMissingBlobs,
            const {
                &[
                    Spec::integer("limit", 1, 1000, 500),
                    Spec::optional("cursor", Format::Any),
                ]
            },
        ),
        "com.atproto.server.describeServer" => (Served::ServerDescribeServer, &[]),
        "com.atproto.server.getSession" => (Served::ServerGetSession, &[]),
        "com.atproto.server.listAppPasswords" => (Served::ServerListAppPasswords, &[]),
        "com.atproto.server.checkAccountStatus" => (Served::ServerCheckAccountStatus, &[]),
        "app.bsky.actor.getPreferences" => (Served::ActorGetPreferences, &[]),
        "com.atproto.identity.getRecommendedDidCredentials" => {
            (Served::IdentityGetRecommendedDidCredentials, &[])
        }
        "com.atproto.admin.getAccountInfos" => (
            Served::AdminGetAccountInfos,
            const { &[Spec::list("dids", Format::Did)] },
        ),
        "com.atproto.admin.getInviteCodes" => (
            Served::AdminGetInviteCodes,
            const {
                &[
                    Spec::optional("sort", Format::Any),
                    Spec::integer("limit", 1, 500, 100),
                    Spec::optional("cursor", Format::Any),
                ]
            },
        ),
        "com.atproto.server.getAccountInviteCodes" => (
            Served::ServerGetAccountInviteCodes,
            const {
                &[
                    Spec::boolean("includeUsed", true),
                    Spec::boolean("createAvailable", true),
                ]
            },
        ),
        "com.atproto.server.getServiceAuth" => (
            Served::ServerGetServiceAuth,
            const {
                &[
                    Spec::required("aud", Format::Any),
                    Spec::optional("exp", Format::Any),
                    Spec::optional("lxm", Format::Nsid),
                ]
            },
        ),
        "com.atproto.admin.getAccountInfo" => (Served::AdminGetAccountInfo, const { &[DID] }),
        "com.atproto.admin.getSubjectStatus" => (
            Served::AdminGetSubjectStatus,
            const {
                &[
                    Spec::optional("did", Format::Did),
                    Spec::optional("uri", Format::Any),
                    Spec::optional("blob", Format::Cid),
                ]
            },
        ),
        "com.atproto.identity.resolveHandle" => (
            Served::IdentityResolveHandle,
            const { &[Spec::required("handle", Format::Handle)] },
        ),
        _ => return None,
    })
}

/// The method name of an `/xrpc/` path, as the Reference extracts it:
/// letters and digits in parts separated by single dots or hyphens, at
/// least two characters, with an optional trailing slash.
fn path_nsid(path: &str) -> Option<&str> {
    let name = path.strip_prefix("/xrpc/")?;
    let name = name.strip_suffix('/').unwrap_or(name);
    let mut separator_allowed = false;
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() {
            separator_allowed = true;
        } else if (byte == b'-' || byte == b'.') && separator_allowed {
            separator_allowed = false;
        } else {
            return None;
        }
    }
    (separator_allowed && name.len() >= 2).then_some(name)
}

/// Every request enters here.
pub async fn entry(State(state): State<AppState>, request: axum::extract::Request) -> Response {
    let (mut parts, body) = request.into_parts();
    let method = parts.method.clone();
    let uri = parts.uri.clone();
    let path = uri.path();
    let reads = method == Method::GET || method == Method::HEAD;
    if let Some(response) = root::plain(&state, &method, path, uri.query(), &parts.headers).await {
        return response;
    }
    let peer = parts
        .extensions
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|info| info.0.ip());
    if !path.starts_with("/xrpc/") {
        // The authorization server (SPEC §11). Each request spends the
        // point of the address bucket.
        let mut body = Some(body);
        let oauth = async {
            if let Err(refusal) = crate::ratelimit::route(&state, &[]) {
                return refusal.into_response();
            }
            let query = uri.query();
            crate::oauth::serve(
                &state,
                &method,
                path,
                query,
                &parts.headers,
                peer,
                &mut body,
            )
            .await
            .unwrap_or_else(|| root::not_found(&method, path))
        };
        return Box::pin(crate::ratelimit::scope(&state, &parts.headers, peer, oauth)).await;
    }
    let Some(nsid) = path_nsid(path) else {
        return XrpcError::invalid_request("invalid xrpc path").into_response();
    };
    // The firehose is a WebSocket. Without an upgrade the method has no
    // route and falls through, as at the pin (SPEC §7.2).
    if nsid == subscribe::NSID
        && let Some(response) = firehose(&state, &mut parts).await
    {
        return response;
    }
    if !reads && method != Method::POST {
        return XrpcError::invalid_request("XRPC requests only supports GET and POST")
            .into_response();
    }
    let mut body = Some(body);
    let dpop = crate::oauth::resource::is_dpop(&parts.headers);
    let answer = async {
        let limited = if ROUTES_WITH_LIMITS.contains(&nsid) {
            Ok(())
        } else {
            crate::ratelimit::route(&state, &[])
        };
        // An OAuth access token is checked once, here (SPEC §11.3).
        let granted = match limited {
            Ok(()) if dpop => oauth_grant(&state, nsid, &method, path, &parts.headers).await,
            Ok(()) if member_call(&state, nsid, &parts.headers) => {
                member_grant(&state, nsid, &parts.headers).await
            }
            Ok(()) => Ok(None),
            Err(refusal) => Err(Box::new(refusal.into_response())),
        };
        let response = match granted {
            Ok(grant) => {
                let query = uri.query();
                Box::pin(call(
                    &state,
                    nsid,
                    &method,
                    query,
                    &parts.headers,
                    &mut body,
                    grant,
                ))
                .await
            }
            Err(refusal) => *refusal,
        };
        let response = if dpop {
            crate::oauth::with_nonce(&state, response)
        } else {
            response
        };
        // A handler that answered without reading the body left it here.
        if let Some(unread) = body {
            crate::body::drain(unread).await;
        }
        response
    };
    Box::pin(crate::ratelimit::scope(
        &state,
        &parts.headers,
        peer,
        answer,
    ))
    .await
}

/// The firehose of a WebSocket upgrade: the server's own, or at an
/// entryway the one of its member host (SPEC §21.3).
async fn firehose(state: &AppState, parts: &mut axum::http::request::Parts) -> Option<Response> {
    match state.config().member_host.clone() {
        Some(host) => entryway::firehose(&host, parts).await,
        None if state.writes().is_some() => subscribe::upgrade(state, parts).await,
        None => None,
    }
}

/// Whether the server has a handler for the method. Every other method
/// goes to the proxy.
pub fn is_handled(nsid: &str) -> bool {
    procedure(nsid).is_some() || served(nsid).is_some() || nsid == subscribe::NSID
}

/// Runs a method with the grant of its OAuth token in reach, and writes
/// its audit entry when the method is audited.
pub(crate) async fn call(
    state: &AppState,
    nsid: &str,
    method: &Method,
    query: Option<&str>,
    headers: &HeaderMap,
    body: &mut Option<Body>,
    grant: Option<crate::oauth::resource::Grant>,
) -> Response {
    let oauth = grant
        .as_ref()
        .map(|grant| (grant.did.clone(), grant.client_id.clone()));
    let run = xrpc(state, nsid, method, query, headers, body);
    let (result, subject) = Box::pin(crate::audit::with_subject(
        crate::oauth::resource::with_grant(grant, run),
    ))
    .await;
    // The answer has a body, which does not cross an await by reference.
    let outcome = match &result {
        Ok(response) if response.status().is_success() => "ok".to_owned(),
        Ok(response) => response.status().as_u16().to_string(),
        Err(refusal) => refusal.error.clone(),
    };
    if crate::audit::is_audited(nsid) {
        audit_request(state, nsid, headers, oauth, subject, outcome).await;
    } else if let Some((did, client_id)) = oauth
        && let Some(ai_client) = mcp_client(state, &client_id, nsid, headers)
    {
        // A write of an MCP sidecar names the sidecar and the AI client
        // it acted for (SPEC §22.2).
        let what = pds_audit::Draft {
            subject: Some(subject.unwrap_or_else(|| did.clone())),
            actor: did,
            action: nsid.to_owned(),
            credential: Some("oauth".to_owned()),
            client_id: Some(client_id),
            outcome,
            sidecar: Some(format!("mcp:{ai_client}")),
            ..pds_audit::Draft::default()
        };
        crate::audit::record(state, what).await;
    }
    result.unwrap_or_else(IntoResponse::into_response)
}

/// The header in which an MCP sidecar names the AI client of a request.
pub const MCP_CLIENT_HEADER: &str = "x-mcp-client-id";

/// The AI client of a repository write that an MCP sidecar of this server
/// made. `None` for every other request.
fn mcp_client(
    state: &AppState,
    client_id: &str,
    nsid: &str,
    headers: &HeaderMap,
) -> Option<String> {
    let writes = matches!(
        nsid,
        "com.atproto.repo.createRecord"
            | "com.atproto.repo.putRecord"
            | "com.atproto.repo.deleteRecord"
            | "com.atproto.repo.applyWrites"
            | "com.atproto.repo.uploadBlob"
    );
    if !writes
        || !state
            .config()
            .mcp_client_ids
            .iter()
            .any(|id| id == client_id)
    {
        return None;
    }
    let named = headers
        .get(MCP_CLIENT_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    Some(named.chars().take(256).collect())
}

/// Writes the audit entry of a method that answered (SPEC §20.1): who
/// called it, with which kind of credential, on what, and how it ended.
async fn audit_request(
    state: &AppState,
    nsid: &str,
    headers: &HeaderMap,
    oauth: Option<(String, String)>,
    subject: Option<String>,
    outcome: String,
) {
    use crate::auth::{SCOPE_TAKENDOWN, optional_credentials};
    // On the ops listener the caller is the operator (SPEC §22.1).
    if let Some(acting) = crate::audit::acting() {
        let what = acting.draft(nsid, subject, outcome);
        return crate::audit::record(state, what).await;
    }
    let (actor, credential, client_id) = if let Some((did, client_id)) = oauth {
        (did, Some("oauth"), Some(client_id))
    } else {
        let auth = &state.config().auth;
        match optional_credentials(headers, auth, &[SCOPE_TAKENDOWN], state.now_s()) {
            Ok(Credentials::Access { did, .. }) => (did, Some("session"), None),
            Ok(Credentials::Admin) => ("admin".to_owned(), Some("admin"), None),
            _ => ("anonymous".to_owned(), None, None),
        }
    };
    // A method that names no subject acts on its caller.
    let subject = subject.or_else(|| actor.starts_with("did:").then(|| actor.clone()));
    let what = pds_audit::Draft {
        actor,
        action: nsid.to_owned(),
        subject,
        credential: credential.map(str::to_owned),
        client_id,
        outcome,
        ..pds_audit::Draft::default()
    };
    crate::audit::record(state, what).await;
}

/// Whether the request is a member host asking its entryway for an
/// account with a service token (SPEC §21.3): see
/// [`entryway::takes_service_token`].
fn member_call(state: &AppState, nsid: &str, headers: &HeaderMap) -> bool {
    state.config().member_host.is_some()
        && entryway::takes_service_token(nsid)
        && crate::service_auth::is_service_token(headers)
}

/// The grant of a member call. The member host checked the OAuth token
/// and its scope; the service token names the method, and the account
/// signed it. The grant lets the handler see the email address, which the
/// member host removes when the token does not allow it.
async fn member_grant(
    state: &AppState,
    nsid: &str,
    headers: &HeaderMap,
) -> Result<Option<crate::oauth::resource::Grant>, Box<Response>> {
    let did = crate::service_auth::verify(state, headers, nsid)
        .await
        .map_err(|refusal| Box::new(refusal.into_response()))?;
    // The OAuth session that the member host checked, which the receipt of
    // a sign-up belongs to.
    let claims = crate::service_auth::claims(headers).unwrap_or_default();
    let claim = |name: &str| {
        claims
            .get(name)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Ok(Some(crate::oauth::resource::Grant {
        did,
        client_id: claim(entryway::CLIENT_CLAIM),
        token_id: claim(entryway::SESSION_CLAIM),
        permissions: pds_oauth::scope::Permissions::new(
            "atproto transition:generic transition:email identity:handle account:status?action=manage",
        ),
    }))
}

/// The grant of a request with an OAuth access token, when the token is
/// good and may call the method.
async fn oauth_grant(
    state: &AppState,
    nsid: &str,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
) -> Result<Option<crate::oauth::resource::Grant>, Box<Response>> {
    let grant = crate::oauth::resource::authenticate(state, method, path, headers).await?;
    let handled = procedure(nsid).is_some() || served(nsid).is_some();
    crate::oauth::resource::check_method(&grant, nsid, handled)
        .map_err(|refusal| Box::new(refusal.into_response()))?;
    Ok(Some(grant))
}

/// The methods with limiters of their own. Each spends its points, and the
/// point of the address bucket, after its credential and its input are
/// read. Every other method spends the point of the address bucket when
/// it arrives.
const ROUTES_WITH_LIMITS: &[&str] = &[
    "com.atproto.sync.getRepo",
    "com.atproto.server.createAccount",
    "com.atproto.server.createSession",
    "com.atproto.server.requestPasswordReset",
    "com.atproto.server.resetPassword",
    "com.atproto.server.deleteAccount",
    "com.atproto.server.requestEmailConfirmation",
    "com.atproto.server.requestEmailUpdate",
    "com.atproto.server.requestAccountDelete",
    "com.atproto.identity.updateHandle",
    "com.atproto.repo.uploadBlob",
    "com.atproto.repo.createRecord",
    "com.atproto.repo.putRecord",
    "com.atproto.repo.deleteRecord",
    "com.atproto.repo.applyWrites",
];

/// Runs a procedure.
// A table: one arm for each procedure the crate serves.
#[allow(clippy::too_many_lines)]
async fn run(
    state: &AppState,
    procedure: Procedure,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    match procedure {
        Procedure::CreateRecord => repo_write::create_record(state, headers, body).await,
        Procedure::PutRecord => repo_write::put_record(state, headers, body).await,
        Procedure::DeleteRecord => repo_write::delete_record(state, headers, body).await,
        Procedure::ApplyWrites => repo_write::apply_writes(state, headers, body).await,
        Procedure::UploadBlob => repo_write::upload_blob(state, headers, body).await,
        Procedure::ImportRepo => import::import_repo(state, headers, body).await,
        Procedure::CreateSession => session::create_session(state, headers, body).await,
        Procedure::RefreshSession => session::refresh_session(state, headers).await,
        Procedure::DeleteSession => session::delete_session(state, headers).await,
        Procedure::CreateAccount => account::create_account(state, headers, body).await,
        Procedure::DeactivateAccount => lifecycle::deactivate_account(state, headers, body).await,
        Procedure::ActivateAccount => lifecycle::activate_account(state, headers).await,
        Procedure::ReserveSigningKey => plc::reserve_signing_key(state, headers, body).await,
        Procedure::UpdateHandle => lifecycle::update_handle(state, headers, body).await,
        Procedure::PutPreferences => prefs::put_preferences(state, headers, body).await,
        Procedure::RequestEmailConfirmation => {
            email::request_email_confirmation(state, headers).await
        }
        Procedure::ConfirmEmail => email::confirm_email(state, headers, body).await,
        Procedure::RequestEmailUpdate => email::request_email_update(state, headers).await,
        Procedure::UpdateEmail => email::update_email(state, headers, body).await,
        Procedure::RequestPasswordReset => {
            email::request_password_reset(state, headers, body).await
        }
        Procedure::RequestAccountDelete => lifecycle::request_account_delete(state, headers).await,
        Procedure::DeleteAccount => lifecycle::delete_account(state, headers, body).await,
        Procedure::RequestPlcOperationSignature => {
            plc::request_plc_operation_signature(state, headers).await
        }
        Procedure::SignPlcOperation => plc::sign_plc_operation(state, headers, body).await,
        Procedure::SubmitPlcOperation => plc::submit_plc_operation(state, headers, body).await,
        Procedure::AdminUpdateAccountEmail => {
            admin::update_account_email(state, headers, body).await
        }
        Procedure::AdminUpdateAccountHandle => {
            admin::update_account_handle(state, headers, body).await
        }
        Procedure::AdminUpdateAccountPassword => {
            admin::update_account_password(state, headers, body).await
        }
        Procedure::AdminSendEmail => admin::send_email(state, headers, body).await,
        Procedure::AdminDeleteAccount => admin::delete_account(state, headers, body).await,
        Procedure::ResetPassword => email::reset_password(state, headers, body).await,
        Procedure::CreateInviteCode => invites::create_invite_code(state, headers, body).await,
        Procedure::CreateInviteCodes => invites::create_invite_codes(state, headers, body).await,
        Procedure::DisableInviteCodes => invites::disable_invite_codes(state, headers, body).await,
        Procedure::DisableAccountInvites => {
            invites::set_account_invites(state, headers, body, true).await
        }
        Procedure::EnableAccountInvites => {
            invites::set_account_invites(state, headers, body, false).await
        }
        Procedure::AdminUpdateSubjectStatus => {
            admin::update_subject_status(state, headers, body).await
        }
        Procedure::CreateAppPassword => session::create_app_password(state, headers, body).await,
        Procedure::RevokeAppPassword => session::revoke_app_password(state, headers, body).await,
    }
}

async fn xrpc(
    state: &AppState,
    nsid: &str,
    method: &Method,
    raw_query: Option<&str>,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    // A member host passes the methods of its entryway on (SPEC §21.3).
    if let Some(entryway) = &state.config().entryway
        && entryway::is_forwarded(nsid)
        && !entryway::from_entryway(headers)
    {
        return entryway::forward(state, entryway, nsid, method, raw_query, headers, body).await;
    }
    // An entryway keeps no repository: what is not its own method goes to
    // its member host.
    if let Some(host) = &state.config().member_host
        && !entryway::is_entryway_method(nsid)
    {
        return entryway::forward_to_member(state, host, nsid, method, raw_query, headers, body)
            .await;
    }
    // The methods of the profile exist in that profile only (SPEC §12.1).
    if state.writes().is_some()
        && let Some(response) = linkjar::serve(state, nsid, method, headers).await
    {
        return response;
    }
    // A read-only server has no handler for a write.
    if state.writes().is_some()
        && let Some(procedure) = procedure(nsid)
    {
        if method != Method::POST {
            return Err(XrpcError::invalid_request(format!(
                "Incorrect HTTP method ({method}) expected POST"
            )));
        }
        return run(state, procedure, headers, body).await;
    }
    let Some((served, specs)) = served(nsid) else {
        // No handler here: another service has the method.
        let path = match raw_query {
            Some(query) => format!("/xrpc/{nsid}?{query}"),
            None => format!("/xrpc/{nsid}"),
        };
        return proxy::proxy(state, nsid, method, &path, headers, body).await;
    };
    if method != Method::GET && method != Method::HEAD {
        return Err(XrpcError::invalid_request(format!(
            "Incorrect HTTP method ({method}) expected GET"
        )));
    }
    let query = Query {
        params: Params::parse(nsid, raw_query, specs)?,
        headers,
        raw_query,
    };
    match served {
        Served::SyncGetRepo => sync::get_repo(state, &query, true).await,
        Served::SyncGetCheckout => sync::get_repo(state, &query, false).await,
        Served::SyncGetRecord => sync::get_record(state, &query).await,
        Served::SyncGetBlocks => sync::get_blocks(state, &query).await,
        Served::SyncGetLatestCommit => sync::get_latest_commit(state, &query).await,
        Served::SyncGetHead => sync::get_head(state, &query).await,
        Served::SyncGetRepoStatus => sync::get_repo_status(state, &query).await,
        Served::SyncListRepos => sync::list_repos(state, &query).await,
        Served::SyncListBlobs => sync::list_blobs(state, &query).await,
        Served::SyncGetBlob => sync::get_blob(state, &query).await,
        Served::RepoGetRecord => repo::get_record(state, &query).await,
        Served::RepoListRecords => repo::list_records(state, &query).await,
        Served::RepoDescribeRepo => repo::describe_repo(state, &query).await,
        Served::ServerDescribeServer => Ok(server::describe_server(state)),
        Served::ServerGetSession => session::get_session(state, headers).await,
        Served::ServerListAppPasswords => session::list_app_passwords(state, headers).await,
        Served::AdminGetAccountInfos => admin::get_account_infos(state, &query).await,
        Served::AdminGetInviteCodes => invites::get_invite_codes(state, &query).await,
        Served::ServerGetAccountInviteCodes => {
            invites::get_account_invite_codes(state, &query).await
        }
        Served::IdentityGetRecommendedDidCredentials => {
            plc::get_recommended_did_credentials(state, headers).await
        }
        Served::ActorGetPreferences => prefs::get_preferences(state, &query).await,
        Served::ServerGetServiceAuth => proxy::get_service_auth(state, &query).await,
        Served::AdminGetAccountInfo => admin::get_account_info(state, &query).await,
        Served::AdminGetSubjectStatus => admin::get_subject_status(state, &query).await,
        Served::ServerCheckAccountStatus => lifecycle::check_account_status(state, headers).await,
        Served::IdentityResolveHandle => identity::resolve_handle(state, &query).await,
        Served::RepoListMissingBlobs if state.writes().is_some() => {
            repo_write::list_missing_blobs(state, &query).await
        }
        Served::RepoListMissingBlobs => Err(unhandled(state, nsid, headers)),
    }
}

/// A method with no handler. At the pin it goes to the catch-all proxy,
/// which wants a session first (SPEC §4.2): no credential is 401, and
/// the admin password is not a session. The proxy itself is not in this
/// unit, so a request with a valid session is 501.
fn unhandled(state: &AppState, nsid: &str, headers: &HeaderMap) -> XrpcError {
    match crate::auth::session(state, headers, None) {
        Ok(_) => XrpcError::not_implemented(nsid),
        Err(error) => error,
    }
}

/// The account behind a repository read, as `assertRepoAvailability` of the
/// Reference finds it: an admin or the account itself reads it whatever its
/// status; anyone else reads an active account only.
pub fn repo_availability(
    state: &AppState,
    handle_or_did: &str,
    admin_or_self: bool,
) -> Result<Actor, XrpcError> {
    let accounts = state.reads().accounts()?;
    let actor = accounts
        .actor(handle_or_did, Availability::ANY)?
        .ok_or_else(|| {
            XrpcError::bad_request(
                "RepoNotFound",
                format!("Could not find repo for DID: {handle_or_did}"),
            )
        })?;
    if admin_or_self {
        return Ok(actor);
    }
    if actor.takedown_ref.is_some() {
        return Err(XrpcError::bad_request(
            "RepoTakendown",
            format!("Repo has been takendown: {handle_or_did}"),
        ));
    }
    if actor.deactivated_at.is_some() {
        return Err(XrpcError::bad_request(
            "RepoDeactivated",
            format!("Repo has been deactivated: {handle_or_did}"),
        ));
    }
    Ok(actor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_method_name_is_read_from_the_path_as_the_reference_reads_it() {
        assert_eq!(
            path_nsid("/xrpc/com.atproto.sync.getRepo"),
            Some("com.atproto.sync.getRepo")
        );
        assert_eq!(
            path_nsid("/xrpc/com.atproto.sync.getRepo/"),
            Some("com.atproto.sync.getRepo")
        );
        // Not an NSID, and still a name: the Reference sends it to its proxy.
        assert_eq!(path_nsid("/xrpc/nope"), Some("nope"));
        for bad in [
            "/xrpc/",
            "/xrpc/a",
            "/xrpc/a/b",
            "/xrpc/_health",
            "/xrpc/a..b",
            "/xrpc/.a",
            "/xrpc/a.",
            "/xrpc/a b",
        ] {
            assert_eq!(path_nsid(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_method_name_selects_its_handler_and_its_parameters() {
        let names = |nsid: &str| -> Option<(Served, Vec<&str>)> {
            served(nsid).map(|(served, specs)| (served, specs.iter().map(|s| s.name).collect()))
        };
        assert_eq!(
            names("com.atproto.sync.getRepo"),
            Some((Served::SyncGetRepo, vec!["did", "since"]))
        );
        assert_eq!(
            names("com.atproto.sync.getCheckout"),
            Some((Served::SyncGetCheckout, vec!["did"]))
        );
        assert_eq!(
            names("com.atproto.identity.resolveHandle"),
            Some((Served::IdentityResolveHandle, vec!["handle"]))
        );
        assert_eq!(
            names("com.atproto.server.describeServer"),
            Some((Served::ServerDescribeServer, vec![]))
        );
        assert_eq!(names("com.atproto.server.createSession"), None);
    }
}
