//! The write path: from a validated request to a stored and sequenced
//! commit (`docs/SPEC.md` §6.3, §6.4, §7.4).
//!
//! The steps and their order are the Reference's (`repo/prepare.ts`,
//! `actor-store/repo/transactor.ts` and `actor-store/blob/transactor.ts` at
//! the pin), with one difference: the Reference appends the event to the log
//! before the actor transaction commits, and this server appends it after.
//! A crash between the two leaves a repository whose head is ahead of the
//! log; a note in `ops.sqlite` names it, and the startup repair emits
//! `#sync` (§7.4).

use cid::Cid;
use pds_lexicon::{Corpus, validate_data_model, validate_record_key};
use pds_repo::{Repo, RepoError, Write};
use pds_sequencer::{CommitOp, commit_event, sync_event};
use pds_storage::{ActorDb, ActorTxn, Backlink, SeqInsert, fail};
use pds_types::content::cid_for_cbor;
use pds_types::{AtUri, Data, Did, Limits, RecordKey, Tid};
use serde_json::{Map, Value};

use crate::error::XrpcError;
use crate::state::{AppState, WriteState};

/// What a request says about the record or commit it replaces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Swap {
    /// The request says nothing.
    #[default]
    Unset,
    /// The request says there is none.
    Absent,
    /// The request names it.
    Cid(Cid),
}

/// A blob that a record references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobUse {
    /// The blob's CID.
    pub cid: String,
    /// The media type the record states.
    pub mime_type: String,
    /// The size the record states.
    pub size: i64,
}

/// A record that is ready to be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordWrite {
    /// `at://<did>/<collection>/<rkey>`.
    pub uri: String,
    /// The collection.
    pub collection: String,
    /// The record key.
    pub rkey: String,
    /// The CID of the record block.
    pub cid: Cid,
    /// The record block: canonical DAG-CBOR.
    pub bytes: Vec<u8>,
    /// The record.
    pub data: Data,
    /// The blobs the record references.
    pub blobs: Vec<BlobUse>,
    /// `valid` when the record was checked against its lexicon, `unknown`
    /// when the server has no lexicon for it, `None` when the client turned
    /// validation off.
    pub validation_status: Option<&'static str>,
    /// What the request says about the record it replaces.
    pub swap: Swap,
}

/// One write of a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prepared {
    /// A record at a key that has none.
    Create(RecordWrite),
    /// A record that replaces the one at its key.
    Update(RecordWrite),
    /// The removal of the record at a key.
    Delete {
        /// `at://<did>/<collection>/<rkey>`.
        uri: String,
        /// The collection.
        collection: String,
        /// The record key.
        rkey: String,
        /// What the request says about the record it removes.
        swap: Swap,
    },
}

impl Prepared {
    /// The AT URI the write is about.
    pub fn uri(&self) -> &str {
        match self {
            Self::Create(record) | Self::Update(record) => &record.uri,
            Self::Delete { uri, .. } => uri,
        }
    }

    fn path(&self) -> String {
        match self {
            Self::Create(record) | Self::Update(record) => {
                format!("{}/{}", record.collection, record.rkey)
            }
            Self::Delete {
                collection, rkey, ..
            } => format!("{collection}/{rkey}"),
        }
    }

    const fn swap(&self) -> &Swap {
        match self {
            Self::Create(record) | Self::Update(record) => &record.swap,
            Self::Delete { swap, .. } => swap,
        }
    }
}

/// The AT URI of a record.
pub fn record_uri(did: &str, collection: &str, rkey: &str) -> String {
    format!("at://{did}/{collection}/{rkey}")
}

/// A JSON value as JavaScript writes it into a template string.
fn js_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_owned(),
        Value::Array(items) => items.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_owned(),
        other => other.to_string(),
    }
}

