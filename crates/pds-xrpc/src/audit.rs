//! The audit log in the server (`docs/SPEC.md` §20).
//!
//! The entry point records each audited method after it answered. A page
//! records its own actions. Every five minutes, and at shutdown, the
//! server signs a checkpoint over the entries since the last one and
//! publishes it as a record, so that it leaves the operator's control on
//! the firehose.

use std::cell::RefCell;
use std::future::Future;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pds_audit::entry::GENESIS;
use pds_audit::{Checkpoint, Draft, Entry, Signer, merkle};
use pds_storage::{AuditDb, CheckpointRow, StorageError};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::error::XrpcError;
use crate::state::AppState;
use crate::write::{Prepared, prepare_record, transact};

/// How often the server signs a checkpoint.
pub const CHECKPOINT_INTERVAL: Duration = Duration::from_mins(5);
/// The collection of a published checkpoint.
pub const CHECKPOINT_COLLECTION: &str = "io.linkjar.pds.audit.checkpoint";

/// The audit log of a server.
pub struct AuditLog {
    db: Mutex<AuditDb>,
    signer: Option<Signer>,
}

impl std::fmt::Debug for AuditLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditLog")
            .field("signer", &self.signer)
            .finish_non_exhaustive()
    }
}

fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn to_row(checkpoint: &Checkpoint) -> CheckpointRow {
    CheckpointRow {
        first: checkpoint.first,
        last: checkpoint.last,
        root: checkpoint.root.to_vec(),
        head: checkpoint.head.to_vec(),
        time: checkpoint.time.clone(),
        key_id: checkpoint.key_id.clone(),
        signature: checkpoint.signature.clone(),
    }
}

impl AuditLog {
    /// Opens the log of a data directory, in the file or in the shared
    /// tier that the layout names. Without a signer the log chains its
    /// entries and signs no checkpoint.
    pub fn open_in(
        layout: &pds_storage::DataLayout,
        signer: Option<Signer>,
    ) -> Result<Self, StorageError> {
        Ok(Self {
            db: Mutex::new(layout.audit()?),
            signer,
        })
    }

