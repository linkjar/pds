//! The `verify` command (`docs/SPEC.md` §18.2).
//!
//! `verify` reads a data directory and writes nothing. For every actor it
//! loads the repository and reads the whole tree, and it names a tree that
//! references a node more than once. It rebuilds the tree from the record
//! index and compares the roots. It loads the signing key and checks the
//! commit signature with it. It checks that every blob a record references
//! has a row in the blob index and, unless the blob is taken down, a file in
//! the blob store. Without a blob store location it skips the file check and
//! says so.
//!
//! It then reads the event log and reports each break in its sequence
//! numbers. A break is not a difference, and it does not change the exit
//! code: a server that deletes the events of a deleted account leaves a
//! break in a sound log. An event log that cannot be read is a difference
//! with the subject `sequencer`.

use std::collections::BTreeMap;
use std::fmt;

use cid::Cid;
use pds_blob::DiskBlobStore;
use pds_identity::{Algorithm, Keypair};
use pds_repo::mst::tree::Tree;
use pds_repo::mst::walk::{Guard, read_tree};
use pds_repo::{Commit, MemoryBlockStore, Repo, RepoError, TreeDefect};
use pds_storage::{AccountDb, ActorDb, SeqGap, SequencerDb, StorageError};

use crate::config::Config;

/// The number of breaks in the event log that a report lists.
const BREAKS_LISTED: usize = 5;

/// One thing `verify` found wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The actor's DID, or `sequencer`.
    pub subject: String,
    /// A stable name for the kind of finding.
    pub kind: &'static str,
    /// What was found.
    pub detail: String,
}

/// What `verify` read and what it found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Actors in the account database.
    pub actors: u64,
    /// Actors whose repository was read.
    pub repositories: u64,
    /// Records in those repositories.
    pub records: u64,
    /// Blob references checked.
    pub blob_references: u64,
    /// Events in the event log.
    pub events: u64,
    /// Breaks in the sequence numbers of the event log. A break is not a
    /// difference.
    pub sequence_breaks: u64,
    /// The first breaks, at most five.
    pub first_sequence_breaks: Vec<SeqGap>,
    /// Whether the blob store was checked: false when none is configured.
    pub blob_store_checked: bool,
    /// The differences. An empty list is a clean data directory.
    pub findings: Vec<Finding>,
}

impl Report {
    fn find(&mut self, subject: &str, kind: &'static str, detail: impl Into<String>) {
        self.findings.push(Finding {
            subject: subject.to_owned(),
            kind,
            detail: detail.into(),
        });
    }

    /// The report as JSON, for a tool.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "actors": self.actors,
            "repositories": self.repositories,
            "records": self.records,
            "blobReferences": self.blob_references,
            "events": self.events,
            "sequenceBreaks": self.sequence_breaks,
            "firstSequenceBreaks": self.first_sequence_breaks.iter().map(|gap| serde_json::json!({
                "after": gap.after, "before": gap.before,
            })).collect::<Vec<_>>(),
            "blobStoreChecked": self.blob_store_checked,
            "differences": self.findings.len(),
            "findings": self.findings.iter().map(|f| serde_json::json!({
                "subject": f.subject, "kind": f.kind, "detail": f.detail,
            })).collect::<Vec<_>>(),
        })
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "actors            {}", self.actors)?;
        writeln!(f, "repositories read {}", self.repositories)?;
        writeln!(f, "records           {}", self.records)?;
        writeln!(f, "blob references   {}", self.blob_references)?;
        writeln!(f, "events            {}", self.events)?;
        writeln!(f, "sequence breaks   {}", self.sequence_breaks)?;
        for gap in &self.first_sequence_breaks {
            writeln!(f, "  no event between {} and {}", gap.after, gap.before)?;
        }
        if !self.blob_store_checked {
            writeln!(
                f,
                "blob store        not checked: PDS_BLOBSTORE_DISK_LOCATION is not set"
            )?;
        }
        writeln!(f, "differences       {}", self.findings.len())?;
        for finding in &self.findings {
            writeln!(
                f,
                "  {} {}: {}",
                finding.subject, finding.kind, finding.detail
            )?;
        }
        Ok(())
    }
}