fn blob_uses(value: &Data, found: &mut Vec<BlobUse>) -> Result<(), XrpcError> {
    match value {
        Data::Array(items) => items.iter().try_for_each(|item| blob_uses(item, found)),
        Data::Object(fields) => {
            let text = |key: &str| match fields.get(key) {
                Some(Data::String(text)) => Some(text),
                _ => None,
            };
            if text("$type").is_some_and(|tag| tag == "blob")
                && let (Some(Data::Link(link)), Some(mime_type), Some(Data::Integer(size))) =
                    (fields.get("ref"), text("mimeType"), fields.get("size"))
            {
                found.push(BlobUse {
                    cid: link.0.to_string(),
                    mime_type: mime_type.clone(),
                    size: *size,
                });
                return Ok(());
            }
            // The form of before 2023: `{cid, mimeType}` and nothing else.
            if fields.len() == 2
                && let (Some(cid), Some(_)) = (text("cid"), text("mimeType"))
                && cid.parse::<Cid>().is_ok()
            {
                return Err(XrpcError::invalid_request(format!(
                    "Legacy blobs are not allowed ({cid})"
                )));
            }
            fields
                .values()
                .try_for_each(|field| blob_uses(field, found))
        }
        _ => Ok(()),
    }
}

/// The CID of every blob a stored record references, in either form: the
/// typed reference and the form of before 2023.
pub fn blob_cids(value: &Data) -> Vec<String> {
    fn walk(value: &Data, found: &mut Vec<String>) {
        match value {
            Data::Array(items) => {
                for item in items {
                    walk(item, found);
                }
            }
            Data::Object(fields) => {
                let text = |key: &str| match fields.get(key) {
                    Some(Data::String(text)) => Some(text),
                    _ => None,
                };
                if text("$type").is_some_and(|tag| tag == "blob")
                    && let Some(Data::Link(link)) = fields.get("ref")
                {
                    found.push(link.0.to_string());
                } else if fields.len() == 2
                    && let (Some(cid), Some(_)) = (text("cid"), text("mimeType"))
                    && cid.parse::<Cid>().is_ok()
                {
                    found.push(cid.clone());
                } else {
                    for field in fields.values() {
                        walk(field, found);
                    }
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    walk(value, &mut found);
    found
}

/// Prepares a record for a create or an update: the `$type`, the record
/// key, validation in the mode the client asked for, the blobs and the CID.
///
/// `at` is the JSON path of the record in the request body, for the error
/// messages: `$.record`, or `$.writes[0].record` in a batch.
pub fn prepare_record(
    state: &WriteState,
    did: &str,
    collection: &str,
    rkey: Option<&str>,
    mut record: Map<String, Value>,
    validate: Option<bool>,
    at: &str,
) -> Result<RecordWrite, XrpcError> {
    match record.get("$type") {
        None => {
            record.insert("$type".to_owned(), Value::String(collection.to_owned()));
        }
        Some(Value::String(tag)) if tag == collection => {}
        Some(other) => {
            return Err(XrpcError::invalid_request(format!(
                "Invalid $type: expected {collection}, got {}",
                js_string(other)
            )));
        }
    }
    if let Some(rkey) = rkey
        && RecordKey::parse(rkey).is_err()
    {
        return Err(XrpcError::invalid_request(format!(
            "Invalid record key: {rkey}"
        )));
    }
    // The Reference draws a key for every write, used or not, so that a
    // batch of creates gets ascending keys.
    let drawn = state.tids().next();
    let rkey = rkey.map_or_else(|| drawn.as_str().to_owned(), str::to_owned);

    // The value moves into the data model without a JSON round trip. The
    // body came in under the method's limit, which is below the size limit
    // of a JSON record; the other limits are checked on the value, as
    // `Data::from_json_slice` checks them.
    let data: Data = serde_json::from_value(Value::Object(record))
        .map_err(|e| XrpcError::invalid_request(e.to_string()))?;
    data.check_limits(Limits::JSON_RECORD)
        .map_err(|e| XrpcError::invalid_request(e.to_string()))?;

    let corpus = Corpus::bundled();
    let validation_status = if validate == Some(false) {
        None
    } else if let Some(lexicon) = corpus.record(collection) {
        validate_record_key(&lexicon.key, &rkey).map_err(|e| {
            XrpcError::invalid_request(format!(
                "Invalid record key for {collection}: {}",
                e.reference_message(&Data::String(rkey.clone()), "$")
            ))
        })?;
        corpus.validate_record(&data).map_err(|e| {
            XrpcError::invalid_request(format!(
                "Invalid {collection} record: {}",
                e.reference_message(&data, at)
            ))
        })?;
        Some("valid")
    } else if validate == Some(true) {
        return Err(XrpcError::invalid_request(format!(
            "Unknown lexicon type: {collection}"
        )));
    } else {
        Some("unknown")
    };
    // The rules of every record, whatever the mode (SPEC §6.3).
    validate_data_model(&data)
        .map_err(|e| XrpcError::invalid_request(e.reference_message(&data, at)))?;

    let mut blobs = Vec::new();
    blob_uses(&data, &mut blobs)?;
    let bytes = data.to_dag_cbor();
    Ok(RecordWrite {
        uri: record_uri(did, collection, &rkey),
        collection: collection.to_owned(),
        rkey,
        cid: cid_for_cbor(&bytes),
        bytes,
        data,
        blobs,
        validation_status,
        swap: Swap::Unset,
    })
}

/// The links a record holds that the server indexes: the subject of a
/// follow or a block, and the subject of a like or a repost.
pub fn backlinks(uri: &str, record: &Data) -> Vec<Backlink> {
    let text = |value: Option<&Data>| match value {
        Some(Data::String(text)) => Some(text.clone()),
        _ => None,
    };
    let link = |path: &'static str, link_to: String| {
        vec![Backlink {
            uri: uri.to_owned(),
            path,
            link_to,
        }]
    };
    match record.type_name() {
        Some("app.bsky.graph.follow" | "app.bsky.graph.block") => text(record.get("subject"))
            .filter(|subject| Did::parse(subject).is_ok())
            .map_or_else(Vec::new, |subject| link("subject", subject)),
        Some("app.bsky.feed.like" | "app.bsky.feed.repost") => {
            text(record.get("subject").and_then(|subject| subject.get("uri")))
                .filter(|subject| AtUri::parse(subject).is_ok())
                .map_or_else(Vec::new, |subject| link("subject.uri", subject))
        }
        _ => Vec::new(),
    }
}

/// A commit that was stored and sequenced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    /// The CID of the commit.
    pub cid: Cid,
    /// The revision.
    pub rev: String,
}

/// The work of one request on one actor store, inside a write transaction.
pub struct Session<'a> {
    state: &'a AppState,
    writes: &'a WriteState,
    did: &'a str,
    txn: &'a ActorTxn<'a>,
    now: String,
    /// The event of the commit, when the work made one.
    event: Option<(SeqInsert, Committed)>,
    /// Staged uploads to move to their place: key and CID.
    permanent: Vec<(String, String)>,
    /// Blobs whose files go when the transaction is durable.
    dereferenced: Vec<String>,
}

fn parse_cid(text: &str) -> Result<Cid, XrpcError> {
    text.parse::<Cid>().map_err(XrpcError::internal)
}

fn repo_error(error: RepoError) -> XrpcError {
    match error {
        // The Reference answers 500 here; the owner decided 400, a
        // deliberate difference (SPEC §2.3).
        RepoError::KeyExists(key) => {
            XrpcError::invalid_request(format!("Record already exists: {key}"))
        }
        RepoError::TooLarge(_) => {
            XrpcError::invalid_request("Too many writes. Max event size: 2MB")
        }
        other => XrpcError::internal(other),
    }
}

impl Session<'_> {
    /// Makes the transaction end with `event`, which states the head
    /// `committed`: the note, the event and the account root follow the
    /// order of a commit (SPEC §7.4).
    pub(crate) fn state_head(&mut self, event: SeqInsert, committed: Committed) {
        self.event = Some((event, committed));
    }

    /// The transaction, for reads that belong to the same work.
    pub const fn txn(&self) -> &ActorTxn<'_> {
        self.txn
    }

    /// The server state.
    pub const fn state(&self) -> &AppState {
        self.state
    }

    /// The write path.
    pub const fn writes(&self) -> &WriteState {
        self.writes
    }

    /// The time of this work, as the log writes it.
    pub fn now(&self) -> &str {
        &self.now
    }

    /// The account.
    pub const fn did(&self) -> &str {
        self.did
    }

    fn swap_check(write: &Prepared, current: Option<&Cid>) -> Result<(), XrpcError> {
        let refuse = || {
            XrpcError::bad_request(
                "InvalidSwap",
                format!(
                    "Record was at {}",
                    current.map_or_else(|| "null".to_owned(), Cid::to_string)
                ),
            )
        };
        match (write, write.swap()) {
            // A create has no record to replace; an update and a delete
            // have one.
            (Prepared::Create(_), Swap::Cid(_))
            | (Prepared::Update(_) | Prepared::Delete { .. }, Swap::Absent) => Err(refuse()),
            (_, Swap::Absent) if current.is_some() => Err(refuse()),
            (_, Swap::Cid(swap)) if current != Some(swap) => Err(refuse()),
            (_, Swap::Unset | Swap::Absent | Swap::Cid(_)) => Ok(()),
        }
    }

    /// Builds the commit of `writes`, stores it in the transaction and
    /// keeps its event for the end of the work.
    #[allow(clippy::too_many_lines)] // the steps of the Reference's `processWrites`, in order
    pub fn process_writes(
        &mut self,
        writes: &[Prepared],
        swap_commit: Option<&Cid>,
    ) -> Result<Committed, XrpcError> {
        if writes.len() > pds_repo::repo::MAX_WRITES {
            return Err(XrpcError::invalid_request("Too many writes. Max: 200"));
        }
        let txn = self.txn;
        let root = txn.root()?.ok_or_else(|| {
            XrpcError::invalid_request(format!("No repo root found for {}", self.did))
        })?;
        if swap_commit.is_some_and(|swap| *swap != root.cid) {
            return Err(XrpcError::bad_request(
                "InvalidSwap",
                format!("Commit was at {}", root.cid),
            ));
        }

        let mut ops = Vec::with_capacity(writes.len());
        let mut new_records = Vec::new();
        let mut touched = Vec::new();
        for write in writes {
            let current = txn
                .record_cid(write.uri())?
                .as_deref()
                .map(parse_cid)
                .transpose()?;
            let (action, cid) = match write {
                Prepared::Create(record) => ("create", Some(record.cid)),
                Prepared::Update(record) => ("update", Some(record.cid)),
                Prepared::Delete { .. } => ("delete", None),
            };
            new_records.extend(cid);
            if !matches!(write, Prepared::Create(_)) {
                touched.push(write.uri().to_owned());
            }
            Self::swap_check(write, current.as_ref())?;
            ops.push(CommitOp {
                action,
                path: write.path(),
                cid,
                prev: current,
            });
        }

        let actor: &ActorDb = txn;
        let repo = Repo::load(actor, root.cid).map_err(XrpcError::internal)?;
        let current_rev = Tid::parse(&root.rev).map_err(XrpcError::internal)?;
        let rev = self.writes.tids().next_after(&current_rev);
        let key = self.state.actor_key(self.did)?;
        let paths: Vec<String> = writes.iter().map(Prepared::path).collect();
        let repo_writes: Vec<Write<'_>> = writes
            .iter()
            .zip(&paths)
            .map(|(write, key)| match write {
                Prepared::Create(record) => Write::Create {
                    key,
                    record: &record.bytes,
                },
                Prepared::Update(record) => Write::Update {
                    key,
                    record: &record.bytes,
                },
                Prepared::Delete { .. } => Write::Delete { key },
            })
            .collect();
        let mut commit = repo
            .format_commit(rev, &repo_writes, |bytes| Ok(key.sign(bytes).to_vec()))
            .map_err(repo_error)?;

        // A block that another record still holds stays.
        let removed: Vec<Cid> = commit.removed_cids.iter().copied().collect();
        for kept in txn.duplicate_record_cids(&removed, &touched)? {
            commit.removed_cids.remove(&parse_cid(&kept)?);
        }
        // A record whose block the store already had is not among the new
        // blocks, and the event still carries it.
        for cid in &new_records {
            if !commit.relevant_blocks.contains(cid)
                && let Some(bytes) = txn.block(cid)?
            {
                commit.relevant_blocks.insert(*cid, bytes);
            }
        }

        let rev = commit.rev.as_str().to_owned();
        txn.update_root(self.did, &commit.cid, &rev, &self.now, false)?;
        txn.put_blocks(&commit.new_blocks, &rev)?;
        let removed: Vec<Cid> = commit.removed_cids.iter().copied().collect();
        txn.delete_blocks(&removed)?;

        for write in writes {
            match write {
                Prepared::Create(record) | Prepared::Update(record) => {
                    txn.index_record(
                        &record.uri,
                        &record.cid.to_string(),
                        &record.collection,
                        &record.rkey,
                        &rev,
                        &self.now,
                    )?;
                    if matches!(write, Prepared::Update(_)) {
                        txn.remove_backlinks(&record.uri)?;
                    }
                    txn.add_backlinks(&backlinks(&record.uri, &record.data))?;
                }
                Prepared::Delete { uri, .. } => txn.delete_record(uri)?,
            }
        }
        self.process_blobs(writes)?;

        let committed = Committed {
            cid: commit.cid,
            rev,
        };
        let event = commit_event(self.did, &commit, &ops, &self.now);
        self.event = Some((event, committed.clone()));
        Ok(committed)
    }

    /// Removes the references of deleted and replaced records, drops the
    /// blobs nothing references any more, and ties each blob of a new
    /// record to it (SPEC §6.4).
    fn process_blobs(&mut self, writes: &[Prepared]) -> Result<(), XrpcError> {
        let txn = self.txn;
        let replaced: Vec<String> = writes
            .iter()
            .filter(|write| !matches!(write, Prepared::Create(_)))
            .map(|write| write.uri().to_owned())
            .collect();
        let removed = txn.delete_record_blobs(&replaced)?;
        if !removed.is_empty() {
            let mut keep = txn.referenced_blobs(&removed)?;
            for write in writes {
                if let Prepared::Create(record) | Prepared::Update(record) = write {
                    keep.extend(record.blobs.iter().map(|blob| blob.cid.clone()));
                }
            }
            let mut gone: Vec<String> = removed
                .into_iter()
                .filter(|cid| !keep.contains(cid))
                .collect();
            gone.sort();
            gone.dedup();
            txn.delete_blobs(&gone)?;
            self.dereferenced.extend(gone);
        }
        for write in writes {
            if let Prepared::Create(record) | Prepared::Update(record) = write {
                for blob in &record.blobs {
                    txn.associate_blob(&blob.cid, &record.uri)?;
                    self.make_permanent(blob)?;
                }
            }
        }
        Ok(())
    }

    /// Checks that the store holds the blob a record references, and that
    /// the record states its type and size. An upload that still waits is
    /// made permanent.
    pub fn make_permanent(&mut self, blob: &BlobUse) -> Result<(), XrpcError> {
        let found = self.txn.stored_blob(&blob.cid)?.ok_or_else(|| {
            XrpcError::bad_request("BlobNotFound", format!("Could not find blob: {}", blob.cid))
        })?;
        let Some(temp_key) = found.temp_key else {
            return Ok(());
        };
        if blob.mime_type != found.mime_type {
            return Err(XrpcError::bad_request(
                "InvalidMimeType",
                format!(
                    "Referenced Mimetype does not match stored blob. Expected: {}, Got: {}",
                    found.mime_type, blob.mime_type
                ),
            ));
        }
        if blob.size != found.size {
            return Err(XrpcError::bad_request(
                "InvalidSize",
                format!(
                    "Referenced Size does not match stored blob. Expected: {}, Got: {}",
                    found.size, blob.size
                ),
            ));
        }
        self.txn.clear_temp_key(&temp_key)?;
        self.permanent.push((temp_key, blob.cid.clone()));
        Ok(())
    }
}

