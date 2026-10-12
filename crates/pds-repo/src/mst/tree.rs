//! The Merkle search tree as a structure that changes in place.
//!
//! [`Tree`] loads nodes from a block store as an operation reaches them and
//! keeps them in memory. A write touches the nodes on one path; every other
//! node stays a CID. After the writes, [`Tree::changes`] encodes the touched
//! nodes and reports which blocks are new and which stored blocks the tree
//! no longer uses, so a commit writes and deletes exactly those (SPEC §6.3).
//!
//! The shape of the tree is a function of its keys alone: a key lives at
//! the layer of its hash ([`layer_of`]), a node holds the keys of one layer
//! that no key of a higher layer separates, and subtrees hang between them.
//! The same set of keys and values gives the same root CID in any order of
//! writes, here and in the Reference (SPEC §6.6).
//!
//! Every descent carries the layer of the node it is in and stops below
//! layer zero, so the depth of recursion is bounded by the root's layer even
//! when the store holds a tree that is not valid.

use std::collections::HashSet;

use cid::Cid;
use pds_types::Limits;
use pds_types::content::cid_for_cbor;

use super::node::{MAX_LAYER, NodeWriter, WireNode, is_valid_key, layer_of};
use crate::block::{BlockMap, BlockStore};
use crate::error::{RepoError, TreeDefect};

/// The encoded form of a node whose content has not changed since.
#[derive(Debug, Clone)]
struct Sealed {
    cid: Cid,
    bytes: Vec<u8>,
}

#[derive(Debug)]
enum Child {
    /// A subtree that was not loaded: its root is in the store.
    Stored(Cid),
    /// A subtree in memory.
    Loaded(Box<Node>),
}

#[derive(Debug)]
struct Entry {
    key: String,
    value: Cid,
    /// The subtree between this key and the next.
    right: Option<Child>,
}

#[derive(Debug, Default)]
struct Node {
    /// The subtree before the first key.
    left: Option<Child>,
    entries: Vec<Entry>,
    /// Present while the content equals the encoded block.
    sealed: Option<Sealed>,
    /// Whether the sealed block came from the store.
    stored: bool,
}

impl Node {
    fn from_wire(wire: WireNode, cid: Cid, bytes: Vec<u8>) -> Self {
        Self {
            left: wire.left.map(Child::Stored),
            entries: wire
                .entries
                .into_iter()
                .map(|e| Entry {
                    key: e.key,
                    value: e.value,
                    right: e.tree.map(Child::Stored),
                })
                .collect(),
            sealed: Some(Sealed { cid, bytes }),
            stored: true,
        }
    }

    fn parent_of(child: Child) -> Self {
        Self {
            left: Some(child),
            ..Self::default()
        }
    }

    fn is_empty(&self) -> bool {
        self.left.is_none() && self.entries.is_empty()
    }

    /// The index of the first entry whose key is not below `key`.
    fn position(&self, key: &str) -> usize {
        self.entries.partition_point(|e| e.key.as_str() < key)
    }

    fn has_key_at(&self, i: usize, key: &str) -> bool {
        self.entries.get(i).is_some_and(|e| e.key == key)
    }

    /// The subtree slot before entry `i`: `left` for zero, otherwise the
    /// right slot of entry `i - 1`. `i` may equal the number of entries.
    fn gap_mut(&mut self, i: usize) -> &mut Option<Child> {
        match i.checked_sub(1).and_then(|j| self.entries.get_mut(j)) {
            Some(entry) => &mut entry.right,
            None => &mut self.left,
        }
    }
}

fn read_node<S: BlockStore>(store: &S, cid: &Cid) -> Result<Node, RepoError> {
    let bytes = store.get(cid)?.ok_or_else(|| RepoError::missing(*cid))?;
    let wire = WireNode::decode(&bytes).map_err(|reason| RepoError::node(*cid, reason))?;
    Ok(Node::from_wire(wire, *cid, bytes))
}

/// Loads the subtree root in place and returns it.
fn load<'c, S: BlockStore>(store: &S, child: &'c mut Child) -> Result<&'c mut Node, RepoError> {
    if let Child::Stored(cid) = *child {
        *child = Child::Loaded(Box::new(read_node(store, &cid)?));
    }
    match child {
        Child::Loaded(node) => Ok(node),
        Child::Stored(cid) => Err(RepoError::missing(*cid)),
    }
}

fn into_node<S: BlockStore>(store: &S, child: Child) -> Result<Box<Node>, RepoError> {
    match child {
        Child::Loaded(node) => Ok(node),
        Child::Stored(cid) => read_node(store, &cid).map(Box::new),
    }
}