/// The repository key of a record URI: `<collection>/<rkey>`.
fn record_key<'a>(uri: &'a str, did: &str) -> Option<&'a str> {
    uri.strip_prefix("at://")?
        .strip_prefix(did)?
        .strip_prefix('/')
}

fn tree_finding(error: &RepoError) -> &'static str {
    match error {
        RepoError::Tree {
            defect: TreeDefect::SharedNode,
            ..
        } => "tree-shared-node",
        RepoError::MissingBlock(_) => "tree-block-missing",
        _ => "tree-invalid",
    }
}

/// The key file loads, and the commit is signed with it.
fn check_key(config: &Config, did: &str, commit: &Commit, report: &mut Report) {
    let key = config
        .layout
        .read_actor_key(did)
        .map_err(|e| e.to_string())
        .and_then(|bytes| {
            Keypair::from_secret_bytes(Algorithm::Secp256k1, &bytes).map_err(|e| e.to_string())
        });
    match key {
        Ok(key) => {
            let public = key.public_key();
            let signed =
                commit.verify(|message, signature| public.verify(message, signature).is_ok());
            if signed.is_err() {
                report.find(
                    did,
                    "signature-invalid",
                    "the commit is not signed by the key in the key file",
                );
            }
        }
        Err(reason) => report.find(did, "key-unreadable", reason),
    }
}

/// The tree is valid and holds what the record index holds, the record
/// index gives the commit's root when written into an empty tree, and every
/// record block is present. `index` maps repository keys to record CIDs.
fn check_tree(
    store: &ActorDb,
    did: &str,
    commit: &Commit,
    index: &BTreeMap<String, String>,
    report: &mut Report,
) -> Result<(), StorageError> {
    let budget = usize::try_from(store.block_count()?).unwrap_or(usize::MAX);
    match read_tree(store, &commit.data, &mut Guard::new(budget)) {
        Ok(tree) => {
            let found: BTreeMap<String, String> = tree
                .leaves
                .iter()
                .map(|(key, cid)| (key.clone(), cid.to_string()))
                .collect();
            if &found != index {
                let only_tree = found
                    .iter()
                    .filter(|(k, v)| index.get(*k) != Some(*v))
                    .count();
                let only_index = index
                    .iter()
                    .filter(|(k, v)| found.get(*k) != Some(*v))
                    .count();
                report.find(
                    did,
                    "record-index-mismatch",
                    format!("{only_tree} tree entries are not in the record index as written; {only_index} index rows are not in the tree as written"),
                );
            }
        }
        Err(error) => report.find(did, tree_finding(&error), error.to_string()),
    }

    let mut fresh = Tree::empty(MemoryBlockStore::new());
    let mut recompute = || -> Result<Cid, String> {
        for (key, cid) in index {
            let cid = Cid::try_from(cid.as_str()).map_err(|e| format!("record CID {cid}: {e}"))?;
            fresh.add(key, cid).map_err(|e| e.to_string())?;
        }
        fresh.root().map_err(|e| e.to_string())
    };
    match recompute() {
        Ok(root) if root == commit.data => {}
        Ok(root) => report.find(
            did,
            "root-recompute-mismatch",
            format!(
                "the record index gives {root}; the commit has {}",
                commit.data
            ),
        ),
        Err(reason) => report.find(did, "root-recompute-failed", reason),
    }

    let record_cids: Vec<Cid> = index
        .values()
        .filter_map(|cid| Cid::try_from(cid.as_str()).ok())
        .collect();
    let missing = store.missing_blocks(&record_cids)?;
    if let Some(first) = missing.first() {
        report.find(
            did,
            "record-block-missing",
            format!(
                "{} record blocks are absent, the first {first}",
                missing.len()
            ),
        );
    }
    Ok(())
}