/// Runs `work` in a write transaction on the actor store of `did` and
/// finishes it: the commit becomes durable, then its event is appended to
/// the log. One call runs at a time for a repository.
///
/// When `work` fails nothing is written. This call blocks.
pub fn transact<T>(
    state: &AppState,
    did: &str,
    work: impl FnOnce(&mut Session<'_>) -> Result<T, XrpcError>,
) -> Result<T, XrpcError> {
    let writes = state
        .writes()
        .ok_or_else(|| XrpcError::internal("the server is read-only"))?;
    let lock = writes.repo_lock(did);
    let _one_writer = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut db = writes.take_store(did, state.layout())?;
    let result = transact_on(state, writes, did, &mut db, work);
    // A connection whose work failed is closed: its transaction rolled
    // back, and the next write starts from the file.
    if result.is_ok() {
        writes.keep_store(did, db);
    }
    result
}

/// The work of [`transact`] on an open store.
fn transact_on<T>(
    state: &AppState,
    writes: &WriteState,
    did: &str,
    db: &mut ActorDb,
    work: impl FnOnce(&mut Session<'_>) -> Result<T, XrpcError>,
) -> Result<T, XrpcError> {
    let txn = db.begin()?;
    let mut session = Session {
        state,
        writes,
        did,
        txn: &txn,
        now: writes.sequencer().now(),
        event: None,
        permanent: Vec::new(),
        dereferenced: Vec::new(),
    };
    let out = work(&mut session)?;
    let Session {
        event,
        permanent,
        dereferenced,
        ..
    } = session;

    // A blob is in its place before the row that calls it permanent is
    // durable. The move is safe to repeat.
    if let Some(blobs) = state.blobs() {
        for (key, cid) in &permanent {
            blobs
                .make_permanent(did, key, cid)
                .map_err(XrpcError::internal)?;
        }
    } else if !permanent.is_empty() {
        return Err(XrpcError::internal("no blob store is configured"));
    }

    let Some((event, committed)) = event else {
        txn.commit()?;
        return Ok(out);
    };
    writes.with_ops(|ops| ops.note_commit(did, &committed.rev))?;
    fail::point("commit.before-store").map_err(XrpcError::internal)?;
    txn.commit()?;
    // From here the commit is durable. A failure below leaves the note,
    // and the repair at the next start emits `#sync`.
    fail::point("commit.stored").map_err(XrpcError::internal)?;
    writes.sequencer().sequence(std::slice::from_ref(&event))?;
    fail::point("commit.sequenced").map_err(XrpcError::internal)?;
    if let Err(error) = writes.with_ops(|ops| ops.clear_commit(did, &committed.rev)) {
        tracing::warn!(%did, %error, "the note of a sequenced commit was not cleared");
    }
    let now = writes.sequencer().now();
    if let Err(error) = writes.with_accounts(|accounts| {
        accounts.update_repo_root(did, &committed.cid.to_string(), &committed.rev, &now)
    }) {
        tracing::error!(%did, %error, "failed to update account root");
    }
    if let Some(blobs) = state.blobs() {
        for cid in &dereferenced {
            if let Err(error) = blobs.delete(did, cid) {
                tracing::error!(%did, %cid, %error, "could not delete a blob from the store");
            }
        }
    }
    Ok(out)
}

/// What the startup repair did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Repaired {
    /// Notes that named a commit the log already had, or one that was
    /// never stored.
    pub settled: usize,
    /// Repositories whose head was ahead of the log: each got `#sync`.
    pub synced: usize,
}

/// Settles the commits that were in flight when the server last stopped
/// (SPEC §7.4).
///
/// For each note in `ops.sqlite`: when the repository's head is the noted
/// revision and the log has no event of that revision, the server appends
/// `#sync` with the head. The note is removed in every case.
///
/// This call blocks. Run it before the server accepts requests.
pub fn repair(state: &AppState) -> Result<Repaired, XrpcError> {
    let Some(writes) = state.writes() else {
        return Ok(Repaired::default());
    };
    let mut repaired = Repaired::default();
    let pending = writes.with_ops(pds_storage::OpsDb::pending_commits)?;
    for note in pending {
        let opened = ActorDb::open(&state.layout().actor_db(&note.did))
            .and_then(|db| db.root().map(|head| (db, head)));
        let (db, head) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                tracing::error!(did = %note.did, %error, "repair: the actor store is unreadable");
                continue;
            }
        };
        let stored = head.as_ref().is_some_and(|head| head.rev == note.rev);
        let sequenced = writes
            .sequencer()
            .reader()?
            .latest_of(&note.did, &["append", "sync"])?
            .and_then(|row| pds_sequencer::event_rev(&row.event))
            .is_some_and(|rev| rev >= note.rev);
        if let (true, false, Some(head)) = (stored, sequenced, head) {
            let block = db
                .block(&head.cid)?
                .ok_or_else(|| XrpcError::internal("the commit block of the head is missing"))?;
            let now = writes.sequencer().now();
            // The revision of the commit block: after an import the store
            // names a newer one.
            let rev = pds_repo::Commit::decode(&block).map_or_else(
                |_| head.rev.clone(),
                |commit| commit.rev.as_str().to_owned(),
            );
            let event = sync_event(&note.did, &rev, &head.cid, &block, &now);
            writes.sequencer().sequence(std::slice::from_ref(&event))?;
            if let Err(error) = writes.with_accounts(|accounts| {
                accounts.update_repo_root(&note.did, &head.cid.to_string(), &head.rev, &now)
            }) {
                tracing::error!(did = %note.did, %error, "failed to update account root");
            }
            tracing::warn!(did = %note.did, rev = %head.rev, "repair: the head was ahead of the log; sent #sync");
            repaired.synced += 1;
        } else {
            repaired.settled += 1;
        }
        writes.with_ops(|ops| ops.clear_commit(&note.did, &note.rev))?;
    }
    Ok(repaired)
}