/// The layer below `layer`, or the defect of a subtree below layer zero.
fn below(layer: u32, node: &Node) -> Result<u32, RepoError> {
    layer.checked_sub(1).ok_or_else(|| match &node.sealed {
        Some(sealed) => RepoError::tree(sealed.cid, TreeDefect::BelowLayerZero),
        None => RepoError::Shape(TreeDefect::BelowLayerZero),
    })
}

/// The mutable state that every tree operation needs.
struct Cx<'a, S> {
    store: &'a S,
    /// CIDs of stored nodes whose content changed or that left the tree.
    removed: &'a mut HashSet<Cid>,
}

impl<S> Cx<'_, S> {
    /// Marks a node as changed. Its stored block, if any, is now unused.
    fn touch(&mut self, node: &mut Node) {
        if let Some(sealed) = node.sealed.take()
            && node.stored
        {
            self.removed.insert(sealed.cid);
        }
        node.stored = false;
    }
}

fn insert<S: BlockStore>(
    cx: &mut Cx<'_, S>,
    node: &mut Node,
    layer: u32,
    key: &str,
    key_layer: u32,
    value: Cid,
) -> Result<(), RepoError> {
    let i = node.position(key);
    if node.has_key_at(i, key) {
        return Err(RepoError::KeyExists(key.to_owned()));
    }
    if key_layer == layer {
        // The subtree in the gap holds keys on both sides of the new key.
        let (lower, upper) = match node.gap_mut(i).take() {
            Some(child) => split(cx, child, below(layer, node)?, key)?,
            None => (None, None),
        };
        cx.touch(node);
        *node.gap_mut(i) = lower;
        node.entries.insert(
            i,
            Entry {
                key: key.to_owned(),
                value,
                right: upper,
            },
        );
        return Ok(());
    }
    let next = below(layer, node)?;
    match node.gap_mut(i) {
        Some(child) => {
            let sub = load(cx.store, child)?;
            insert(cx, sub, next, key, key_layer, value)?;
        }
        slot @ None => {
            // No subtree here yet. The new node may be several layers above
            // the key; the recursion then adds the nodes in between.
            let mut fresh = Node::default();
            insert(cx, &mut fresh, next, key, key_layer, value)?;
            *slot = Some(Child::Loaded(Box::new(fresh)));
        }
    }
    cx.touch(node);
    Ok(())
}

/// Divides a subtree into the part below `key` and the part above it. A
/// part without keys is `None`. A subtree that lies on one side whole comes
/// back unchanged.
fn split<S: BlockStore>(
    cx: &mut Cx<'_, S>,
    child: Child,
    layer: u32,
    key: &str,
) -> Result<(Option<Child>, Option<Child>), RepoError> {
    let mut node = into_node(cx.store, child)?;
    let i = node.position(key);
    if node.has_key_at(i, key) {
        // The key sits below the layer of its hash: the tree is not valid.
        // This is not `KeyExists`, which promises an unchanged tree.
        return Err(RepoError::Shape(TreeDefect::KeyLayer));
    }
    let straddling = node.gap_mut(i).take();
    if straddling.is_none() {
        if i == node.entries.len() {
            return Ok((Some(Child::Loaded(node)), None));
        }
        if i == 0 {
            return Ok((None, Some(Child::Loaded(node))));
        }
    }
    let (lower, upper) = match straddling {
        Some(sub) => split(cx, sub, below(layer, &node)?, key)?,
        None => (None, None),
    };
    cx.touch(&mut node);
    let above = Node {
        left: upper,
        entries: node.entries.split_off(i),
        ..Node::default()
    };
    *node.gap_mut(i) = lower;
    let lower_part = (!node.is_empty()).then_some(Child::Loaded(node));
    let upper_part = (!above.is_empty()).then(|| Child::Loaded(Box::new(above)));
    Ok((lower_part, upper_part))
}

/// Joins two subtrees of one layer whose keys do not interleave: every key
/// of `first` is below every key of `second`.
fn merge<S: BlockStore>(
    cx: &mut Cx<'_, S>,
    first: Child,
    second: Child,
    layer: u32,
) -> Result<Child, RepoError> {
    let mut lower = into_node(cx.store, first)?;
    let mut upper = into_node(cx.store, second)?;
    cx.touch(&mut lower);
    cx.touch(&mut upper);
    let end = lower.entries.len();
    let middle = match (lower.gap_mut(end).take(), upper.left.take()) {
        (Some(before), Some(after)) => Some(merge(cx, before, after, below(layer, &lower)?)?),
        (before, None) => before,
        (None, after) => after,
    };
    *lower.gap_mut(end) = middle;
    lower.entries.append(&mut upper.entries);
    Ok(Child::Loaded(lower))
}