    /// Opens the log in the file at `path`, and creates it when it is not
    /// there.
    pub fn open(path: &Path, signer: Option<Signer>) -> Result<Self, StorageError> {
        Ok(Self {
            db: Mutex::new(AuditDb::open(path)?),
            signer,
        })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, AuditDb> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Appends an entry at the time `time` (ISO 8601). This call blocks.
    pub fn record(&self, time: &str, what: Draft) -> Result<u64, StorageError> {
        self.db().append(|last| {
            // The hash of a canonical entry is the hash of its bytes. An
            // entry before it that does not decode still gives its
            // number, so the log goes on; the verifier names the break.
            let (seq, prev) = match last {
                None => (1, GENESIS),
                Some((seq, bytes)) => match Entry::from_bytes(bytes) {
                    Ok(previous) => (previous.seq + 1, Sha256::digest(bytes).into()),
                    Err(_) => (seq + 1, GENESIS),
                },
            };
            let entry = Entry::following(seq, prev, time, what);
            let (bytes, hash) = entry.encode();
            (entry.seq, hash, bytes)
        })
    }

    /// Signs and stores a checkpoint over the entries since the last one.
    /// `None` when there is no new entry or no key. A key that is not the
    /// one of the last checkpoint is itself logged first. This call
    /// blocks.
    pub fn checkpoint(&self, time: &str) -> Result<Option<Checkpoint>, StorageError> {
        Ok(self
            .checkpoint_after(time)?
            .map(|(checkpoint, _)| checkpoint))
    }

    /// [`AuditLog::checkpoint`], with the root of the checkpoint before
    /// it: 32 zero bytes before the first.
    pub fn checkpoint_after(
        &self,
        time: &str,
    ) -> Result<Option<(Checkpoint, Vec<u8>)>, StorageError> {
        let Some(signer) = &self.signer else {
            return Ok(None);
        };
        let last = self.db().last_checkpoint()?;
        let covered = last.as_ref().map_or(0, |row| row.last);
        let previous_root = last
            .as_ref()
            .map_or_else(|| vec![0; 32], |row| row.root.clone());
        if self.db().last_seq()? == covered {
            return Ok(None);
        }
        if last
            .as_ref()
            .is_none_or(|row| row.key_id != signer.key_id())
        {
            let what = Draft {
                actor: "server".to_owned(),
                action: "audit.key".to_owned(),
                subject: Some(signer.key_id()),
                credential: Some("server".to_owned()),
                outcome: "ok".to_owned(),
                reason: last.map(|row| format!("the key before it was {}", row.key_id)),
                ..Draft::default()
            };
            self.record(time, what)?;
        }
        let db = self.db();
        db.remember_key(&signer.key_id(), &signer.public_key())?;
        let entries: Vec<Vec<u8>> = db
            .entries(covered + 1, db.last_seq()?)?
            .into_iter()
            .map(|(_, bytes)| bytes)
            .collect();
        let Some(checkpoint) = signer.checkpoint(covered + 1, &entries, time) else {
            return Ok(None);
        };
        db.insert_checkpoint(&to_row(&checkpoint))?;
        drop(db);
        Ok(Some((checkpoint, previous_root)))
    }

    /// How many entries the log has.
    pub fn len(&self) -> Result<u64, StorageError> {
        self.db().last_seq()
    }

    /// How many entries no checkpoint covers yet.
    pub fn unsigned(&self) -> Result<u64, StorageError> {
        let db = self.db();
        let covered = db.last_checkpoint()?.map_or(0, |row| row.last);
        Ok(db.last_seq()?.saturating_sub(covered))
    }

    /// The seconds since the last checkpoint was signed. `None` for a
    /// log without a checkpoint.
    pub fn checkpoint_age_s(&self, now_ms: i64) -> Result<Option<i64>, StorageError> {
        Ok(self.db().last_checkpoint()?.and_then(|row| {
            chrono::DateTime::parse_from_rfc3339(&row.time)
                .ok()
                .map(|time| (now_ms - time.timestamp_millis()) / 1000)
        }))
    }

    /// Whether the log has no entry yet.
    pub fn is_empty(&self) -> Result<bool, StorageError> {
        Ok(self.db().last_seq()? == 0)
    }

    /// The `limit` entries before `before`, the newest first, as the ops
    /// listener shows them. This call blocks.
    pub fn page(&self, before: Option<u64>, limit: u64) -> Result<Vec<Value>, StorageError> {
        let db = self.db();
        let last = db.last_seq()?;
        let to = before.map_or(last, |before| before.saturating_sub(1).min(last));
        let from = to.saturating_sub(limit.saturating_sub(1)).max(1);
        let signed = db
            .last_checkpoint()?
            .map_or(0, |checkpoint| checkpoint.last);
        let rows = if to == 0 {
            Vec::new()
        } else {
            db.entries(from, to)?
        };
        drop(db);
        Ok(rows
            .iter()
            .rev()
            .filter_map(|(seq, bytes)| {
                let entry = Entry::from_bytes(bytes).ok()?;
                let what = entry.what;
                Some(json!({
                    "seq": seq,
                    "time": entry.time,
                    "actor": what.actor,
                    "action": what.action,
                    "subject": what.subject,
                    "credential": what.credential,
                    "clientId": what.client_id,
                    "outcome": what.outcome,
                    "reason": what.reason,
                    "sidecar": what.sidecar,
                    "signed": *seq <= signed,
                }))
            })
            .collect())
    }

    /// The export bundle of the entries from `from` to `to` (SPEC §20.4):
    /// the entries, the hash before the first, the checkpoints that cover
    /// them, an inclusion proof for each covered entry, and the public
    /// key of this server. This call blocks.
    pub fn export(&self, from: u64, to: u64) -> Result<Value, StorageError> {
        let db = self.db();
        let from = from.max(1);
        let to = to.min(db.last_seq()?);
        let entries = db.entries(from, to)?;
        let anchor = if from > 1 {
            db.entry_hash(from - 1)?.unwrap_or_default()
        } else {
            GENESIS.to_vec()
        };
        let checkpoints = db.checkpoints(from, to)?;
        let mut proofs = Vec::new();
        for checkpoint in &checkpoints {
            // The proof needs every leaf of the checkpoint, also those
            // outside the range that was asked for.
            let leaves: Vec<[u8; 32]> = db
                .entries(checkpoint.first, checkpoint.last)?
                .iter()
                .map(|(_, bytes)| merkle::leaf(bytes))
                .collect();
            for (seq, _) in entries
                .iter()
                .filter(|(seq, _)| (checkpoint.first..=checkpoint.last).contains(seq))
            {
                let index = usize::try_from(seq - checkpoint.first).unwrap_or(usize::MAX);
                let path: Vec<String> = merkle::proof(&leaves, index)
                    .iter()
                    .map(|hash| encode(hash))
                    .collect();
                proofs.push(json!({ "seq": seq, "checkpoint": checkpoint.last, "path": path }));
            }
        }
        let keys: Vec<Value> = db
            .keys()?
            .iter()
            .map(|(id, public)| json!({ "keyId": id, "publicKey": encode(public) }))
            .collect();
        drop(db);
        Ok(json!({
            "version": 1,
            "anchor": encode(&anchor),
            "keys": keys,
            "entries": entries.iter().map(|(_, bytes)| encode(bytes)).collect::<Vec<_>>(),
            "checkpoints": checkpoints.iter().map(checkpoint_json).collect::<Vec<_>>(),
            "proofs": proofs,
        }))
    }
}

fn checkpoint_json(row: &CheckpointRow) -> Value {
    json!({
        "first": row.first,
        "last": row.last,
        "root": encode(&row.root),
        "head": encode(&row.head),
        "time": row.time,
        "keyId": row.key_id,
        "signature": encode(&row.signature),
    })
}

tokio::task_local! {
    static SUBJECT: RefCell<Option<String>>;
}

/// An operator on the ops listener, for the audit entries of the request
/// (SPEC §22.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acting {
    /// The operator: a DID, or `admin`.
    pub actor: String,
    /// `oauth` or `admin`.
    pub credential: &'static str,
    /// The OAuth client of the operator's session.
    pub client_id: Option<String>,
    /// The reason the operator gave.
    pub reason: Option<String>,
}

