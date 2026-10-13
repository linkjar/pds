//! The handlers, by namespace, and the way from a request to one.

pub mod identity;
pub mod import;
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
use pds_identity::handle::WELL_KNOWN_PATH;
use pds_storage::{AccountDb, Actor, Availability};

use crate::auth::{Credentials, optional_credentials};
use crate::error::XrpcError;
use crate::params::{Format, Params, Spec};
use crate::state::AppState;
use axum::body::Body;

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
        "com.atproto.server.createAppPassword" => Procedure::CreateAppPassword,
        "com.atproto.server.revokeAppPassword" => Procedure::RevokeAppPassword,
        _ => return None,
    })
}

/// The query that `nsid` names and its parameters, in the order its lexicon
/// declares them. `None` for a method without a handler.
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
    match path {
        "/" if reads => return root::banner(),
        "/robots.txt" if reads => return root::robots(),
        "/xrpc/_health" if reads => return root::health(&state).await,
        WELL_KNOWN_PATH if reads => return root::atproto_did(&state, &parts.headers).await,
        _ => {}
    }
    if !path.starts_with("/xrpc/") {
        return root::not_found(&method, path);
    }
    let Some(nsid) = path_nsid(path) else {
        return XrpcError::invalid_request("invalid xrpc path").into_response();
    };
    // The firehose is a WebSocket. Without an upgrade the method has no
    // route and falls through, as at the pin (SPEC §7.2).
    if nsid == subscribe::NSID
        && state.writes().is_some()
        && let Some(response) = subscribe::upgrade(&state, &mut parts).await
    {
        return response;
    }
    if !reads && method != Method::POST {
        return XrpcError::invalid_request("XRPC requests only supports GET and POST")
            .into_response();
    }
    let mut body = Some(body);
    let response = xrpc(
        &state,
        nsid,
        &method,
        uri.query(),
        &parts.headers,
        &mut body,
    )
    .await
    .unwrap_or_else(IntoResponse::into_response);
    // A handler that answered without reading the body left it here.
    if let Some(unread) = body {
        crate::body::drain(unread).await;
    }
    response
}

async fn xrpc(
    state: &AppState,
    nsid: &str,
    method: &Method,
    raw_query: Option<&str>,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    // A read-only server has no handler for a write.
    if state.writes().is_some()
        && let Some(procedure) = procedure(nsid)
    {
        if method != Method::POST {
            return Err(XrpcError::invalid_request(format!(
                "Incorrect HTTP method ({method}) expected POST"
            )));
        }
        return match procedure {
            Procedure::CreateRecord => repo_write::create_record(state, headers, body).await,
            Procedure::PutRecord => repo_write::put_record(state, headers, body).await,
            Procedure::DeleteRecord => repo_write::delete_record(state, headers, body).await,
            Procedure::ApplyWrites => repo_write::apply_writes(state, headers, body).await,
            Procedure::UploadBlob => repo_write::upload_blob(state, headers, body).await,
            Procedure::ImportRepo => import::import_repo(state, headers, body).await,
            Procedure::CreateSession => session::create_session(state, headers, body).await,
            Procedure::RefreshSession => session::refresh_session(state, headers).await,
            Procedure::DeleteSession => session::delete_session(state, headers).await,
            Procedure::CreateAppPassword => {
                session::create_app_password(state, headers, body).await
            }
            Procedure::RevokeAppPassword => {
                session::revoke_app_password(state, headers, body).await
            }
        };
    }
    let Some((served, specs)) = served(nsid) else {
        return Err(unhandled(state, nsid, headers));
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
        Served::IdentityResolveHandle => identity::resolve_handle(state, &query).await,
        Served::RepoListMissingBlobs if state.writes().is_some() => {
            repo_write::list_missing_blobs(state, &query).await
        }
        Served::RepoListMissingBlobs => Err(unhandled(state, nsid, headers)),
    }
}

/// A method with no handler. At the pin it goes to the catch-all proxy,
/// which wants a session first (SPEC §4.2, VP-6): no credential is 401, and
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
    let accounts = AccountDb::open(&state.layout().account_db)?;
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