fn remove<S: BlockStore>(
    cx: &mut Cx<'_, S>,
    node: &mut Node,
    layer: u32,
    key: &str,
) -> Result<Cid, RepoError> {
    let i = node.position(key);
    if node.has_key_at(i, key) {
        cx.touch(node);
        let entry = node.entries.remove(i);
        // The subtrees on both sides of the key are neighbours now.
        let joined = match (node.gap_mut(i).take(), entry.right) {
            (Some(before), Some(after)) => Some(merge(cx, before, after, below(layer, node)?)?),
            (before, None) => before,
            (None, after) => after,
        };
        *node.gap_mut(i) = joined;
        return Ok(entry.value);
    }
    // No subtree here means no such key; layer zero never has one.
    if node.gap_mut(i).is_none() {
        return Err(RepoError::KeyNotFound(key.to_owned()));
    }
    let next = below(layer, node)?;
    let Some(child) = node.gap_mut(i) else {
        return Err(RepoError::KeyNotFound(key.to_owned()));
    };
    let sub = load(cx.store, child)?;
    let value = remove(cx, sub, next, key)?;
    if sub.is_empty() {
        *node.gap_mut(i) = None;
    }
    cx.touch(node);
    Ok(value)
}

fn replace<S: BlockStore>(
    cx: &mut Cx<'_, S>,
    node: &mut Node,
    layer: u32,
    key: &str,
    value: Cid,
) -> Result<Cid, RepoError> {
    let i = node.position(key);
    if node.has_key_at(i, key) {
        cx.touch(node);
        return node
            .entries
            .get_mut(i)
            .map(|e| std::mem::replace(&mut e.value, value))
            .ok_or_else(|| RepoError::KeyNotFound(key.to_owned()));
    }
    if node.gap_mut(i).is_none() {
        return Err(RepoError::KeyNotFound(key.to_owned()));
    }
    let next = below(layer, node)?;
    let Some(child) = node.gap_mut(i) else {
        return Err(RepoError::KeyNotFound(key.to_owned()));
    };
    let sub = load(cx.store, child)?;
    let previous = replace(cx, sub, next, key, value)?;
    cx.touch(node);
    Ok(previous)
}

/// Encodes every changed node below and including `node`.
fn seal(node: &mut Node) -> Result<Cid, RepoError> {
    if let Some(sealed) = &node.sealed {
        return Ok(sealed.cid);
    }
    let seal_child = |child: &mut Child| match child {
        Child::Stored(cid) => Ok(*cid),
        Child::Loaded(sub) => seal(sub),
    };
    let left = node.left.as_mut().map(seal_child).transpose()?;
    let mut trees = Vec::with_capacity(node.entries.len());
    for entry in &mut node.entries {
        trees.push(entry.right.as_mut().map(seal_child).transpose()?);
    }
    let mut writer = NodeWriter::new(left.as_ref(), node.entries.len());
    for (entry, tree) in node.entries.iter().zip(&trees) {
        writer.entry(&entry.key, &entry.value, tree.as_ref());
    }
    let bytes = writer.finish();
    if bytes.len() > Limits::RECORD.max_len {
        return Err(RepoError::NodeTooLarge);
    }
    let cid = cid_for_cbor(&bytes);
    node.sealed = Some(Sealed { cid, bytes });
    node.stored = false;
    Ok(cid)
}

/// Collects the blocks of the nodes that are not in the store, parents
/// before children.
fn collect_new(node: &Node, out: &mut BlockMap) {
    if !node.stored
        && let Some(sealed) = &node.sealed
    {
        out.insert(sealed.cid, sealed.bytes.clone());
    }
    let children = node
        .left
        .iter()
        .chain(node.entries.iter().filter_map(|e| e.right.as_ref()));
    for child in children {
        if let Child::Loaded(sub) = child {
            collect_new(sub, out);
        }
    }
}

fn add_block(node: &Node, out: &mut BlockMap) -> Result<(), RepoError> {
    let sealed = node
        .sealed
        .as_ref()
        .ok_or(RepoError::Commit("a proof needs an encoded tree"))?;
    out.insert(sealed.cid, sealed.bytes.clone());
    Ok(())
}

