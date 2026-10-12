// Included by lib.rs once for each version of rsky-repo, with `rr` naming
// the crate and `NAME` the version.

use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use lexicon_cid::Cid;
use rr::block_map::BlockMap;
use rr::car::blocks_to_car_file;
use rr::repo::Repo;
use rr::storage::memory_blockstore::MemoryBlockstore;
use rr::storage::types::RepoStorage;
use rr::sync::provider::get_full_repo;
use rr::types::{
    RecordCreateOrUpdateOp, RecordWriteEnum, RecordWriteOp, RepoRecord, WriteOpAction,
};
use secp256k1::{Keypair, Secp256k1, SecretKey};
use tokio::runtime::Runtime;
use tokio::sync::RwLock;

use crate::{Engine, ExportSample, WriteSample, alloc, record_json, rkey};

/// The storage handle `rsky-repo` takes everywhere.
pub type Storage = Arc<RwLock<dyn RepoStorage>>;

/// A current-thread tokio runtime.
///
/// # Panics
/// When the runtime cannot be built.
pub fn runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
}

/// The fixed key as `rsky-repo` wants it.
///
/// # Panics
/// When the fixed key of this crate does not parse.
pub fn keypair() -> Keypair {
    let bytes = data_encoding::HEXLOWER
        .decode(crate::KEY_HEX.as_bytes())
        .expect("hex");
    let secret = SecretKey::from_slice(&bytes).expect("key");
    Keypair::from_secret_key(&Secp256k1::new(), &secret)
}

/// A record as `rsky-repo` takes it, from JSON through its own serde
/// implementation.
///
/// # Panics
/// When `rsky-repo` does not accept the JSON as a record.
pub fn record(json: &serde_json::Value) -> RepoRecord {
    serde_json::from_value(json.clone()).expect("a record rsky-repo accepts")
}

/// A new memory block store behind the lock.
pub fn memory_storage() -> (MemoryBlockstore, Storage) {
    let store = MemoryBlockstore::default();
    (store.clone(), Arc::new(RwLock::new(store)))
}

/// One repository on `rsky-repo`.
pub struct Rsky {
    rt: Runtime,
    blocks: MemoryBlockstore,
    storage: Storage,
    head: Option<Cid>,
    records: usize,
    keypair: Keypair,
}

impl Rsky {
    /// An empty store and the fixed key.
    pub fn new() -> Self {
        let (blocks, storage) = memory_storage();
        Self {
            rt: runtime(),
            blocks,
            storage,
            head: None,
            records: 0,
            keypair: keypair(),
        }
    }
}

impl Default for Rsky {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine for Rsky {
    fn name(&self) -> &'static str {
        NAME
    }

    fn records(&self) -> usize {
        self.records
    }

    fn write(&mut self, count: usize) -> WriteSample {
        let ops: Vec<RecordCreateOrUpdateOp> = (self.records..self.records + count)
            .map(|n| RecordCreateOrUpdateOp {
                action: WriteOpAction::Create,
                collection: crate::NOTE.to_owned(),
                rkey: rkey(n),
                record: record(&record_json(n)),
            })
            .collect();
        let (storage, keypair, head) = (self.storage.clone(), self.keypair, self.head);

        let start = Instant::now();
        let (commit, event_len) = self.rt.block_on(async move {
            let commit = match head {
                None => Repo::format_init_commit(storage.clone(), crate::DID.to_owned(), &keypair, Some(ops))
                    .await
                    .expect("init commit"),
                Some(head) => {
                    let mut repo = Repo::load(storage.clone(), Some(head)).await.expect("head");
                    let writes = ops.into_iter().map(RecordWriteOp::Create).collect();
                    repo.format_commit(RecordWriteEnum::List(writes), &keypair)
                        .await
                        .expect("commit")
                }
            };
            let event = blocks_to_car_file(Some(&commit.cid), commit.relevant_blocks.clone())
                .await
                .expect("car");
            storage
                .read()
                .await
                .apply_commit(commit.clone(), None)
                .await
                .expect("apply");
            (commit, event.len())
        });
        let elapsed = start.elapsed();

        self.head = Some(commit.cid);
        self.records += count;
        WriteSample {
            elapsed,
            event_car_len: event_len,
            new_blocks: commit.new_blocks.size(),
            removed_blocks: commit.removed_cids.size(),
        }
    }

    fn export(&mut self) -> ExportSample {
        let (storage, head) = (self.storage.clone(), self.head.expect("a commit"));
        let before = alloc::reset_peak();
        let start = Instant::now();
        let car_len = self.rt.block_on(async move {
            let stream = get_full_repo(storage, head).await.expect("export");
            futures::pin_mut!(stream);
            let mut total = 0usize;
            while let Some(chunk) = stream.next().await {
                total += chunk.expect("chunk").len();
            }
            total
        });
        ExportSample {
            elapsed: start.elapsed(),
            car_len,
            peak_heap: alloc::peak().saturating_sub(before),
        }
    }

    fn tree_root(&mut self) -> String {
        let (storage, head) = (self.storage.clone(), self.head.expect("a commit"));
        self.rt.block_on(async move {
            Repo::load(storage, Some(head))
                .await
                .expect("head")
                .commit
                .data
                .to_string()
        })
    }

    fn stored_blocks(&mut self) -> usize {
        let blocks = self.blocks.blocks.clone();
        self.rt.block_on(async move { blocks.read().await.size() })
    }
}

/// The blocks of a CAR in a new `rsky-repo` store.
///
/// # Panics
/// When `rsky-repo` cannot read the CAR.
pub fn storage_from_car(rt: &Runtime, bytes: &[u8]) -> (Cid, BlockMap, Storage) {
    rt.block_on(async {
        let car = rr::car::read_car_with_root(bytes.to_vec())
            .await
            .expect("a CAR rsky-repo reads");
        let store = MemoryBlockstore::new(Some(car.blocks.clone()))
            .await
            .expect("store");
        let storage: Storage = Arc::new(RwLock::new(store));
        (car.root, car.blocks, storage)
    })
}
