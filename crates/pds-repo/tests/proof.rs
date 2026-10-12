//! The proof of `com.atproto.sync.getRecord`: the blocks of
//! `Repo::record_proof` are enough to read the key under the commit, each
//! of them is needed, and they come in the Reference's order.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use cid::Cid;
use pds_repo::mst::{WireNode, layer_of};
use pds_repo::{BlockMap, MemoryBlockStore, Repo, RepoError, Write};
use pds_types::content::cid_for_cbor;
use pds_types::{Did, Tid};

/// Enough keys for a tree of several layers. A commit takes at most 200
/// writes.
const KEYS: u32 = 200;

fn key(n: u32) -> String {
    format!("com.example.post/k{n:05}")
}

fn record(n: u32) -> Vec<u8> {
    // DAG-CBOR: a byte string of four bytes.
    let mut bytes = vec![0x44];
    bytes.extend_from_slice(&n.to_be_bytes());
    bytes
}

/// A repository with one commit that holds `KEYS` records.
fn repository() -> (MemoryBlockStore, Cid) {
    let did = Did::parse("did:plc:aaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let rev = Tid::parse("3jqfcqzm3fn2j").unwrap();
    let records: Vec<(String, Vec<u8>)> = (0..KEYS).map(|n| (key(n), record(n))).collect();
    let writes: Vec<Write<'_>> = records
        .iter()
        .map(|(key, record)| Write::Create { key, record })
        .collect();
    let mut store = MemoryBlockStore::new();
    // Nothing here verifies the signature.
    let commit = Repo::format_init_commit(&store, &did, rev, &writes, |_| Ok(vec![0; 64])).unwrap();
    store.put_all(&commit.new_blocks);
    (store, commit.cid)
}

/// The links of a node, in the order a search can follow them.
fn links(node: &[u8]) -> Vec<Cid> {
    let node = WireNode::decode(node).unwrap();
    node.left
        .into_iter()
        .chain(node.entries.into_iter().filter_map(|entry| entry.tree))
        .collect()
}

/// Checks the order of a proof: the commit, the record when `value` is
/// given, then tree nodes from the root down, each a child of the one before.
/// Returns the number of nodes.
fn check_order(repo: &Repo<&MemoryBlockStore>, proof: &BlockMap, value: Option<Cid>) -> usize {
    let cids: Vec<Cid> = proof.cids().copied().collect();
    assert_eq!(cids[0], *repo.cid());
    let nodes = match value {
        Some(value) => {
            assert_eq!(cids[1], value);
            &cids[2..]
        }
        None => &cids[1..],
    };
    assert_eq!(nodes[0], repo.commit().data);
    for pair in nodes.windows(2) {
        let parent = proof.get(&pair[0]).unwrap();
        assert!(links(parent).contains(&pair[1]));
    }
    nodes.len()
}

/// Checks that reading `key` from `proof` alone gives `want`, and that the
/// read fails for a missing block when any one block of the proof is absent.
fn check_sufficient_and_minimal(proof: &BlockMap, root: Cid, key: &str, want: Option<&[u8]>) {
    let alone = Repo::load(proof, root).unwrap();
    let got = alone.get_record(key).unwrap();
    assert_eq!(got.as_ref().map(|(_, bytes)| bytes.as_slice()), want);
    if let (Some((cid, bytes)), Some(_)) = (&got, want) {
        assert_eq!(*cid, cid_for_cbor(bytes));
    }

    for absent in proof.cids() {
        let mut partial = BlockMap::new();
        for (cid, bytes) in proof.iter().filter(|(cid, _)| *cid != absent) {
            partial.insert(*cid, bytes.to_vec());
        }
        let read = Repo::load(&partial, root).and_then(|repo| repo.get_record(key));
        assert!(
            matches!(&read, Err(RepoError::MissingBlock(cid)) if **cid == *absent),
            "{key} without {absent}: {read:?}"
        );
    }
}

#[test]
fn the_proof_of_a_present_key_reads_the_record() {
    let (store, root) = repository();
    let repo = Repo::load(&store, root).unwrap();
    let mut deepest = 0;
    for n in 0..KEYS {
        let key = key(n);
        let proof = repo.record_proof(&key).unwrap();
        let value = cid_for_cbor(&record(n));
        deepest = deepest.max(check_order(&repo, &proof, Some(value)));
        check_sufficient_and_minimal(&proof, root, &key, Some(&record(n)));
    }
    // The tree has several layers, so some paths pass more than one node.
    assert!(deepest >= 3, "the deepest path has {deepest} nodes");
}

#[test]
fn the_proof_of_an_absent_key_reads_nothing() {
    let (store, root) = repository();
    let repo = Repo::load(&store, root).unwrap();
    let mut deepest = 0;
    // Keys before, between and after the stored ones. Their own layers
    // differ, so the search for them stops at different depths.
    let absent = (0..400)
        .map(|n| format!("com.example.post/k{n:05}x"))
        .chain([
            "com.example.a/first".to_owned(),
            "com.example.z/last".to_owned(),
        ]);
    let mut layers = std::collections::BTreeSet::new();
    for key in absent {
        layers.insert(layer_of(key.as_bytes()));
        let proof = repo.record_proof(&key).unwrap();
        deepest = deepest.max(check_order(&repo, &proof, None));
        check_sufficient_and_minimal(&proof, root, &key, None);
    }
    assert!(deepest >= 3, "the deepest path has {deepest} nodes");
    assert!(
        layers.len() >= 3,
        "the absent keys are in layers {layers:?}"
    );
}