/// The nodes from the root to the node that holds `key`. When the key is
/// absent, the node where the path ends is left out, as in the Reference;
/// the sibling proofs cover it.
fn proof_for_key<S: BlockStore>(
    store: &S,
    node: &mut Node,
    layer: u32,
    key: &str,
    out: &mut BlockMap,
) -> Result<(), RepoError> {
    let i = node.position(key);
    if !node.has_key_at(i, key) {
        let next = below(layer, node);
        let Some(child) = node.gap_mut(i) else {
            return Ok(());
        };
        proof_for_key(store, load(store, child)?, next?, key, out)?;
    }
    add_block(node, out)
}

/// The nodes from the root to the node that holds the greatest key below
/// `key`.
fn proof_for_left_sibling<S: BlockStore>(
    store: &S,
    node: &mut Node,
    layer: u32,
    key: &str,
    out: &mut BlockMap,
) -> Result<(), RepoError> {
    let i = node.position(key);
    let next = below(layer, node);
    if let Some(child) = node.gap_mut(i) {
        proof_for_left_sibling(store, load(store, child)?, next?, key, out)?;
    }
    add_block(node, out)
}

/// The nodes from the root to the node that holds the least key above
/// `key`.
fn proof_for_right_sibling<S: BlockStore>(
    store: &S,
    node: &mut Node,
    layer: u32,
    key: &str,
    out: &mut BlockMap,
) -> Result<(), RepoError> {
    let i = node.position(key);
    let next = below(layer, node);
    let slot = if node.has_key_at(i, key) {
        // The subtree right after the key.
        node.entries.get_mut(i).and_then(|e| e.right.as_mut())
    } else {
        // The subtree before the first greater key, or after the last key.
        node.gap_mut(i).as_mut()
    };
    if let Some(child) = slot {
        proof_for_right_sibling(store, load(store, child)?, next?, key, out)?;
    }
    add_block(node, out)
}

/// What a set of writes did to the stored tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeChanges {
    /// The root after the writes.
    pub root: Cid,
    /// Node blocks that the store does not hold yet.
    pub new_blocks: BlockMap,
    /// CIDs of stored node blocks that the tree no longer uses.
    pub removed: HashSet<Cid>,
}

/// A Merkle search tree over a block store.
///
/// After an operation returns an error other than
/// [`RepoError::InvalidKey`], [`RepoError::KeyExists`] and
/// [`RepoError::KeyNotFound`], the tree in memory may be incomplete: drop
/// it and load the root again.
#[derive(Debug)]
pub struct Tree<S> {
    store: S,
    root: Node,
    /// The layer of the root node; zero for the empty tree.
    layer: u32,
    removed: HashSet<Cid>,
}

impl<S: BlockStore> Tree<S> {
    /// A tree without keys.
    pub fn empty(store: S) -> Self {
        Self {
            store,
            root: Node::default(),
            layer: 0,
            removed: HashSet::new(),
        }
    }

    /// Opens the tree with the root `root`. Only the root node is read.
    pub fn load(store: S, root: Cid) -> Result<Self, RepoError> {
        let mut node = read_node(&store, &root)?;
        let layer = root_layer(&store, &mut node, 0)?;
        Ok(Self {
            store,
            root: node,
            layer,
            removed: HashSet::new(),
        })
    }

    /// The block store.
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// The layer of the root node.
    pub const fn layer(&self) -> u32 {
        self.layer
    }

    /// The record at `key`.
    pub fn get(&mut self, key: &str) -> Result<Option<Cid>, RepoError> {
        let mut node = &mut self.root;
        let mut layer = self.layer;
        loop {
            let i = node.position(key);
            if let Some(entry) = node.entries.get(i)
                && entry.key == key
            {
                return Ok(Some(entry.value));
            }
            layer = match layer.checked_sub(1) {
                Some(next) => next,
                // Layer zero has no subtrees in a valid tree.
                None => return Ok(None),
            };
            match node.gap_mut(i) {
                Some(child) => node = load(&self.store, child)?,
                None => return Ok(None),
            }
        }
    }

