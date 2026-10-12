//! The engine of `crates/pds-repo`.

use std::time::Instant;

use lexicon_cid::Cid;
use k256::ecdsa::signature::Signer;
use k256::ecdsa::{Signature, SigningKey};
use pds_repo::{MemoryBlockStore, Repo, SignError, Write, write_car};
use pds_types::{Data, Did, TidGenerator};

use crate::{CountingSink, Engine, ExportSample, WriteSample, alloc, record_json, rkey};

/// One repository on the in-house engine.
pub struct InHouse {
    store: MemoryBlockStore,
    head: Option<Cid>,
    records: usize,
    key: SigningKey,
    did: Did,
    clock: TidGenerator,
}

impl InHouse {
    /// An empty store and the fixed key.
    ///
    /// # Panics
    /// When the fixed key or DID of this crate do not parse.
    pub fn new() -> Self {
        let key = data_encoding::HEXLOWER
            .decode(crate::KEY_HEX.as_bytes())
            .expect("hex");
        Self {
            store: MemoryBlockStore::new(),
            head: None,
            records: 0,
            key: SigningKey::from_slice(&key).expect("key"),
            did: Did::parse(crate::DID).expect("did"),
            clock: TidGenerator::new(),
        }
    }
}

impl Default for InHouse {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine for InHouse {
    fn name(&self) -> &'static str {
        "in-house (pds-repo)"
    }

    fn records(&self) -> usize {
        self.records
    }

    fn write(&mut self, count: usize) -> WriteSample {
        // The server holds parsed and validated values when the engine's
        // work starts; parsing JSON is not part of the sample.
        let values: Vec<(String, Data)> = (self.records..self.records + count)
            .map(|n| {
                let key = format!("{}/{}", crate::NOTE, rkey(n));
                let data = Data::from_json_str(&record_json(n).to_string()).expect("record");
                (key, data)
            })
            .collect();
        let key = &self.key;
        let sign = |message: &[u8]| -> Result<Vec<u8>, SignError> {
            let sig: Signature = key.sign(message);
            Ok(sig.to_bytes().to_vec())
        };

        let start = Instant::now();
        let encoded: Vec<Vec<u8>> = values.iter().map(|(_, data)| data.to_dag_cbor()).collect();
        let writes: Vec<Write<'_>> = values
            .iter()
            .zip(&encoded)
            .map(|((key, _), record)| Write::Create { key, record })
            .collect();
        let rev = self.clock.next();
        let commit = match self.head {
            None => Repo::format_init_commit(&self.store, &self.did, rev, &writes, sign),
            Some(head) => Repo::load(&self.store, head)
                .expect("head")
                .format_commit(rev, &writes, sign),
        }
        .expect("commit");
        let event = write_car(Some(&commit.cid), &commit.relevant_blocks);
        for removed in &commit.removed_cids {
            self.store.remove(removed);
        }
        self.store.put_all(&commit.new_blocks);
        let elapsed = start.elapsed();

        self.head = Some(commit.cid);
        self.records += count;
        WriteSample {
            elapsed,
            event_car_len: event.len(),
            new_blocks: commit.new_blocks.len(),
            removed_blocks: commit.removed_cids.len(),
        }
    }

    fn export(&mut self) -> ExportSample {
        let head = self.head.expect("a commit");
        let before = alloc::reset_peak();
        let start = Instant::now();
        let repo = Repo::load(&self.store, head).expect("head");
        let sink = repo
            .export(CountingSink::default(), self.store.len())
            .expect("export");
        let elapsed = start.elapsed();
        ExportSample {
            elapsed,
            car_len: sink.0,
            peak_heap: alloc::peak().saturating_sub(before),
        }
    }

    fn tree_root(&mut self) -> String {
        let head = self.head.expect("a commit");
        Repo::load(&self.store, head)
            .expect("head")
            .commit()
            .data
            .to_string()
    }

    fn stored_blocks(&mut self) -> usize {
        self.store.len()
    }
}
