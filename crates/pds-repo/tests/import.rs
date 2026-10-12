//! Trees that are not valid, each built by hand in canonical encoding so
//! that exactly one rule is broken. `read_tree` names the defect; an import
//! refuses the CAR.
//!
//! The keys are upstream's example keys, whose names give their layer:
//! `B1/986427` is at layer 1.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use cid::Cid;
use pds_repo::mst::{Guard, Tree, WireEntry, WireNode, layer_of, read_tree};
use pds_repo::{BlockMap, MemoryBlockStore, RepoError, TreeDefect};
use pds_types::content::{cid_for_cbor, cid_for_raw};

const A0: &str = "A0/374913";
const B1: &str = "B1/986427";
const C0: &str = "C0/451630";
const D2: &str = "D2/269196";
const F1: &str = "F1/085263";

fn value() -> Cid {
    cid_for_cbor(b"a record")
}

fn entry(key: &str, tree: Option<Cid>) -> WireEntry {
    WireEntry {
        key: key.to_owned(),
        value: value(),
        tree,
    }
}

fn put(blocks: &mut BlockMap, left: Option<Cid>, entries: Vec<WireEntry>) -> Cid {
    blocks.insert_cbor(WireNode { left, entries }.encode())
}

fn defect(blocks: &BlockMap, root: &Cid) -> TreeDefect {
    match read_tree(blocks, root, &mut Guard::new(blocks.len())) {
        Err(RepoError::Tree { defect, .. }) => defect,
        other => panic!("expected a tree defect, got {other:?}"),
    }
}

#[test]
fn the_example_keys_have_the_layers_their_names_give() {
    for (key, layer) in [(A0, 0), (B1, 1), (C0, 0), (D2, 2), (F1, 1)] {
        assert_eq!(layer_of(key.as_bytes()), layer, "{key}");
    }
}

#[test]
fn a_node_reached_twice() {
    // The same layer-0 node on both sides of B1. Everything else about the
    // encoding is canonical, so this is the rule on its own.
    let mut blocks = BlockMap::new();
    let shared = put(&mut blocks, None, vec![entry(A0, None)]);
    let root = put(&mut blocks, Some(shared), vec![entry(B1, Some(shared))]);
    match read_tree(&blocks, &root, &mut Guard::new(blocks.len())) {
        Err(RepoError::Tree {
            cid,
            defect: TreeDefect::SharedNode,
        }) => assert_eq!(*cid, shared),
        other => panic!("{other:?}"),
    }
    // Two records with the same content share a value CID. That is valid.
    let mut blocks = BlockMap::new();
    let root = put(&mut blocks, None, vec![entry(A0, None), entry(C0, None)]);
    let contents = read_tree(&blocks, &root, &mut Guard::new(1)).unwrap();
    assert_eq!(contents.leaves[0].1, contents.leaves[1].1);
}

#[test]
fn keys_out_of_order_across_nodes() {
    // C0 sorts after B1 and hangs to its left.
    let mut blocks = BlockMap::new();
    let left = put(&mut blocks, None, vec![entry(C0, None)]);
    let root = put(&mut blocks, Some(left), vec![entry(B1, None)]);
    assert_eq!(defect(&blocks, &root), TreeDefect::KeyOrder);
    // A0 sorts before B1 and hangs to its right.
    let mut blocks = BlockMap::new();
    let right = put(&mut blocks, None, vec![entry(A0, None)]);
    let root = put(&mut blocks, None, vec![entry(B1, Some(right))]);
    assert_eq!(defect(&blocks, &root), TreeDefect::KeyOrder);
}

#[test]
fn a_key_at_the_wrong_layer() {
    // B1 beside A0 in a layer-0 node.
    let mut blocks = BlockMap::new();
    let root = put(&mut blocks, None, vec![entry(A0, None), entry(B1, None)]);
    assert_eq!(defect(&blocks, &root), TreeDefect::KeyLayer);
    // C0 directly below D2: one layer too high.
    let mut blocks = BlockMap::new();
    let child = put(&mut blocks, None, vec![entry(C0, None)]);
    let root = put(&mut blocks, Some(child), vec![entry(D2, None)]);
    assert_eq!(defect(&blocks, &root), TreeDefect::KeyLayer);
}

