//! `com.atproto.repo.importRepo` (`docs/SPEC.md` §6.5).
//!
//! The steps are those of `importRepo.ts` at the pin. Three checks are the
//! Candidate's own, each a deliberate difference of SPEC §2.3: the tree is
//! checked in full, a record block must be canonical DAG-CBOR, and the
//! commit must name the account and carry a signature of its key. Bytes
//! that are not a CAR are 400, also a deliberate difference.
//!
//! An account that moves in is deactivated while it imports, and its
//! activation emits `#sync`; the import appends no event, as at the pin.
//! An import into an active account appends `#sync` (SPEC §2.3): the pin
//! appends nothing, and a strict relay then drops the account's next
//! commit, which does not follow the relay's copy.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use pds_repo::repo::{Expect, verify_diff_car, verify_repo_car};
use pds_repo::{BlockMap, Repo, RepoError, TreeDefect};
use pds_sequencer::sync_event;
use pds_storage::ActorDb;
use pds_types::{Data, Did};

use super::repo_write::find_account;
use crate::auth::{ACCESS_FULL, session};
use crate::body;
use crate::error::XrpcError;
use crate::state::AppState;
use crate::write::{Committed, Session, backlinks, blob_cids, record_uri, transact};

/// The encoding of the input.
const CAR: &str = "application/vnd.ipld.car";

/// The import body limit when the operator has not configured one: 64 MiB.
pub const DEFAULT_MAX_IMPORT_SIZE: u64 = 64 * 1024 * 1024;

fn refused(error: &RepoError) -> XrpcError {
    match error {
        // The wording of upstream's own refusal, which the production
        // Reference carries as its ninth patch.
        RepoError::Tree {
            cid,
            defect: TreeDefect::SharedNode,
        } => XrpcError::invalid_request(format!(
            "invalid MST: node referenced more than once: {cid}"
        )),
        RepoError::Car("expected exactly one root") => {
            XrpcError::invalid_request("expected one root")
        }
        other => XrpcError::invalid_request(format!("invalid repository: {other}")),
    }
}