impl Acting {
    /// The entry of `action` by this operator through the ops listener.
    pub fn draft(&self, action: &str, subject: Option<String>, outcome: String) -> Draft {
        Draft {
            actor: self.actor.clone(),
            action: action.to_owned(),
            subject,
            credential: Some(self.credential.to_owned()),
            client_id: self.client_id.clone(),
            outcome,
            reason: self.reason.clone(),
            sidecar: Some("console".to_owned()),
            ..Draft::default()
        }
    }
}

tokio::task_local! {
    static ACTING: Acting;
}

/// Serves a request of the ops listener, so that the audit entry of the
/// method names the operator and not the credential the listener used.
pub async fn acting_as<F: Future>(who: Acting, serve: F) -> F::Output {
    ACTING.scope(who, serve).await
}

/// The operator of this task's request, on the ops listener.
pub fn acting() -> Option<Acting> {
    ACTING.try_with(Clone::clone).ok()
}

/// Names what the request of this task acts on, for its audit entry. A
/// handler calls it when the subject is not the caller.
pub fn set_subject(subject: &str) {
    let _ = SUBJECT.try_with(|slot| *slot.borrow_mut() = Some(subject.to_owned()));
}

/// Serves a request with a place for the subject of its audit entry, and
/// gives the subject that the handler named.
pub async fn with_subject<F: Future>(serve: F) -> (F::Output, Option<String>) {
    SUBJECT
        .scope(RefCell::new(None), async move {
            let output = serve.await;
            let subject = SUBJECT
                .try_with(|slot| slot.borrow_mut().take())
                .ok()
                .flatten();
            (output, subject)
        })
        .await
}