#[test]
fn nodes_where_none_may_be() {
    // A subtree below layer zero.
    let mut blocks = BlockMap::new();
    let child = put(&mut blocks, None, vec![entry(C0, None)]);
    let root = put(&mut blocks, None, vec![entry(A0, Some(child))]);
    assert_eq!(defect(&blocks, &root), TreeDefect::BelowLayerZero);
    // An empty node below the root.
    let mut blocks = BlockMap::new();
    let empty = put(&mut blocks, None, vec![]);
    let root = put(&mut blocks, Some(empty), vec![entry(B1, None)]);
    assert_eq!(defect(&blocks, &root), TreeDefect::EmptyNode);
    // A root that holds only a subtree.
    let mut blocks = BlockMap::new();
    let child = put(&mut blocks, None, vec![entry(A0, None)]);
    let root = put(&mut blocks, Some(child), vec![]);
    assert_eq!(defect(&blocks, &root), TreeDefect::LeaflessRoot);
}

#[test]
fn pointers_must_be_dag_cbor_sha256() {
    let raw = cid_for_raw(b"a blob");
    let mut blocks = BlockMap::new();
    let root = put(
        &mut blocks,
        None,
        vec![WireEntry {
            key: A0.to_owned(),
            value: raw,
            tree: None,
        }],
    );
    assert_eq!(defect(&blocks, &root), TreeDefect::PointerCid);
    let mut blocks = BlockMap::new();
    let root = put(&mut blocks, Some(raw), vec![entry(B1, None)]);
    assert_eq!(defect(&blocks, &root), TreeDefect::PointerCid);
}

#[test]
fn missing_blocks_and_the_work_limit() {
    let mut blocks = BlockMap::new();
    let absent = cid_for_cbor(b"not in the CAR");
    let root = put(&mut blocks, Some(absent), vec![entry(B1, None)]);
    assert!(matches!(
        read_tree(&blocks, &root, &mut Guard::new(8)),
        Err(RepoError::MissingBlock(c)) if *c == absent
    ));

    // A valid tree of three nodes: D2 over a node without keys over A0.
    let mut tree = Tree::empty(MemoryBlockStore::new());
    tree.add(A0, value()).unwrap();
    tree.add(D2, value()).unwrap();
    let changes = tree.changes().unwrap();
    assert_eq!(changes.new_blocks.len(), 3);
    let read = |budget| read_tree(&changes.new_blocks, &changes.root, &mut Guard::new(budget));
    assert_eq!(read(3).unwrap().nodes.len(), 3);
    assert!(matches!(read(2), Err(RepoError::WorkLimit(2))));
    assert!(matches!(read(0), Err(RepoError::WorkLimit(0))));
    // The same tree built by hand is the same tree.
    let mut blocks = BlockMap::new();
    let bottom = put(&mut blocks, None, vec![entry(A0, None)]);
    let middle = put(&mut blocks, Some(bottom), vec![]);
    let root = put(&mut blocks, Some(middle), vec![entry(D2, None)]);
    assert_eq!(root, changes.root);
}

#[test]
fn writes_on_a_stored_tree_that_is_not_valid_stop_with_an_error() {
    // A data directory from another server may hold such a tree. A write
    // must fail, not loop and not panic.
    let mut blocks = BlockMap::new();
    let child = put(&mut blocks, None, vec![entry(C0, None)]);
    let root = put(&mut blocks, None, vec![entry(A0, Some(child))]);
    let mut tree = Tree::load(&blocks, root).unwrap();
    assert_eq!(tree.get(C0).unwrap(), None);
    assert!(tree.delete(C0).is_err());
    assert!(tree.update(C0, value()).is_err());

    // A chain of nodes without keys, longer than any tree is deep.
    let mut blocks = BlockMap::new();
    let mut top = put(&mut blocks, None, vec![entry(A0, None)]);
    for _ in 0..1000 {
        top = put(&mut blocks, Some(top), vec![]);
    }
    assert!(Tree::load(&blocks, top).is_err());
    assert_eq!(defect(&blocks, &top), TreeDefect::LeaflessRoot);
}