/// Appends `#sync` with the head of the repository of `did`, so that a
/// consumer takes the state of the server (SPEC §7.4, §22.1). Returns the
/// sequence number, or `None` when the account has no repository.
///
/// This call blocks.
pub fn emit_sync(state: &AppState, did: &str) -> Result<Option<i64>, XrpcError> {
    let writes = state
        .writes()
        .ok_or_else(|| XrpcError::internal("a read-only server appends no event"))?;
    let lock = writes.repo_lock(did);
    let _held = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = state.layout().actor_db(did);
    if !path.exists() {
        return Ok(None);
    }
    let db = ActorDb::open(&path)?;
    let Some(head) = db.root()? else {
        return Ok(None);
    };
    let block = db
        .block(&head.cid)?
        .ok_or_else(|| XrpcError::internal("the commit block of the head is missing"))?;
    let rev = pds_repo::Commit::decode(&block).map_or_else(
        |_| head.rev.clone(),
        |commit| commit.rev.as_str().to_owned(),
    );
    let now = writes.sequencer().now();
    let event = sync_event(did, &rev, &head.cid, &block, &now);
    let seqs = writes.sequencer().sequence(std::slice::from_ref(&event))?;
    Ok(seqs.first().copied())
}

/// How long an upload waits for a record before the server collects it:
/// one hour (SPEC §6.4). Collecting such uploads is a deliberate
/// difference (SPEC §2.3).
pub const UPLOAD_GRACE_MS: i64 = 60 * 60 * 1000;