/// Whether a method is audited (SPEC §20.1): account and credential
/// changes, identity changes, invite administration and every admin
/// method. Reads and repository writes are not.
pub fn is_audited(nsid: &str) -> bool {
    nsid.starts_with("com.atproto.admin.")
        || matches!(
            nsid,
            "com.atproto.server.createAccount"
                | "com.atproto.server.deactivateAccount"
                | "com.atproto.server.activateAccount"
                | "com.atproto.server.requestAccountDelete"
                | "com.atproto.server.deleteAccount"
                | "com.atproto.server.requestPasswordReset"
                | "com.atproto.server.resetPassword"
                | "com.atproto.server.requestEmailUpdate"
                | "com.atproto.server.updateEmail"
                | "com.atproto.server.confirmEmail"
                | "com.atproto.server.createAppPassword"
                | "com.atproto.server.revokeAppPassword"
                | "com.atproto.server.createInviteCode"
                | "com.atproto.server.createInviteCodes"
                | "com.atproto.identity.updateHandle"
                | "com.atproto.identity.requestPlcOperationSignature"
                | "com.atproto.identity.signPlcOperation"
                | "com.atproto.identity.submitPlcOperation"
                | "com.atproto.repo.importRepo"
        )
}

/// Appends an entry. A log that cannot be written is an error in the
/// server's log: the action already happened.
pub async fn record(state: &AppState, what: Draft) {
    let Some(_) = state.audit() else {
        return;
    };
    let time = pds_sequencer::iso_millis(state.now_ms());
    let state = state.clone();
    let action = what.action.clone();
    let written =
        tokio::task::spawn_blocking(move || state.audit().map(|log| log.record(&time, what))).await;
    if !matches!(written, Ok(Some(Ok(_)))) {
        tracing::error!(%action, "an audit entry was not written");
    }
}

/// Appends the entry of an action on a page (SPEC §20.1): a sign-up or a
/// change of a sign-in method. `detail` is what the action was done with,
/// such as the provider.
pub async fn record_page(
    state: &AppState,
    action: &str,
    did: &str,
    device_id: &str,
    client_id: Option<&str>,
    detail: Option<&str>,
) {
    let what = Draft {
        actor: did.to_owned(),
        action: action.to_owned(),
        subject: Some(detail.map_or_else(|| did.to_owned(), |detail| format!("{did} {detail}"))),
        credential: Some("page".to_owned()),
        client_id: client_id.map(str::to_owned),
        device_id: Some(device_id.to_owned()),
        outcome: "ok".to_owned(),
        ..Draft::default()
    };
    record(state, what).await;
}

/// Signs a checkpoint when there are new entries, and publishes it as a
/// record of the operator's repository when one is configured.
pub async fn checkpoint(state: &AppState) {
    if state.audit().is_none() {
        return;
    }
    let time = pds_sequencer::iso_millis(state.now_ms());
    let made = {
        let state = state.clone();
        tokio::task::spawn_blocking(move || state.audit().map(|log| log.checkpoint_after(&time)))
            .await
    };
    let (checkpoint, previous_root) = match made {
        Ok(Some(Ok(Some(made)))) => made,
        Ok(Some(Ok(None)) | None) => return,
        _ => {
            tracing::error!("an audit checkpoint was not written");
            return;
        }
    };
    let Some(repo) = state.config().audit_repo.clone() else {
        return;
    };
    // The record of `lexicons/io/linkjar/pds/audit/checkpoint.json`. The
    // signature covers the range, the root, the chain hash, the time and
    // the key id.
    let bytes = |value: &[u8]| json!({ "$bytes": data_encoding::BASE64_NOPAD.encode(value) });
    let record = json!({
        "$type": CHECKPOINT_COLLECTION,
        "server": state.config().auth.service_did,
        "firstSeq": checkpoint.first,
        "lastSeq": checkpoint.last,
        "treeSize": checkpoint.last,
        "root": bytes(&checkpoint.root),
        "previousRoot": bytes(&previous_root),
        "chainHash": bytes(&checkpoint.head),
        "keyId": checkpoint.key_id,
        "signature": bytes(&checkpoint.signature),
        "createdAt": checkpoint.time,
    });
    let published = {
        let state = state.clone();
        tokio::task::spawn_blocking(move || publish(&state, &repo, &record)).await
    };
    if !matches!(published, Ok(Ok(()))) {
        tracing::error!(
            last = checkpoint.last,
            "an audit checkpoint was not published"
        );
    }
}

