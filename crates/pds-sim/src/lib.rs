//! The simulation harness (`docs/SPEC.md` §19).
//!
//! A run is a function of its seed. The seed draws a plan of operations
//! against one server over one data directory: record writes on several
//! accounts, batches, uploads, subscribers that connect with a cursor and
//! leave, restarts, and stops at the fail points of the commit path. The
//! clock, the revision clock and the server's random source are the
//! simulation's, so two runs of a seed produce the same trace, down to the
//! commit CIDs.
//!
//! After every stop and at intervals the run checks its oracles (§19.4):
//!
//! - `verify` finds no difference in the data directory.
//! - No acknowledged write is lost, and no write that was refused or
//!   interrupted before its commit is present: every repository holds
//!   exactly the records the model holds.
//! - A subscriber that resumes from its cursor receives every event once,
//!   in order.
//! - A relay model that checks every commit strictly, and fetches a
//!   repository again only on `#sync`, ends with the server's repositories.
//! - A blob is served when a record references it, and only then.
//!
//! # What this simulator does not model
//!
//! - **Storage faults below SQLite.** A stop is a process stop between two
//!   steps of a commit: the files hold what SQLite committed. Lost or torn
//!   writes, failed syncs, bit flips and a full disk need a fault-injecting
//!   VFS, which this crate does not have yet.
//! - **The network.** Requests enter the router in process, and subscribers
//!   read the log through the subscription type. No socket, no partition
//!   and no slow consumer is simulated.
//! - **Concurrency.** Operations run one after the other. Two writers on
//!   one repository are not interleaved here; the repository lock is.
//! - **Two operations on one key in one batch**, which the Reference
//!   reports with the record that was there before the batch.
//! - Accounts, imports, OAuth and mail, which other units bring.

pub mod model;
pub mod rng;
pub mod world;

use std::collections::BTreeMap;
use std::sync::Arc;

use cid::Cid;
use pds_repo::Commit;
use pds_sequencer::Subscription;
use pds_storage::ActorDb;
use pds_storage::fail::{self, Action};
use pds_types::Data;
use pds_types::content::cid_for_cbor;
use pds_xrpc::write::Repaired;
use serde_json::{Value, json};

use model::{Entry, Event, Expected, Next, Relay};
use rng::Rng;
use world::{Answer, World};

/// A run did not finish, or an oracle failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    /// The world could not be built or driven.
    #[error("setup: {0}")]
    Setup(String),
    /// An oracle failed.
    #[error("step {step}: {what}")]
    Oracle {
        /// The step of the plan.
        step: usize,
        /// What was wrong.
        what: String,
    },
}

/// The size of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// How many accounts the data directory has.
    pub accounts: usize,
    /// How many operations the plan has.
    pub steps: usize,
    /// How many writes in a hundred stop at a fail point.
    pub fault_percent: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            accounts: 3,
            steps: 150,
            fault_percent: 8,
        }
    }
}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    /// One line for each operation and each check, the same for every run
    /// of the seed.
    pub trace: Vec<String>,
    /// Commits that the server acknowledged.
    pub commits: u32,
    /// Stops at a fail point.
    pub crashes: u32,
    /// Starts of the server, the first one included.
    pub starts: u32,
    /// Events in the log at the end.
    pub events: i64,
    /// Repositories the relay model fetched again after `#sync`.
    pub relay_resyncs: u32,
    /// Commits the relay model dropped because they did not follow the
    /// commit before.
    pub relay_dropped: u32,
}

const COLLECTIONS: [&str; 2] = ["com.example.sim.note", "com.example.sim.card"];
const KEYS: u64 = 12;
const POINTS: [&str; 3] = ["commit.before-store", "commit.stored", "commit.sequenced"];
/// A check runs at least this often, in steps.
const CHECK_EVERY: usize = 40;
const SUBSCRIBERS: usize = 3;