/// Every blob a record references is in the blob index, and in the blob
/// store unless it is taken down.
fn check_blobs(
    store: &ActorDb,
    blobs: Option<&DiskBlobStore>,
    did: &str,
    report: &mut Report,
) -> Result<(), StorageError> {
    let mut checked: Option<String> = None;
    for (blob, uri) in store.all_record_blobs()? {
        report.blob_references += 1;
        if checked.as_deref() == Some(blob.as_str()) {
            continue;
        }
        checked = Some(blob.clone());
        match store.blob_takedown(&blob)? {
            None => report.find(
                did,
                "blob-unindexed",
                format!("{blob}, referenced by {uri}, has no row in the blob index"),
            ),
            // A taken-down blob is in quarantine and not in the store.
            Some(true) => {}
            Some(false) => match blobs.map(|blobs| blobs.has(did, &blob)) {
                None | Some(Ok(true)) => {}
                Some(Ok(false)) => report.find(
                    did,
                    "blob-missing",
                    format!("{blob}, referenced by {uri}, is not in the blob store"),
                ),
                Some(Err(error)) => report.find(
                    did,
                    "blob-unreadable",
                    format!("{blob}, referenced by {uri}: {error}"),
                ),
            },
        }
    }
    Ok(())
}

/// Checks one actor. A storage failure is the actor's finding and does not
/// stop the run.
fn verify_actor(
    config: &Config,
    accounts: &AccountDb,
    blobs: Option<&DiskBlobStore>,
    did: &str,
    report: &mut Report,
) -> Result<(), StorageError> {
    let path = config.layout.actor_db(did);
    if !path.is_file() {
        report.find(
            did,
            "store-missing",
            format!("{} does not exist", path.display()),
        );
        return Ok(());
    }
    let store = ActorDb::open(&path)?;
    let Some(root) = store.root()? else {
        report.find(did, "root-missing", "the store has no repository root");
        return Ok(());
    };
    match accounts.repo_root(did)? {
        None => report.find(did, "root-mismatch", "account.sqlite has no repo_root row"),
        Some((cid, rev)) if cid != root.cid.to_string() || rev != root.rev => report.find(
            did,
            "root-mismatch",
            format!(
                "account.sqlite has {cid} at {rev}; the store has {} at {}",
                root.cid, root.rev
            ),
        ),
        Some(_) => {}
    }
    let repo = match Repo::load(&store, root.cid) {
        Ok(repo) => repo,
        Err(error) => {
            report.find(did, "commit-invalid", error.to_string());
            return Ok(());
        }
    };
    report.repositories += 1;
    let commit = repo.commit();
    if commit.did.as_str() != did {
        report.find(
            did,
            "commit-did",
            format!("the commit names {}", commit.did),
        );
    }
    // The revision in `repo_root` is not compared with the commit's: the
    // Reference gives the root a new revision when it imports a repository
    // whose commit it already holds (parity scenario `default/15-import`).
    check_key(config, did, commit, report);

    let rows = store.all_records()?;
    report.records += rows.len() as u64;
    let mut index: BTreeMap<String, String> = BTreeMap::new();
    for (uri, cid) in &rows {
        match record_key(uri, did) {
            Some(key) => {
                index.insert(key.to_owned(), cid.clone());
            }
            None => report.find(
                did,
                "record-uri",
                format!("{uri} is not a record of this repository"),
            ),
        }
    }
    check_tree(&store, did, commit, &index, report)?;
    check_blobs(&store, blobs, did, report)
}

/// Counts the events and the breaks in their sequence numbers.
fn read_sequencer(config: &Config, report: &mut Report) -> Result<(), StorageError> {
    let sequencer = SequencerDb::open(&config.layout.sequencer_db)?;
    report.events = sequencer.event_count()?;
    let gaps = sequencer.gaps()?;
    report.sequence_breaks = gaps.len() as u64;
    report.first_sequence_breaks = gaps.into_iter().take(BREAKS_LISTED).collect();
    Ok(())
}

/// Verifies the data directory that `config` names.
///
/// An account database that cannot be read is the error. A failure of any
/// other store is a finding, so the report of the other stores is kept.
pub fn verify(config: &Config) -> Result<Report, StorageError> {
    let mut report = Report::default();
    let accounts = AccountDb::open(&config.layout.account_db)?;
    let blobs = config.blob_dir.clone().map(DiskBlobStore::new);
    report.blob_store_checked = blobs.is_some();
    for actor in accounts.all_actors()? {
        report.actors += 1;
        if let Err(error) = verify_actor(config, &accounts, blobs.as_ref(), &actor.did, &mut report)
        {
            report.find(&actor.did, "store-unreadable", error.to_string());
        }
    }
    if let Err(error) = read_sequencer(config, &mut report) {
        report.find("sequencer", "store-unreadable", error.to_string());
    }
    Ok(report)
}