/// Removes the uploads that arrived more than [`UPLOAD_GRACE_MS`] ago and
/// that no record references: the index row and the staged file. Returns
/// how many it removed.
///
/// This call blocks, and it opens every actor store.
pub fn collect_uploads(state: &AppState) -> Result<usize, XrpcError> {
    let Some(blobs) = state.blobs() else {
        return Ok(0);
    };
    let before = pds_sequencer::iso_millis(state.now_ms().saturating_sub(UPLOAD_GRACE_MS));
    let actors = state.layout().accounts()?.all_actors()?;
    let mut collected = 0;
    for actor in actors {
        let path = state.layout().actor_db(&actor.did);
        // A read first: most stores have nothing to collect, and a read
        // takes no write lock.
        let waiting = match ActorDb::open(&path).and_then(|db| db.pending_uploads(&before)) {
            Ok(waiting) => waiting,
            Err(error) => {
                tracing::warn!(did = %actor.did, %error, "uploads: the actor store is unreadable");
                continue;
            }
        };
        if waiting.is_empty() {
            continue;
        }
        // Read again inside the transaction: a record may have referenced
        // an upload since.
        let gone = transact(state, &actor.did, |session| {
            let waiting = session.txn().pending_uploads(&before)?;
            let cids: Vec<String> = waiting.iter().map(|upload| upload.cid.clone()).collect();
            session.txn().delete_blobs(&cids)?;
            Ok(waiting)
        })?;
        for upload in &gone {
            // The file may already be in place, when a commit stopped
            // after it moved the file and before it stored the reference.
            if let Err(error) = blobs.delete(&actor.did, &upload.cid) {
                tracing::error!(did = %actor.did, cid = %upload.cid, %error, "could not delete an unreferenced blob");
            }
            if let Err(error) = blobs.delete_staged(&actor.did, &upload.temp_key) {
                tracing::error!(did = %actor.did, cid = %upload.cid, %error, "could not delete a staged upload");
            }
        }
        collected += gone.len();
    }
    Ok(collected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(json: &str) -> Data {
        Data::from_json_str(json).unwrap()
    }

    #[test]
    fn the_indexed_links_are_those_of_follows_blocks_likes_and_reposts() {
        let follow = data(r#"{"$type":"app.bsky.graph.follow","subject":"did:plc:abc"}"#);
        assert_eq!(
            backlinks("at://did:plc:x/app.bsky.graph.follow/1", &follow),
            vec![Backlink {
                uri: "at://did:plc:x/app.bsky.graph.follow/1".to_owned(),
                path: "subject",
                link_to: "did:plc:abc".to_owned(),
            }]
        );
        let like = data(
            r#"{"$type":"app.bsky.feed.like","subject":{"uri":"at://did:plc:abc/app.bsky.feed.post/3aaaaaaaaaaa2","cid":"x"}}"#,
        );
        assert_eq!(backlinks("u", &like)[0].path, "subject.uri");
        let not_a_did = data(r#"{"$type":"app.bsky.graph.follow","subject":"nobody"}"#);
        assert_eq!(backlinks("u", &not_a_did), []);
        let post = data(r#"{"$type":"app.bsky.feed.post","subject":"did:plc:abc"}"#);
        assert_eq!(backlinks("u", &post), []);
    }

    #[test]
    fn blobs_are_found_at_any_depth_and_the_legacy_form_is_refused() {
        let record = data(
            r#"{"a":[{"file":{"$type":"blob","ref":{"$link":"bafkreib5njcqaneu6gbwsmfbqeswb654woqznfim3pjbn7klh3alxjsrhm"},"mimeType":"text/plain","size":11}}]}"#,
        );
        let mut found = Vec::new();
        blob_uses(&record, &mut found).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].size, 11);

        let legacy = data(
            r#"{"file":{"cid":"bafkreib5njcqaneu6gbwsmfbqeswb654woqznfim3pjbn7klh3alxjsrhm","mimeType":"text/plain"}}"#,
        );
        let error = blob_uses(&legacy, &mut Vec::new()).unwrap_err();
        assert!(error.message.starts_with("Legacy blobs are not allowed"));
    }

    #[test]
    fn a_swap_is_checked_as_the_reference_checks_it() {
        let cid: Cid = "bafkreib5njcqaneu6gbwsmfbqeswb654woqznfim3pjbn7klh3alxjsrhm"
            .parse()
            .unwrap();
        let delete = |swap| Prepared::Delete {
            uri: "u".to_owned(),
            collection: "c".to_owned(),
            rkey: "r".to_owned(),
            swap,
        };
        assert!(Session::swap_check(&delete(Swap::Unset), None).is_ok());
        assert!(Session::swap_check(&delete(Swap::Cid(cid)), Some(&cid)).is_ok());
        let stale = Session::swap_check(&delete(Swap::Cid(cid)), None).unwrap_err();
        assert_eq!(stale.error, "InvalidSwap");
        assert_eq!(stale.message, "Record was at null");
        assert!(Session::swap_check(&delete(Swap::Absent), Some(&cid)).is_err());
    }
}