/// What the model expects of a write before it is sent.
enum Prediction {
    /// The server refuses with this status.
    Refused(u16),
    /// 200, and no commit: the write changes nothing.
    NoCommit,
    /// 200 and a commit with these changes: a key and its record, or `None`
    /// for a delete.
    Commit(Vec<(String, Option<Entry>)>),
}

/// A subscriber of the model: it connects, reads for a while, and leaves.
#[derive(Debug, Default)]
struct Subscriber {
    /// The sequence number after which it must see every event. `None`
    /// before its first connection.
    start: Option<i64>,
    cursor: i64,
    seen: Vec<i64>,
}

struct Sim {
    world: World,
    rng: Rng,
    options: Options,
    expected: Expected,
    relay: Relay,
    relay_cursor: i64,
    subscribers: Vec<Subscriber>,
    outcome: Outcome,
    step: usize,
    counter: u64,
}

fn record_cid(record: &Value) -> Result<String, String> {
    let data = Data::from_json_str(&record.to_string()).map_err(|e| e.to_string())?;
    Ok(cid_for_cbor(&data.to_dag_cbor()).to_string())
}

impl Sim {
    fn fail(&self, what: impl Into<String>) -> Failure {
        Failure::Oracle {
            step: self.step,
            what: what.into(),
        }
    }

    fn trace(&mut self, line: &str) {
        self.outcome.trace.push(format!("{:>4} {line}", self.step));
    }

    fn account(&mut self) -> String {
        self.world.dids[self.rng.index(self.world.dids.len())].clone()
    }

    fn key(&mut self) -> (String, String) {
        let collection = *self.rng.pick(&COLLECTIONS);
        (collection.to_owned(), format!("k{}", self.rng.below(KEYS)))
    }

    fn record(&mut self, collection: &str, blob: Option<&Value>) -> Value {
        self.counter += 1;
        let pad = "x".repeat(self.rng.index(200));
        let mut record = json!({"$type": collection, "n": self.counter, "pad": pad});
        if let Some(blob) = blob {
            record["file"] = blob.clone();
        }
        record
    }

    fn current(&self, did: &str, path: &str) -> Option<&Entry> {
        self.expected.repos.get(did)?.get(path)
    }

