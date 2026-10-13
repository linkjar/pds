//! What a simulation expects: the repositories as the acknowledged writes
//! left them, and a relay that follows the firehose strictly.

use std::collections::BTreeMap;

use cid::Cid;
use pds_repo::{Commit, read_car};
use pds_types::cbor::Reader;
use pds_types::{Data, Limits};

/// One record as the model knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The CID of the record.
    pub cid: String,
    /// The blob the record references, when it references one.
    pub blob: Option<String>,
}

/// The records every repository must hold, by `<collection>/<rkey>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expected {
    /// The records of each account.
    pub repos: BTreeMap<String, BTreeMap<String, Entry>>,
    /// Every blob that was uploaded, by account.
    pub uploads: BTreeMap<String, Vec<String>>,
}

impl Expected {
    /// Whether a record of `did` references the blob.
    pub fn references(&self, did: &str, blob: &str) -> bool {
        self.repos.get(did).is_some_and(|records| {
            records
                .values()
                .any(|entry| entry.blob.as_deref() == Some(blob))
        })
    }
}

/// One decoded frame of the firehose.
#[derive(Debug)]
pub struct Event {
    /// `#commit`, `#sync`, `#identity`, `#account` or `#info`.
    pub kind: String,
    /// The body.
    pub body: Data,
}

impl Event {
    /// Decodes a frame.
    pub fn decode(frame: &[u8]) -> Result<Self, String> {
        let mut reader = Reader::new(frame, Limits::FRAME).map_err(|e| e.to_string())?;
        let head = reader.data(0).map_err(|e| e.to_string())?;
        let body = reader.data(0).map_err(|e| e.to_string())?;
        if !reader.remaining().is_empty() {
            return Err("bytes after the body of a frame".to_owned());
        }
        match (head.get("op"), head.get("t")) {
            (Some(Data::Integer(1)), Some(Data::String(kind))) => Ok(Self {
                kind: kind.clone(),
                body,
            }),
            _ => Err(format!("unexpected frame header {head:?}")),
        }
    }

    /// The sequence number.
    pub fn seq(&self) -> Result<i64, String> {
        match self.body.get("seq") {
            Some(Data::Integer(seq)) => Ok(*seq),
            other => Err(format!("seq is {other:?}")),
        }
    }

    fn text(&self, key: &str) -> Result<&str, String> {
        match self.body.get(key) {
            Some(Data::String(text)) => Ok(text),
            other => Err(format!("{key} is {other:?}")),
        }
    }

    fn link(&self, key: &str) -> Option<Cid> {
        match self.body.get(key) {
            Some(Data::Link(link)) => Some(link.0),
            _ => None,
        }
    }
}

/// A repository as the relay model holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayRepo {
    /// The revision of the last commit the relay accepted. Empty before
    /// the first.
    pub rev: String,
    /// The tree root of that commit.
    pub data: Option<Cid>,
    /// The records, by `<collection>/<rkey>`.
    pub records: BTreeMap<String, Cid>,
    /// Whether the relay dropped a commit and now waits for `#sync`.
    pub desynced: bool,
}

/// What the relay model needs after an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Nothing.
    Continue,
    /// The repository of this account must be fetched again: the event was
    /// `#sync`.
    Resync(String),
}

/// A relay that checks every commit as Sync 1.1 requires.
///
/// It checks the signature, the link to the commit before, and each
/// operation against the record it replaces. A commit that does not follow
/// the one before is dropped, and the repository waits for `#sync`.
#[derive(Debug, Default)]
pub struct Relay {
    /// The repositories, by account.
    pub repos: BTreeMap<String, RelayRepo>,
    /// How many commits the relay dropped.
    pub dropped: u32,
}

impl Relay {
    /// Takes one event. `key_of` gives the public key of an account.
    pub fn apply(
        &mut self,
        event: &Event,
        key_of: impl Fn(&str) -> Option<pds_identity::PublicKey>,
    ) -> Result<Next, String> {
        match event.kind.as_str() {
            "#commit" => {}
            "#sync" => return Ok(Next::Resync(event.text("did")?.to_owned())),
            _ => return Ok(Next::Continue),
        }
        let did = event.text("repo")?.to_owned();
        let rev = event.text("rev")?.to_owned();
        let repo = self.repos.entry(did.clone()).or_default();
        // A resync fetched a head at or after this commit.
        if !repo.rev.is_empty() && rev <= repo.rev {
            return Ok(Next::Continue);
        }
        let Some(Data::Bytes(blocks)) = event.body.get("blocks") else {
            return Err("a commit without blocks".to_owned());
        };
        let car = read_car(&blocks.0).map_err(|e| e.to_string())?;
        let root = car.root().map_err(|e| e.to_string())?;
        if Some(root) != event.link("commit") {
            return Err("the root of the slice is not the commit".to_owned());
        }
        let commit = Commit::decode(car.blocks.get(&root).ok_or("no commit block")?)
            .map_err(|e| e.to_string())?;
        let key = key_of(&did).ok_or("an event for an unknown account")?;
        commit
            .verify(|bytes, signature| key.verify(bytes, signature).is_ok())
            .map_err(|e| format!("commit {rev} of {did}: {e}"))?;
        if commit.rev.as_str() != rev || commit.did.as_str() != did {
            return Err("the commit block disagrees with its event".to_owned());
        }

        let since = match event.body.get("since") {
            Some(Data::String(since)) => since.clone(),
            _ => String::new(),
        };
        let follows = since == repo.rev && event.link("prevData") == repo.data;
        if repo.desynced || !follows {
            repo.desynced = true;
            self.dropped += 1;
            return Ok(Next::Continue);
        }
        let Some(Data::Array(ops)) = event.body.get("ops") else {
            return Err("a commit without ops".to_owned());
        };
        let mut records = repo.records.clone();
        for op in ops {
            let (Some(Data::String(action)), Some(Data::String(path))) =
                (op.get("action"), op.get("path"))
            else {
                return Err("an operation without action or path".to_owned());
            };
            let cid = match op.get("cid") {
                Some(Data::Link(link)) => Some(link.0),
                _ => None,
            };
            let prev = match op.get("prev") {
                Some(Data::Link(link)) => Some(link.0),
                _ => None,
            };
            if prev != records.get(path).copied() {
                return Err(format!(
                    "commit {rev} of {did}: {action} {path} names another previous record"
                ));
            }
            match (action.as_str(), cid) {
                ("create" | "update", Some(cid)) => {
                    if !car.blocks.contains(&cid) {
                        return Err(format!("commit {rev}: the record of {path} is not sent"));
                    }
                    records.insert(path.clone(), cid);
                }
                ("delete", None) => {
                    records.remove(path);
                }
                _ => return Err(format!("commit {rev}: malformed operation on {path}")),
            }
        }
        repo.records = records;
        repo.rev = rev;
        repo.data = Some(commit.data);
        Ok(Next::Continue)
    }

    /// Replaces a repository by what the relay fetched from the server.
    pub fn resync(&mut self, did: &str, rev: String, data: Cid, records: BTreeMap<String, Cid>) {
        self.repos.insert(
            did.to_owned(),
            RelayRepo {
                rev,
                data: Some(data),
                records,
                desynced: false,
            },
        );
    }
}
