//! The tree against a model: whatever the order of writes, the tree equals
//! the one built from its keys, the stored blocks are exactly the blocks
//! the tree reaches, and the tree passes the checks of an import.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use std::collections::{BTreeMap, HashSet};

use cid::Cid;
use pds_repo::mst::{Guard, Tree, read_tree};
use pds_repo::{BlockStore, MemoryBlockStore, RepoError};
use pds_types::content::cid_for_cbor;
use proptest::prelude::*;

fn value(n: u32) -> Cid {
    cid_for_cbor(&n.to_be_bytes())
}

/// Keys from a small space, so that updates, deletes and collisions occur.
fn key(n: u16) -> String {
    format!("com.example.c{}/k{n:05}", n % 3)
}

/// The root of the tree with exactly these entries, built in key order.
fn root_from_scratch(model: &BTreeMap<String, Cid>) -> Cid {
    let mut tree = Tree::empty(MemoryBlockStore::new());
    for (k, v) in model {
        tree.add(k, *v).unwrap();
    }
    tree.root().unwrap()
}

#[derive(Debug, Clone)]
enum Op {
    Put(u16, u32),
    Delete(u16),
}

fn ops() -> impl Strategy<Value = Vec<Vec<Op>>> {
    let op = prop_oneof![
        3 => (0u16..400, any::<u32>()).prop_map(|(k, v)| Op::Put(k, v)),
        1 => (0u16..400).prop_map(Op::Delete),
    ];
    proptest::collection::vec(proptest::collection::vec(op, 1..40), 1..12)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn batches_of_writes_match_the_model(batches in ops()) {
        let mut store = MemoryBlockStore::new();
        let mut model: BTreeMap<String, Cid> = BTreeMap::new();
        // The empty tree is stored first, as a new repository has it.
        let mut tree = Tree::empty(&store);
        let first = tree.changes().unwrap();
        let mut root = first.root;
        store.put_all(&first.new_blocks);

        for batch in batches {
            let mut tree = Tree::load(&store, root).unwrap();
            for op in &batch {
                match *op {
                    Op::Put(k, v) => {
                        let (k, v) = (key(k), value(v));
                        match model.insert(k.clone(), v) {
                            Some(prev) => prop_assert_eq!(tree.update(&k, v).unwrap(), prev),
                            None => tree.add(&k, v).unwrap(),
                        }
                    }
                    Op::Delete(k) => {
                        let k = key(k);
                        match model.remove(&k) {
                            Some(v) => prop_assert_eq!(tree.delete(&k).unwrap(), v),
                            None => prop_assert!(matches!(tree.delete(&k), Err(RepoError::KeyNotFound(_)))),
                        }
                    }
                }
            }
            let changes = tree.changes().unwrap();
            drop(tree);
            // The root is a function of the entries alone (SPEC 6.6).
            prop_assert_eq!(changes.root, root_from_scratch(&model));
            // No block is both new and removed, and new blocks are new.
            for cid in changes.new_blocks.cids() {
                prop_assert!(!changes.removed.contains(cid));
                prop_assert!(!store.has(cid).unwrap(), "a new block was stored already");
            }
            for cid in &changes.removed {
                prop_assert!(store.has(cid).unwrap(), "a removed block was never stored");
                store.remove(cid);
            }
            store.put_all(&changes.new_blocks);
            root = changes.root;

            // The stored tree passes the checks of an import: every key at
            // its layer, in order, and no node reached twice.
            let mut guard = Guard::new(store.len());
            let contents = read_tree(&store, &root, &mut guard).unwrap();
            let leaves: Vec<(String, Cid)> = model.iter().map(|(k, v)| (k.clone(), *v)).collect();
            prop_assert_eq!(&contents.leaves, &leaves);
            // The store holds the reachable nodes and nothing else.
            let reachable: HashSet<Cid> = contents.nodes.iter().copied().collect();
            prop_assert_eq!(reachable.len(), contents.nodes.len());
            prop_assert_eq!(reachable.len(), store.len());

            // Reads agree with the model.
            let mut tree = Tree::load(&store, root).unwrap();
            for (k, v) in model.iter().take(8) {
                prop_assert_eq!(tree.get(k).unwrap(), Some(*v));
            }
            prop_assert_eq!(tree.get("com.example.c0/absent").unwrap(), None);
        }
    }

    #[test]
    fn the_root_does_not_depend_on_the_order_of_writes(
        keys in proptest::collection::btree_set(0u16..5000, 1..300),
        seed in any::<u64>(),
    ) {
        let model: BTreeMap<String, Cid> = keys.iter().map(|k| (key(*k), value(u32::from(*k)))).collect();
        let want = root_from_scratch(&model);
        // A shuffle by a multiplicative hash of the seed.
        let mut shuffled: Vec<&String> = model.keys().collect();
        shuffled.sort_by_key(|k| {
            k.bytes().fold(seed | 1, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
        });
        let mut tree = Tree::empty(MemoryBlockStore::new());
        for k in shuffled {
            tree.add(k, model[k]).unwrap();
        }
        prop_assert_eq!(tree.root().unwrap(), want);
    }
}

#[test]
fn write_errors_leave_the_tree_usable() {
    let mut tree = Tree::empty(MemoryBlockStore::new());
    let v = value(1);
    tree.add("com.example.a/1", v).unwrap();
    let root = tree.root().unwrap();
    assert!(matches!(
        tree.add("com.example.a/1", v),
        Err(RepoError::KeyExists(_))
    ));
    assert!(matches!(
        tree.update("com.example.a/2", v),
        Err(RepoError::KeyNotFound(_))
    ));
    assert!(matches!(
        tree.delete("com.example.a/2"),
        Err(RepoError::KeyNotFound(_))
    ));
    for bad in ["", "nokey", "a/b/c", "a/ b"] {
        assert!(
            matches!(tree.add(bad, v), Err(RepoError::InvalidKey(_))),
            "{bad:?}"
        );
        assert!(matches!(tree.update(bad, v), Err(RepoError::InvalidKey(_))));
        assert!(matches!(tree.delete(bad), Err(RepoError::InvalidKey(_))));
    }
    assert_eq!(tree.root().unwrap(), root);
    // Deleting the last key gives the empty tree, whose root is known.
    assert_eq!(tree.delete("com.example.a/1").unwrap(), v);
    assert_eq!(
        tree.root().unwrap().to_string(),
        "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm"
    );
}