    /// The records of `did` as its store indexes them.
    async fn stored(&self, did: &str) -> Result<(String, Cid, BTreeMap<String, Cid>), Failure> {
        let path = self.world.config.layout.actor_db(did);
        let prefix = format!("at://{did}/");
        let read = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let store = ActorDb::open(&path).map_err(|e| e.to_string())?;
            let root = store
                .root()
                .map_err(|e| e.to_string())?
                .ok_or("no repository root")?;
            let block = store
                .block(&root.cid)
                .map_err(|e| e.to_string())?
                .ok_or("no commit block")?;
            let commit = Commit::decode(&block).map_err(|e| e.to_string())?;
            let mut records = BTreeMap::new();
            for (uri, cid) in store.all_records().map_err(|e| e.to_string())? {
                let key = uri.strip_prefix(&prefix).unwrap_or(&uri).to_owned();
                records.insert(key, cid.parse::<Cid>().map_err(|e| e.to_string())?);
            }
            Ok((root.rev, commit.data, records))
        })
        .await
        .map_err(|e| self.fail(e.to_string()))?;
        read.map_err(|what| self.fail(format!("{did}: {what}")))
    }

    /// Sends a write and holds the answer and the data directory to the
    /// prediction. With a fault the server stops inside the commit and is
    /// started again.
    async fn write(
        &mut self,
        what: &str,
        nsid: &str,
        did: &str,
        body: &Value,
        prediction: Prediction,
    ) -> Result<(), Failure> {
        let commits = matches!(prediction, Prediction::Commit(_));
        let point = (commits && self.rng.chance(self.options.fault_percent))
            .then(|| *self.rng.pick(&POINTS));
        if let Some(point) = point {
            fail::arm(point, Action::Error);
        }
        let answer = self.world.procedure(nsid, did, body).await;
        fail::disarm_all();
        let answer = answer?;

        if let Some(point) = point {
            if answer.status != 500 {
                return Err(self.fail(format!(
                    "{what}: stopped at {point} and answered {}",
                    answer.status
                )));
            }
            let applied = point != "commit.before-store";
            self.trace(&format!("{what} -> stopped at {point}"));
            if applied && let Prediction::Commit(changes) = prediction {
                self.apply(did, changes);
            }
            self.outcome.crashes += 1;
            let repaired = self.world.restart().await?;
            let want = if point == "commit.stored" {
                Repaired {
                    settled: 0,
                    synced: 1,
                }
            } else {
                Repaired {
                    settled: 1,
                    synced: 0,
                }
            };
            if repaired != want {
                return Err(self.fail(format!(
                    "after a stop at {point} the repair did {repaired:?}"
                )));
            }
            self.trace(&format!("restart -> {repaired:?}"));
            return self.check().await;
        }

        match prediction {
            Prediction::Refused(status) => {
                if answer.status != status {
                    return Err(self.fail(format!(
                        "{what}: expected {status}, got {} {}",
                        answer.status,
                        String::from_utf8_lossy(&answer.body)
                    )));
                }
                self.trace(&format!("{what} -> {status}"));
            }
            Prediction::NoCommit => {
                let json = self.accepted(what, &answer)?;
                if json.get("commit").is_some() {
                    return Err(
                        self.fail(format!("{what}: a commit for a write that changes nothing"))
                    );
                }
                self.trace(&format!("{what} -> 200, no commit"));
            }
            Prediction::Commit(changes) => {
                let json = self.accepted(what, &answer)?;
                let rev = json["commit"]["rev"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if rev.is_empty() {
                    return Err(self.fail(format!("{what}: no commit in the answer")));
                }
                // A single write names the record's CID.
                if let (Some(cid), [(_, Some(entry))]) = (json["cid"].as_str(), changes.as_slice())
                    && cid != entry.cid
                {
                    return Err(self.fail(format!(
                        "{what}: the answer names {cid}, the record is {}",
                        entry.cid
                    )));
                }
                self.apply(did, changes);
                self.outcome.commits += 1;
                self.trace(&format!("{what} -> 200 {rev}"));
            }
        }
        Ok(())
    }

    fn accepted(&self, what: &str, answer: &Answer) -> Result<Value, Failure> {
        if World::ok(answer) {
            Ok(answer.json())
        } else {
            Err(self.fail(format!(
                "{what}: expected 200, got {} {}",
                answer.status,
                String::from_utf8_lossy(&answer.body)
            )))
        }
    }

    fn apply(&mut self, did: &str, changes: Vec<(String, Option<Entry>)>) {
        let records = self.expected.repos.entry(did.to_owned()).or_default();
        for (path, entry) in changes {
            match entry {
                Some(entry) => {
                    records.insert(path, entry);
                }
                None => {
                    records.remove(&path);
                }
            }
        }
    }

    fn entry(&self, record: &Value) -> Result<Entry, Failure> {
        Ok(Entry {
            cid: record_cid(record).map_err(|what| self.fail(what))?,
            blob: record["file"]["ref"]["$link"].as_str().map(str::to_owned),
        })
    }

    async fn create(&mut self) -> Result<(), Failure> {
        let did = self.account();
        let (collection, rkey) = self.key();
        let path = format!("{collection}/{rkey}");
        let record = self.record(&collection, None);
        let prediction = if self.current(&did, &path).is_some() {
            Prediction::Refused(400)
        } else {
            Prediction::Commit(vec![(path.clone(), Some(self.entry(&record)?))])
        };
        let body = json!({"repo": did, "collection": collection, "rkey": rkey, "record": record});
        self.write(
            &format!("create {did} {path}"),
            "com.atproto.repo.createRecord",
            &did,
            &body,
            prediction,
        )
        .await
    }

    async fn put(&mut self, blob: Option<&Value>, did: Option<String>) -> Result<(), Failure> {
        let did = did.unwrap_or_else(|| self.account());
        let (collection, rkey) = self.key();
        let path = format!("{collection}/{rkey}");
        // One time in five the content is the one already there.
        let exists = self.current(&did, &path).is_some();
        let record = if exists && blob.is_none() && self.rng.chance(20) {
            None
        } else {
            Some(self.record(&collection, blob))
        };
        let (record, prediction) = if let Some(record) = record {
            let entry = self.entry(&record)?;
            (
                record,
                Prediction::Commit(vec![(path.clone(), Some(entry))]),
            )
        } else {
            // Read the record back and write it again.
            let answer = self
                .world
                .get(&format!(
                    "/xrpc/com.atproto.repo.getRecord?repo={did}&collection={collection}&rkey={rkey}"
                ))
                .await?;
            let value = self.accepted("getRecord", &answer)?["value"].clone();
            (value, Prediction::NoCommit)
        };
        let body = json!({"repo": did, "collection": collection, "rkey": rkey, "record": record});
        self.write(
            &format!("put {did} {path}"),
            "com.atproto.repo.putRecord",
            &did,
            &body,
            prediction,
        )
        .await
    }

    async fn delete(&mut self) -> Result<(), Failure> {
        let did = self.account();
        let (collection, rkey) = self.key();
        let path = format!("{collection}/{rkey}");
        let prediction = if self.current(&did, &path).is_some() {
            Prediction::Commit(vec![(path.clone(), None)])
        } else {
            Prediction::NoCommit
        };
        let body = json!({"repo": did, "collection": collection, "rkey": rkey});
        self.write(
            &format!("delete {did} {path}"),
            "com.atproto.repo.deleteRecord",
            &did,
            &body,
            prediction,
        )
        .await
    }

    async fn batch(&mut self) -> Result<(), Failure> {
        let did = self.account();
        let count = 1 + self.rng.index(6);
        let mut writes = Vec::new();
        let mut changes: Vec<(String, Option<Entry>)> = Vec::new();
        let mut refusal = None;
        for _ in 0..count {
            let (collection, rkey) = self.key();
            let path = format!("{collection}/{rkey}");
            // One operation for a key in a batch (see the module comment).
            if changes.iter().any(|(seen, _)| *seen == path) {
                continue;
            }
            let exists = self.current(&did, &path).is_some();
            let kind = self.rng.below(3);
            // Mostly operations that fit the repository, so that most
            // batches commit.
            let kind = if self.rng.chance(85) {
                match (kind, exists) {
                    (0, true) => 1,
                    (1 | 2, false) => 0,
                    (kind, _) => kind,
                }
            } else {
                kind
            };
            match kind {
                0 => {
                    let record = self.record(&collection, None);
                    if exists && refusal.is_none() {
                        refusal = Some(400);
                    }
                    changes.push((path, Some(self.entry(&record)?)));
                    writes.push(json!({"$type": "com.atproto.repo.applyWrites#create", "collection": collection, "rkey": rkey, "value": record}));
                }
                1 => {
                    let record = self.record(&collection, None);
                    if !exists && refusal.is_none() {
                        refusal = Some(500);
                    }
                    changes.push((path, Some(self.entry(&record)?)));
                    writes.push(json!({"$type": "com.atproto.repo.applyWrites#update", "collection": collection, "rkey": rkey, "value": record}));
                }
                _ => {
                    if !exists && refusal.is_none() {
                        refusal = Some(500);
                    }
                    changes.push((path, None));
                    writes.push(json!({"$type": "com.atproto.repo.applyWrites#delete", "collection": collection, "rkey": rkey}));
                }
            }
        }
        let what = format!("batch {did} of {}", writes.len());
        let prediction = refusal.map_or(Prediction::Commit(changes), Prediction::Refused);
        let body = json!({"repo": did, "writes": writes});
        self.write(
            &what,
            "com.atproto.repo.applyWrites",
            &did,
            &body,
            prediction,
        )
        .await
    }

    async fn upload(&mut self) -> Result<(), Failure> {
        let did = self.account();
        let len = 1 + self.rng.index(1500);
        // The first byte is a letter, so that the content names no type.
        let mut bytes = vec![b'b'; len];
        for byte in bytes.iter_mut().skip(1) {
            *byte = u8::try_from(self.rng.below(256)).unwrap_or(0);
        }
        let answer = self.world.upload(&did, bytes).await?;
        let blob = self.accepted("upload", &answer)?["blob"].clone();
        let cid = blob["ref"]["$link"].as_str().unwrap_or_default().to_owned();
        if cid.is_empty() {
            return Err(self.fail("upload: no blob in the answer"));
        }
        self.trace(&format!("upload {did} -> {cid}"));
        let uploads = self.expected.uploads.entry(did.clone()).or_default();
        if !uploads.contains(&cid) {
            uploads.push(cid);
        }
        if self.rng.chance(70) {
            return self.put(Some(&blob), Some(did)).await;
        }
        Ok(())
    }

    /// The relay model reads the log from its cursor to the head.
    async fn relay_read(&mut self) -> Result<(), Failure> {
        let sequencer = Arc::clone(self.world.sequencer()?);
        if sequencer.head() <= self.relay_cursor {
            return Ok(());
        }
        let mut subscription = Subscription::open(
            Arc::clone(&sequencer),
            Some(self.relay_cursor),
            86_400_000,
            500,
        )
        .await
        .map_err(|e| self.fail(e.to_string()))?
        .map_err(|refusal| self.fail(format!("the relay was refused: {refusal:?}")))?;
        while sequencer.head() > subscription.cursor() {
            let frames = subscription
                .next()
                .await
                .map_err(|e| self.fail(e.to_string()))?
                .map_err(|refusal| self.fail(format!("the relay was refused: {refusal:?}")))?;
            for frame in frames {
                let event = Event::decode(&frame).map_err(|what| self.fail(what))?;
                let seq = event.seq().map_err(|what| self.fail(what))?;
                if seq <= self.relay_cursor {
                    return Err(self.fail(format!("event {seq} after {}", self.relay_cursor)));
                }
                self.relay_cursor = seq;
                let dids = self.world.dids.clone();
                let key_of = |did: &str| {
                    dids.iter()
                        .position(|known| known == did)
                        .map(|n| world::key(n).public_key())
                };
                match self
                    .relay
                    .apply(&event, key_of)
                    .map_err(|what| self.fail(format!("relay: {what}")))?
                {
                    Next::Continue => {}
                    Next::Resync(did) => {
                        let (rev, data, records) = self.stored(&did).await?;
                        // The four events of an account's creation end
                        // with `#sync`; that one is not a repair.
                        if self
                            .relay
                            .repos
                            .get(&did)
                            .is_some_and(|repo| repo.rev != rev)
                        {
                            self.outcome.relay_resyncs += 1;
                        }
                        self.relay.resync(&did, rev, data, records);
                    }
                }
            }
        }
        Ok(())
    }

    /// One subscriber connects, reads some of what it has not seen, and
    /// leaves. With `drain` it reads to the head.
    async fn subscriber_read(&mut self, index: usize, drain: bool) -> Result<(), Failure> {
        let sequencer = Arc::clone(self.world.sequencer()?);
        let cursor = match self.subscribers[index].start {
            Some(_) => Some(self.subscribers[index].cursor),
            // The first connection: from the start of the log, or live only.
            None if self.rng.chance(50) => Some(0),
            None => None,
        };
        let mut subscription = Subscription::open(Arc::clone(&sequencer), cursor, 86_400_000, 500)
            .await
            .map_err(|e| self.fail(e.to_string()))?
            .map_err(|refusal| self.fail(format!("a subscriber was refused: {refusal:?}")))?;
        if self.subscribers[index].start.is_none() {
            self.subscribers[index].start = Some(subscription.cursor());
            self.subscribers[index].cursor = subscription.cursor();
        }
        while sequencer.head() > subscription.cursor() {
            let frames = subscription
                .next()
                .await
                .map_err(|e| self.fail(e.to_string()))?
                .map_err(|refusal| self.fail(format!("a subscriber was refused: {refusal:?}")))?;
            for frame in frames {
                let seq = Event::decode(&frame)
                    .and_then(|event| event.seq())
                    .map_err(|what| self.fail(what))?;
                let before = self.subscribers[index].cursor;
                if seq <= before {
                    return Err(
                        self.fail(format!("subscriber {index}: event {seq} after {before}"))
                    );
                }
                self.subscribers[index].cursor = seq;
                self.subscribers[index].seen.push(seq);
            }
            if !drain && self.rng.chance(40) {
                break;
            }
        }
        Ok(())
    }

    /// The oracles of SPEC §19.4.
    async fn check(&mut self) -> Result<(), Failure> {
        // 1. The data directory verifies.
        let config = self.world.config.clone();
        let report = tokio::task::spawn_blocking(move || linkjar_pds::verify::verify(&config))
            .await
            .map_err(|e| self.fail(e.to_string()))?
            .map_err(|e| self.fail(format!("verify: {e}")))?;
        if let Some(finding) = report.findings.first() {
            return Err(self.fail(format!(
                "verify: {} {}: {} ({} findings)",
                finding.subject,
                finding.kind,
                finding.detail,
                report.findings.len()
            )));
        }

        // 2. Every repository holds the records of the model, and no other.
        self.relay_read().await?;
        for did in self.world.dids.clone() {
            let (rev, _, records) = self.stored(&did).await?;
            let want: BTreeMap<String, String> = self
                .expected
                .repos
                .get(&did)
                .map(|records| {
                    records
                        .iter()
                        .map(|(path, entry)| (path.clone(), entry.cid.clone()))
                        .collect()
                })
                .unwrap_or_default();
            let have: BTreeMap<String, String> = records
                .iter()
                .map(|(path, cid)| (path.clone(), cid.to_string()))
                .collect();
            if have != want {
                let odd = have
                    .iter()
                    .find(|(path, cid)| want.get(*path) != Some(*cid))
                    .map(|(path, _)| path.clone())
                    .or_else(|| want.keys().find(|path| !have.contains_key(*path)).cloned())
                    .unwrap_or_default();
                return Err(self.fail(format!(
                    "{did}: the store has {} records and the model {}; first at {odd}",
                    have.len(),
                    want.len()
                )));
            }
            // 3. The relay model converged on the same repository.
            let relay = self.relay.repos.get(&did).cloned().unwrap_or_default();
            if relay.desynced || relay.rev != rev || relay.records != records {
                return Err(self.fail(format!(
                    "{did}: the relay is at {} (desynced: {}) and the server at {rev}",
                    relay.rev, relay.desynced
                )));
            }
            // 4. A blob is served when a record references it, and only then.
            for blob in self.expected.uploads.get(&did).cloned().unwrap_or_default() {
                let answer = self
                    .world
                    .get(&format!(
                        "/xrpc/com.atproto.sync.getBlob?did={did}&cid={blob}"
                    ))
                    .await?;
                let referenced = self.expected.references(&did, &blob);
                if World::ok(&answer) != referenced {
                    return Err(self.fail(format!(
                        "{did}: blob {blob} is {} and getBlob answers {}",
                        if referenced {
                            "referenced"
                        } else {
                            "not referenced"
                        },
                        answer.status
                    )));
                }
            }
        }

        self.check_subscribers().await?;
        let head = self.world.sequencer()?.head();
        self.trace(&format!("check -> ok, {head} events"));
        Ok(())
    }

    /// Every subscriber saw each event after its start once, in order.
    async fn check_subscribers(&mut self) -> Result<(), Failure> {
        let sequencer = Arc::clone(self.world.sequencer()?);
        for index in 0..self.subscribers.len() {
            let Some(start) = self.subscribers[index].start else {
                continue;
            };
            self.subscriber_read(index, true).await?;
            let reader = Arc::clone(&sequencer);
            let logged = tokio::task::spawn_blocking(move || -> Result<Vec<i64>, String> {
                let rows = reader
                    .reader()
                    .and_then(|db| db.range(start, 1_000_000))
                    .map_err(|e| e.to_string())?;
                Ok(rows.into_iter().map(|row| row.seq).collect())
            })
            .await
            .map_err(|e| self.fail(e.to_string()))?
            .map_err(|what| self.fail(what))?;
            if self.subscribers[index].seen != logged {
                return Err(self.fail(format!(
                    "subscriber {index} saw {} events after {start} and the log has {}",
                    self.subscribers[index].seen.len(),
                    logged.len()
                )));
            }
        }
        Ok(())
    }

    async fn run(&mut self) -> Result<(), Failure> {
        for step in 1..=self.options.steps {
            self.step = step;
            let pause = 1 + self.rng.below(5000);
            self.world.clock.advance(i64::try_from(pause).unwrap_or(1));
            match self.rng.below(100) {
                0..=31 => self.create().await?,
                32..=46 => self.put(None, None).await?,
                47..=58 => self.delete().await?,
                59..=71 => self.batch().await?,
                72..=79 => self.upload().await?,
                80..=90 => {
                    let index = self.rng.index(SUBSCRIBERS);
                    self.subscriber_read(index, false).await?;
                    let seen = self.subscribers[index].seen.len();
                    self.trace(&format!("subscriber {index} -> {seen} events seen"));
                }
                91..=95 => {
                    self.relay_read().await?;
                    let cursor = self.relay_cursor;
                    self.trace(&format!("relay -> at {cursor}"));
                }
                _ => {
                    let repaired = self.world.restart().await?;
                    if repaired != Repaired::default() {
                        return Err(
                            self.fail(format!("a restart without a stop repaired {repaired:?}"))
                        );
                    }
                    self.trace("restart");
                    self.check().await?;
                }
            }
            if step % CHECK_EVERY == 0 {
                self.check().await?;
            }
        }
        self.step = self.options.steps + 1;
        self.check().await
    }
}

