//! The upstream interop vectors for the firehose commit proofs
//! (`interop/firehose/commit-proof-fixtures.json`, CC0). The MST vectors
//! for key heights and common prefixes run beside the node codec.
//!
//! Each case names the keys of a tree, the keys a commit adds and deletes,
//! the root before and after, and the tree blocks the commit must carry so
//! that a consumer can check it against the previous root.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use std::collections::BTreeSet;
use std::path::PathBuf;

use cid::Cid;
use pds_repo::mst::Tree;
use pds_repo::{BlockMap, MemoryBlockStore};
use serde_json::Value;

/// Vectors that do not pass, with the finding that explains each. None.
const KNOWN_FAILURES: &[(&str, &str)] = &[];

fn cid(v: &Value) -> Cid {
    Cid::try_from(v.as_str().expect("a CID string")).expect("a CID")
}

fn strings(v: &Value) -> Vec<&str> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|s| s.as_str().expect("string"))
        .collect()
}

#[test]
fn commit_proof_fixtures() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../interop/firehose/commit-proof-fixtures.json");
    let cases: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(path).expect("vectors")).expect("json");
    assert_eq!(cases.len(), 6);
    for case in &cases {
        let comment = case["comment"].as_str().unwrap();
        assert!(!KNOWN_FAILURES.iter().any(|(name, _)| *name == comment));
        let value = cid(&case["leafValue"]);

        // The tree before the commit, stored.
        let mut store = MemoryBlockStore::new();
        let mut tree = Tree::empty(&store);
        for key in strings(&case["keys"]) {
            tree.add(key, value).unwrap();
        }
        let before = tree.changes().unwrap();
        assert_eq!(
            before.root,
            cid(&case["rootBeforeCommit"]),
            "{comment}: root before"
        );
        drop(tree);
        store.put_all(&before.new_blocks);

        // The commit.
        let mut tree = Tree::load(&store, before.root).unwrap();
        let (adds, dels) = (strings(&case["adds"]), strings(&case["dels"]));
        for key in &adds {
            tree.add(key, value).unwrap();
        }
        for key in &dels {
            tree.delete(key).unwrap();
        }
        assert_eq!(
            tree.root().unwrap(),
            cid(&case["rootAfterCommit"]),
            "{comment}: root after"
        );

        // The proof: for every key the commit touches, the nodes that
        // prove it and its neighbours in the new tree.
        let mut proof = BlockMap::new();
        for key in adds.iter().chain(&dels) {
            tree.covering_proof(key, &mut proof).unwrap();
        }
        let got: BTreeSet<String> = proof.cids().map(ToString::to_string).collect();
        let want: BTreeSet<String> = strings(&case["blocksInProof"])
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(got, want, "{comment}: blocks in proof");
    }
}