impl Session<'_> {
    /// Replaces the repository with the one in `car`, and indexes the
    /// records that differ.
    fn import(&mut self, car: &[u8], active: bool) -> Result<(), XrpcError> {
        let did = Did::parse(self.did()).map_err(XrpcError::internal)?;
        let public = self.state().actor_key(self.did())?.public_key();
        // The tree is checked first and the commit second, so that a tree
        // the Reference also refuses gets the Reference's answer.
        let expect = Expect {
            did: None,
            verify: None,
        };
        let check_commit = |commit: &pds_repo::Commit| -> Result<(), XrpcError> {
            if commit.did != did {
                return Err(XrpcError::invalid_request(
                    "invalid repository: the commit is for another account",
                ));
            }
            commit
                .verify(|bytes, signature| public.verify(bytes, signature).is_ok())
                .map_err(|_| {
                    XrpcError::invalid_request(
                        "invalid repository: the commit signature does not verify",
                    )
                })
        };
        let invalid = |error: RepoError| {
            if error.is_invalid_input() {
                refused(&error)
            } else {
                XrpcError::internal(error)
            }
        };

        let txn = self.txn();
        let actor: &ActorDb = txn;
        let root = txn.root()?;
        // The stored revision of an import is a new one, whatever the
        // commit says, as at the pin.
        let rev = self.writes().tids().next().as_str().to_owned();
        let now = self.now().to_owned();
        let (cid, commit_rev, changes, new_blocks, removed) = if let Some(root) = &root {
            let repo = Repo::load(actor, root.cid).map_err(XrpcError::internal)?;
            let stored = usize::try_from(txn.block_count()?).unwrap_or(usize::MAX);
            let diff = verify_diff_car(&repo, car, &expect, stored).map_err(invalid)?;
            check_commit(&diff.commit)?;
            let removed: Vec<_> = diff.removed_cids.into_iter().collect();
            let commit_rev = diff.commit.rev.as_str().to_owned();
            (diff.cid, commit_rev, diff.changes, diff.new_blocks, removed)
        } else {
            let repo = verify_repo_car(car, &expect, false).map_err(invalid)?;
            check_commit(&repo.commit)?;
            let mut blocks = BlockMap::new();
            let reached = repo
                .nodes
                .iter()
                .chain(repo.records.iter().map(|(_, cid)| cid))
                .chain(std::iter::once(&repo.cid));
            for cid in reached {
                if let Some(bytes) = repo.blocks.get_shared(cid) {
                    blocks.insert(*cid, bytes);
                }
            }
            let changes = repo
                .records
                .into_iter()
                .map(|(key, cid)| pds_repo::repo::RecordChange {
                    key,
                    prev: None,
                    cid: Some(cid),
                })
                .collect();
            let commit_rev = repo.commit.rev.as_str().to_owned();
            (repo.cid, commit_rev, changes, blocks, Vec::new())
        };

        txn.update_root(self.did(), &cid, &rev, &now, root.is_none())?;
        txn.put_blocks(&new_blocks, &rev)?;
        txn.delete_blocks(&removed)?;
        self.index_imported(&changes, &new_blocks, &rev, &now)?;
        if active {
            // The event names the revision of the commit block, which a
            // relay checks; the note and the account root name the stored
            // revision.
            let block = match new_blocks.get(&cid) {
                Some(block) => block.to_vec(),
                None => self.txn().block(&cid)?.ok_or_else(|| {
                    XrpcError::internal("the commit block of the import is missing")
                })?,
            };
            let event = sync_event(self.did(), &commit_rev, &cid, &block, &now);
            self.state_head(event, Committed { cid, rev });
        }
        Ok(())
    }

    /// Indexes the records an import changed: the record row, the links and
    /// the blob references of each.
    fn index_imported(
        &mut self,
        changes: &[pds_repo::repo::RecordChange],
        new_blocks: &BlockMap,
        rev: &str,
        now: &str,
    ) -> Result<(), XrpcError> {
        let txn = self.txn();
        let replaced: Vec<_> = changes
            .iter()
            .filter(|change| change.prev.is_some())
            .map(|change| format!("at://{}/{}", self.did(), change.key))
            .collect();
        let removed_blobs = txn.delete_record_blobs(&replaced)?;
        for change in changes {
            let (collection, rkey) = change
                .key
                .split_once('/')
                .ok_or_else(|| XrpcError::internal("a repository key without a slash"))?;
            let uri = record_uri(self.did(), collection, rkey);
            let Some(record_cid) = change.cid else {
                txn.delete_record(&uri)?;
                continue;
            };
            let unparsed = || {
                XrpcError::invalid_request(format!(
                    "Could not parse record at '{collection}/{rkey}'"
                ))
            };
            let bytes = match new_blocks.get_shared(&record_cid) {
                Some(bytes) => bytes,
                None => txn.block(&record_cid)?.ok_or_else(unparsed)?.into(),
            };
            let record = Data::from_dag_cbor(&bytes).map_err(|_| unparsed())?;
            txn.index_record(&uri, &record_cid.to_string(), collection, rkey, rev, now)?;
            if change.prev.is_some() {
                txn.remove_backlinks(&uri)?;
            }
            txn.add_backlinks(&backlinks(&uri, &record))?;
            for blob in blob_cids(&record) {
                txn.associate_blob(&blob, &uri)?;
            }
        }
        self.remove_unreferenced_blobs(removed_blobs)?;
        Ok(())
    }
}

/// `com.atproto.repo.importRepo`.
pub async fn import_repo(
    state: &AppState,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let did = session(state, headers, Some(ACCESS_FULL))?;
    let active = {
        let did = did.clone();
        state
            .blocking(move |state| {
                find_account(state, &did, true, false).map(|actor| actor.deactivated_at.is_none())
            })
            .await?
    };
    body::expect_body(headers)?;
    let encoding = body::request_encoding(headers)?;
    if encoding != CAR {
        return Err(XrpcError::invalid_request(format!(
            "Wrong request encoding (Content-Type): {encoding}"
        )));
    }
    let settings = state
        .writes()
        .ok_or_else(|| XrpcError::internal("the server is read-only"))?
        .settings()
        .clone();
    if !settings.accepting_imports {
        return Err(XrpcError::invalid_request(
            "Service is not accepting repo imports",
        ));
    }
    let limit = settings.max_import_size.unwrap_or(DEFAULT_MAX_IMPORT_SIZE);
    if body::content_length(headers)?.is_some_and(|length| length > limit) {
        return Err(body::payload_too_large());
    }
    let permit = state.resources().import()?;
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let car = body::collect(body.take().unwrap_or_default(), limit).await?;
    state
        .repo_blocking(did.clone(), move |state| {
            let _permit = permit;
            transact(state, &did, |session| session.import(&car, active))
        })
        .await?;
    Ok(StatusCode::OK.into_response())
}