/// Fail points are one set for the process, so one run holds them at a
/// time.
static ONE_RUN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Runs the plan of `seed` and checks the oracles.
pub async fn run_seed(seed: u64, options: Options) -> Result<Outcome, Failure> {
    let _one_run = ONE_RUN.lock().await;
    fail::disarm_all();
    let world = World::create(seed, options.accounts).await?;
    let mut sim = Sim {
        world,
        rng: Rng::new(seed),
        options,
        expected: Expected::default(),
        relay: Relay::default(),
        relay_cursor: 0,
        subscribers: (0..SUBSCRIBERS).map(|_| Subscriber::default()).collect(),
        outcome: Outcome::default(),
        step: 0,
        counter: 0,
    };
    let result = sim.run().await;
    fail::disarm_all();
    result?;
    let mut outcome = std::mem::take(&mut sim.outcome);
    outcome.starts = sim.world.starts;
    outcome.events = sim.world.sequencer()?.head();
    outcome.relay_dropped = sim.relay.dropped;
    Ok(outcome)
}

/// Runs the plan of `seed` on a runtime of its own, with one worker thread
/// so that the order of the work is the order of the plan.
pub fn run_seed_blocking(seed: u64, options: Options) -> Result<Outcome, Failure> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Failure::Setup(error.to_string()))?
        .block_on(run_seed(seed, options))
}
