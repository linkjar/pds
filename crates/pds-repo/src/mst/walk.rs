//! Traversals of a whole tree, and the checks that make them safe.
//!
//! A tree that the engine did not build in the same call is untrusted: it
//! comes from an upload, or from a data directory that another server
//! wrote. Blocks are addressed by hash, so a set of blocks cannot hold a
//! cycle, but it can hold a graph: one node reachable by many paths. A walk
//! that follows every path then costs far more than the blocks it reads.
//!
//! Every traversal here goes through one [`Guard`]. It refuses to enter a
//! node twice, and it counts the nodes entered against a budget that the
//! caller derives from the size of the input. The second rule holds even
//! if a future traversal fails to apply the first.
//!
//! [`read_tree`] also checks the shape of the whole tree: every key at the
//! layer of its hash, keys ascending across node boundaries, no empty node
//! below the root. A tree that passes is the one tree its keys can form.

use std::collections::HashSet;

use cid::Cid;
use pds_types::content::{CidKind, repo_cid_kind};

use super::node::{WireNode, layer_of};
use crate::block::BlockStore;
use crate::error::{RepoError, TreeDefect};

/// Bounds a traversal: no node twice, and no more nodes than the budget.
#[derive(Debug)]
pub struct Guard {
    entered: HashSet<Cid>,
    count: usize,
    budget: usize,
}

impl Guard {
    /// A guard that lets a traversal enter at most `budget` nodes. For an
    /// uploaded CAR the budget is its number of blocks; for a stored
    /// repository, its number of block rows.
    pub fn new(budget: usize) -> Self {
        Self {
            entered: HashSet::new(),
            count: 0,
            budget,
        }
    }

    /// Records that the traversal enters `cid`.
    pub fn enter(&mut self, cid: &Cid) -> Result<(), RepoError> {
        if self.entered.contains(cid) {
            return Err(RepoError::tree(*cid, TreeDefect::SharedNode));
        }
        // Counted apart from the set above, so that the bound holds for a
        // traversal that enters nodes without asking whether it has before.
        if self.count >= self.budget {
            return Err(RepoError::WorkLimit(self.budget));
        }
        self.count += 1;
        self.entered.insert(*cid);
        Ok(())
    }

    /// The number of nodes entered.
    pub const fn entered(&self) -> usize {
        self.count
    }
}

/// A pointer to a node must be a dag-cbor SHA-256 CID. Checked before the
/// node counts against the budget: it costs nothing to refuse.
fn check_pointer(cid: &Cid) -> Result<(), RepoError> {
    if repo_cid_kind(cid) == Ok(CidKind::DagCbor) {
        Ok(())
    } else {
        Err(RepoError::tree(*cid, TreeDefect::PointerCid))
    }
}

fn fetch_node<S: BlockStore>(store: &S, cid: &Cid) -> Result<(WireNode, Vec<u8>), RepoError> {
    let bytes = store.get(cid)?.ok_or_else(|| RepoError::missing(*cid))?;
    let node = WireNode::decode(&bytes).map_err(|reason| RepoError::node(*cid, reason))?;
    Ok((node, bytes))
}

/// A tree read in full.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeContents {
    /// Every key with its record, in key order.
    pub leaves: Vec<(String, Cid)>,
    /// Every node, parents before children.
    pub nodes: Vec<Cid>,
}

struct Reader<'a, S> {
    store: &'a S,
    guard: &'a mut Guard,
    out: TreeContents,
}

impl<S: BlockStore> Reader<'_, S> {
    fn subtree(&mut self, cid: &Cid, parent: &Cid, parent_layer: u32) -> Result<(), RepoError> {
        let layer = parent_layer
            .checked_sub(1)
            .ok_or_else(|| RepoError::tree(*parent, TreeDefect::BelowLayerZero))?;
        check_pointer(cid)?;
        self.guard.enter(cid)?;
        let (node, _) = fetch_node(self.store, cid)?;
        if node.left.is_none() && node.entries.is_empty() {
            return Err(RepoError::tree(*cid, TreeDefect::EmptyNode));
        }
        self.node(cid, &node, layer)
    }

    fn node(&mut self, cid: &Cid, node: &WireNode, layer: u32) -> Result<(), RepoError> {
        self.out.nodes.push(*cid);
        if let Some(left) = &node.left {
            self.subtree(left, cid, layer)?;
        }
        for entry in &node.entries {
            if layer_of(entry.key.as_bytes()) != layer {
                return Err(RepoError::tree(*cid, TreeDefect::KeyLayer));
            }
            if self
                .out
                .leaves
                .last()
                .is_some_and(|(prev, _)| prev.as_str() >= entry.key.as_str())
            {
                return Err(RepoError::tree(*cid, TreeDefect::KeyOrder));
            }
            if repo_cid_kind(&entry.value) != Ok(CidKind::DagCbor) {
                return Err(RepoError::tree(*cid, TreeDefect::PointerCid));
            }
            self.out.leaves.push((entry.key.clone(), entry.value));
            if let Some(tree) = &entry.tree {
                self.subtree(tree, cid, layer)?;
            }
        }
        Ok(())
    }
}

/// Reads and checks the whole tree below `root`.
///
/// Fails when a node is missing or does not decode, when a node is
/// reachable twice, when the walk exceeds the guard's budget, and when the
/// tree has any [`TreeDefect`]. On success the tree is canonical: its root
/// CID is the one the same keys and records give when written one by one.
pub fn read_tree<S: BlockStore>(
    store: &S,
    root: &Cid,
    guard: &mut Guard,
) -> Result<TreeContents, RepoError> {
    check_pointer(root)?;
    guard.enter(root)?;
    let (node, _) = fetch_node(store, root)?;
    let mut reader = Reader {
        store,
        guard,
        out: TreeContents::default(),
    };
    let Some(first) = node.entries.first() else {
        if node.left.is_some() {
            return Err(RepoError::tree(*root, TreeDefect::LeaflessRoot));
        }
        reader.out.nodes.push(*root);
        return Ok(reader.out);
    };
    let layer = layer_of(first.key.as_bytes());
    reader.node(root, &node, layer)?;
    Ok(reader.out)
}

/// Visits the blocks of a tree in the order the Reference exports them:
/// nodes layer by layer from the root, then records in the order their
/// keys first name them, each record once.
///
/// `block` receives each CID with its bytes. A shared node fails the
/// export; shape defects other than that are not looked for, because the
/// export copies blocks and does not interpret keys.
pub fn export_blocks<S, F>(
    store: &S,
    root: &Cid,
    guard: &mut Guard,
    mut block: F,
) -> Result<(), RepoError>
where
    S: BlockStore,
    F: FnMut(&Cid, &[u8]) -> Result<(), RepoError>,
{
    let mut records: Vec<Cid> = Vec::new();
    let mut seen_records: HashSet<Cid> = HashSet::new();
    let mut layer = vec![*root];
    while !layer.is_empty() {
        let mut next = Vec::new();
        for cid in &layer {
            check_pointer(cid)?;
            guard.enter(cid)?;
            let (node, bytes) = fetch_node(store, cid)?;
            block(cid, &bytes)?;
            next.extend(node.left);
            for entry in node.entries {
                if seen_records.insert(entry.value) {
                    records.push(entry.value);
                }
                next.extend(entry.tree);
            }
        }
        layer = next;
    }
    for cid in &records {
        let bytes = store.get(cid)?.ok_or_else(|| RepoError::missing(*cid))?;
        block(cid, &bytes)?;
    }
    Ok(())
}