/// Writes the checkpoint record into the repository `did`. The commit
/// travels the firehose as any commit does.
fn publish(state: &AppState, did: &str, record: &Value) -> Result<(), XrpcError> {
    let writes = state
        .writes()
        .ok_or_else(|| XrpcError::internal("the server is read-only"))?;
    let fields: Map<String, Value> = record.as_object().cloned().unwrap_or_default();
    let write = prepare_record(
        writes,
        did,
        CHECKPOINT_COLLECTION,
        None,
        fields,
        Some(false),
        "$.record",
    )?;
    transact(state, did, |session| {
        session.process_writes(&[Prepared::Create(write)], None)
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(action: &str) -> Draft {
        Draft {
            actor: "admin".to_owned(),
            action: action.to_owned(),
            outcome: "ok".to_owned(),
            ..Draft::default()
        }
    }

    #[test]
    fn a_log_is_recorded_checkpointed_exported_and_verified() {
        let dir = std::env::temp_dir().join(format!("pds-audit-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit.sqlite");
        let log = AuditLog::open(&path, Signer::from_seed(&[3; 32])).unwrap();
        assert_eq!(
            log.checkpoint("t0").unwrap(),
            None,
            "an empty log has no checkpoint"
        );
        for n in 0..5 {
            log.record("2026-10-13T00:00:00.000Z", draft(&format!("a{n}")))
                .unwrap();
        }
        // The first checkpoint logs its key, then covers that entry too.
        let first = log.checkpoint("2026-10-13T00:05:00.000Z").unwrap().unwrap();
        assert_eq!((first.first, first.last), (1, 6));
        assert_eq!(log.checkpoint("2026-10-13T00:10:00.000Z").unwrap(), None);
        for n in 5..8 {
            log.record("2026-10-13T00:11:00.000Z", draft(&format!("a{n}")))
                .unwrap();
        }
        let second = log.checkpoint("2026-10-13T00:15:00.000Z").unwrap().unwrap();
        assert_eq!(
            (second.first, second.last),
            (7, 9),
            "the same key is not logged again"
        );

        let whole = log.export(1, u64::MAX).unwrap();
        let report = pds_audit::verify::verify(&whole);
        assert!(report.ok(), "{:?}", report.problems);
        assert_eq!(
            (report.entries, report.checkpoints, report.proofs),
            (9, 2, 9)
        );
        assert_eq!(report.roots_recomputed, 2);
        // A part of the log verifies by its proofs and its anchor.
        let part = log.export(3, 8).unwrap();
        let report = pds_audit::verify::verify(&part);
        assert!(report.ok(), "{:?}", report.problems);
        assert_eq!(
            (report.entries, report.checkpoints, report.proofs),
            (6, 2, 6)
        );
        assert_eq!(report.roots_recomputed, 0);

        // A new key is logged before it signs.
        drop(log);
        let log = AuditLog::open(&path, Signer::from_seed(&[4; 32])).unwrap();
        log.record("2026-10-13T00:20:00.000Z", draft("later"))
            .unwrap();
        let third = log.checkpoint("2026-10-13T00:25:00.000Z").unwrap().unwrap();
        assert_eq!((third.first, third.last), (10, 11));
        assert_ne!(third.key_id, second.key_id);
        let report = pds_audit::verify::verify(&log.export(1, u64::MAX).unwrap());
        assert!(report.ok(), "{:?}", report.problems);
        assert_eq!(report.checkpoints, 3, "the export carries both keys");
        // Without a key the log still chains.
        drop(log);
        let log = AuditLog::open(&path, None).unwrap();
        assert_eq!(log.record("t", draft("unsigned")).unwrap(), 12);
        assert_eq!(log.checkpoint("t").unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn account_changes_and_admin_methods_are_audited_and_reads_are_not() {
        for nsid in [
            "com.atproto.server.createAccount",
            "com.atproto.admin.updateSubjectStatus",
            "com.atproto.admin.getAccountInfo",
            "com.atproto.identity.updateHandle",
        ] {
            assert!(is_audited(nsid), "{nsid}");
        }
        for nsid in [
            "com.atproto.repo.createRecord",
            "com.atproto.sync.getRepo",
            "com.atproto.server.createSession",
            "app.bsky.feed.getTimeline",
        ] {
            assert!(!is_audited(nsid), "{nsid}");
        }
    }
}