    /// Adds a key. Fails with [`RepoError::KeyExists`] when it is present.
    pub fn add(&mut self, key: &str, value: Cid) -> Result<(), RepoError> {
        if !is_valid_key(key) {
            return Err(RepoError::InvalidKey(key.to_owned()));
        }
        let key_layer = layer_of(key.as_bytes());
        let mut cx = Cx {
            store: &self.store,
            removed: &mut self.removed,
        };
        if key_layer <= self.layer && !self.root.is_empty() {
            return insert(&mut cx, &mut self.root, self.layer, key, key_layer, value);
        }
        // The key belongs above the root, or the tree is empty: the key
        // becomes the new root's only entry and the old tree divides
        // around it.
        let mut old = std::mem::take(&mut self.root);
        let (mut lower, mut upper) = if old.is_empty() {
            cx.touch(&mut old);
            (None, None)
        } else {
            let old_layer = self.layer;
            split(&mut cx, Child::Loaded(Box::new(old)), old_layer, key)?
        };
        // The halves sit one layer below the new root. When the key is
        // more than one layer above the old root, nodes without keys fill
        // the layers between.
        for _ in 1..key_layer.saturating_sub(self.layer) {
            lower = lower.map(|c| Child::Loaded(Box::new(Node::parent_of(c))));
            upper = upper.map(|c| Child::Loaded(Box::new(Node::parent_of(c))));
        }
        self.root = Node {
            left: lower,
            entries: vec![Entry {
                key: key.to_owned(),
                value,
                right: upper,
            }],
            ..Node::default()
        };
        self.layer = key_layer;
        Ok(())
    }

    /// Replaces the record at `key` and returns the previous one. Fails
    /// with [`RepoError::KeyNotFound`] when the key is absent.
    pub fn update(&mut self, key: &str, value: Cid) -> Result<Cid, RepoError> {
        if !is_valid_key(key) {
            return Err(RepoError::InvalidKey(key.to_owned()));
        }
        let mut cx = Cx {
            store: &self.store,
            removed: &mut self.removed,
        };
        replace(&mut cx, &mut self.root, self.layer, key, value)
    }

    /// Removes a key and returns its record. Fails with
    /// [`RepoError::KeyNotFound`] when the key is absent.
    pub fn delete(&mut self, key: &str) -> Result<Cid, RepoError> {
        if !is_valid_key(key) {
            return Err(RepoError::InvalidKey(key.to_owned()));
        }
        let mut cx = Cx {
            store: &self.store,
            removed: &mut self.removed,
        };
        let value = remove(&mut cx, &mut self.root, self.layer, key)?;
        // A root without keys and with one subtree is not canonical: the
        // subtree is the root.
        while self.root.entries.is_empty() {
            let Some(child) = self.root.left.take() else {
                break;
            };
            self.root = *into_node(&self.store, child)?;
            self.layer = self.layer.saturating_sub(1);
        }
        if self.root.is_empty() {
            self.layer = 0;
        }
        Ok(value)
    }

    /// Encodes the changed nodes and returns the root CID.
    pub fn root(&mut self) -> Result<Cid, RepoError> {
        seal(&mut self.root)
    }

    /// The root, the node blocks that are new, and the stored node blocks
    /// that are no longer used. A node that changed and changed back is in
    /// neither set.
    pub fn changes(&mut self) -> Result<TreeChanges, RepoError> {
        let root = seal(&mut self.root)?;
        let mut encoded = BlockMap::new();
        collect_new(&self.root, &mut encoded);
        let mut new_blocks = BlockMap::new();
        for (cid, bytes) in encoded.iter() {
            if !self.removed.contains(cid) {
                new_blocks.insert(*cid, bytes.to_vec());
            }
        }
        let removed = self
            .removed
            .iter()
            .filter(|cid| !encoded.contains(cid))
            .copied()
            .collect();
        Ok(TreeChanges {
            root,
            new_blocks,
            removed,
        })
    }

    /// Adds to `out` the node blocks that prove the value of `key`, or its
    /// absence, and the keys next to it on both sides.
    ///
    /// This is the covering proof that a commit carries for each of its
    /// operations, so that a consumer can check the operation against the
    /// previous root (SPEC §7.1). Record blocks are not part of it.
    pub fn covering_proof(&mut self, key: &str, out: &mut BlockMap) -> Result<(), RepoError> {
        seal(&mut self.root)?;
        let (store, layer) = (&self.store, self.layer);
        proof_for_key(store, &mut self.root, layer, key, out)?;
        proof_for_left_sibling(store, &mut self.root, layer, key, out)?;
        proof_for_right_sibling(store, &mut self.root, layer, key, out)
    }
}

/// The layer of a root node: the layer of its first key, or one above its
/// only subtree for a root without keys (which a valid tree does not have,
/// but a stored one may).
fn root_layer<S: BlockStore>(store: &S, node: &mut Node, depth: u32) -> Result<u32, RepoError> {
    if let Some(entry) = node.entries.first() {
        return Ok(layer_of(entry.key.as_bytes()));
    }
    if depth > MAX_LAYER {
        return Err(RepoError::Shape(TreeDefect::BelowLayerZero));
    }
    match &mut node.left {
        None => Ok(0),
        Some(child) => Ok(root_layer(store, load(store, child)?, depth + 1)? + 1),
    }
}
