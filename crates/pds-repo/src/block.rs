//! Blocks and where they are kept.
//!
//! A block is a byte string addressed by its CID. [`BlockStore`] is the read
//! side that the tree and the exporter need; the SQLite `repo_block` table
//! implements it in `pds-storage`. The trait is synchronous: SQLite runs on
//! a blocking pool and each actor has one writer (SPEC §3.2).
//!
//! A [`BlockMap`] holds its blocks as [`Bytes`]: the maps of one commit
//! share their blocks, and a clone of a map or of a block is a reference
//! count, not a copy.

use std::collections::HashMap;

use bytes::Bytes;
use cid::Cid;
use pds_types::content::cid_for_cbor;

/// A failure of the store itself, as opposed to a block that is absent.
#[derive(Debug, thiserror::Error)]
#[error("block store: {0}")]
pub struct StoreError(#[from] pub Box<dyn std::error::Error + Send + Sync>);

/// Read access to blocks by CID.
pub trait BlockStore {
    /// The bytes of a block, or `None` when the store does not hold it.
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StoreError>;

    /// Whether the store holds a block.
    fn has(&self, cid: &Cid) -> Result<bool, StoreError> {
        Ok(self.get(cid)?.is_some())
    }
}

impl<T: BlockStore + ?Sized> BlockStore for &T {
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StoreError> {
        (**self).get(cid)
    }
    fn has(&self, cid: &Cid) -> Result<bool, StoreError> {
        (**self).has(cid)
    }
}

/// Blocks by CID, in the order they were first added.
///
/// The order is kept because a CAR file lists blocks in it, and the
/// Reference writes its block maps in insertion order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockMap {
    order: Vec<Cid>,
    blocks: HashMap<Cid, Bytes>,
}

impl BlockMap {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a block. A CID that is present keeps its place.
    pub fn insert(&mut self, cid: Cid, bytes: impl Into<Bytes>) {
        if self.blocks.insert(cid, bytes.into()).is_none() {
            self.order.push(cid);
        }
    }

    /// Adds a DAG-CBOR block and returns its CID.
    pub fn insert_cbor(&mut self, bytes: impl Into<Bytes>) -> Cid {
        let bytes = bytes.into();
        let cid = cid_for_cbor(&bytes);
        self.insert(cid, bytes);
        cid
    }

    /// Adds every block of `other` that is not present, in its order. The
    /// blocks are shared, not copied.
    pub fn extend(&mut self, other: &Self) {
        for cid in &other.order {
            if !self.blocks.contains_key(cid)
                && let Some(bytes) = other.blocks.get(cid)
            {
                self.insert(*cid, bytes.clone());
            }
        }
    }

    /// The bytes of a block.
    pub fn get(&self, cid: &Cid) -> Option<&[u8]> {
        self.blocks.get(cid).map(|bytes| &bytes[..])
    }

    /// A block as shared bytes: the clone counts a reference.
    pub fn get_shared(&self, cid: &Cid) -> Option<Bytes> {
        self.blocks.get(cid).cloned()
    }

    /// Whether a block is present.
    pub fn contains(&self, cid: &Cid) -> bool {
        self.blocks.contains_key(cid)
    }

    /// The number of blocks.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The sum of the block lengths, without CIDs.
    pub fn byte_len(&self) -> usize {
        self.blocks.values().map(Bytes::len).sum()
    }

    /// The blocks in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&Cid, &[u8])> {
        self.order
            .iter()
            .filter_map(|cid| self.blocks.get(cid).map(|b| (cid, &b[..])))
    }

    /// The CIDs in insertion order.
    pub fn cids(&self) -> impl Iterator<Item = &Cid> {
        self.order.iter()
    }
}

impl BlockStore for BlockMap {
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.blocks.get(cid).map(|bytes| bytes.to_vec()))
    }
    fn has(&self, cid: &Cid) -> Result<bool, StoreError> {
        Ok(self.blocks.contains_key(cid))
    }
}

/// A block store in memory, for tests, benchmarks and staging.
#[derive(Debug, Clone, Default)]
pub struct MemoryBlockStore {
    blocks: HashMap<Cid, Bytes>,
}

impl MemoryBlockStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a block.
    pub fn put(&mut self, cid: Cid, bytes: impl Into<Bytes>) {
        self.blocks.insert(cid, bytes.into());
    }

    /// Adds every block of a map. The blocks are shared, not copied.
    pub fn put_all(&mut self, blocks: &BlockMap) {
        for cid in blocks.cids() {
            if let Some(bytes) = blocks.get_shared(cid) {
                self.blocks.insert(*cid, bytes);
            }
        }
    }

    /// Removes a block.
    pub fn remove(&mut self, cid: &Cid) {
        self.blocks.remove(cid);
    }

    /// The number of blocks.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

impl BlockStore for MemoryBlockStore {
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.blocks.get(cid).map(|bytes| bytes.to_vec()))
    }
    fn has(&self, cid: &Cid) -> Result<bool, StoreError> {
        Ok(self.blocks.contains_key(cid))
    }
}

/// Two stores read as one: `staged` first, then `base`. An import reads the
/// uploaded blocks over the blocks the account already has.
#[derive(Debug, Clone, Copy)]
pub struct Overlay<A, B> {
    /// The blocks read first.
    pub staged: A,
    /// The blocks read when `staged` lacks one.
    pub base: B,
}

impl<A: BlockStore, B: BlockStore> BlockStore for Overlay<A, B> {
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StoreError> {
        match self.staged.get(cid)? {
            Some(bytes) => Ok(Some(bytes)),
            None => self.base.get(cid),
        }
    }
    fn has(&self, cid: &Cid) -> Result<bool, StoreError> {
        Ok(self.staged.has(cid)? || self.base.has(cid)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_map_keeps_first_insertion_order() {
        let mut map = BlockMap::new();
        let a = map.insert_cbor(vec![0x01]);
        let b = map.insert_cbor(vec![0x02]);
        map.insert(a, vec![0x01]);
        assert_eq!(map.cids().copied().collect::<Vec<_>>(), vec![a, b]);
        assert_eq!(map.len(), 2);
        assert_eq!(map.byte_len(), 2);
        let mut other = BlockMap::new();
        let c = other.insert_cbor(vec![0x03]);
        other.insert_cbor(vec![0x02]);
        map.extend(&other);
        assert_eq!(map.cids().copied().collect::<Vec<_>>(), vec![a, b, c]);
    }

    #[test]
    fn overlay_reads_staged_first() {
        let mut staged = MemoryBlockStore::new();
        let mut base = MemoryBlockStore::new();
        let cid = cid_for_cbor(&[0x01]);
        let only_base = cid_for_cbor(&[0x02]);
        staged.put(cid, vec![0x01]);
        base.put(cid, vec![0xff]);
        base.put(only_base, vec![0x02]);
        let both = Overlay {
            staged: &staged,
            base: &base,
        };
        assert_eq!(both.get(&cid).unwrap(), Some(vec![0x01]));
        assert_eq!(both.get(&only_base).unwrap(), Some(vec![0x02]));
        assert!(!both.has(&cid_for_cbor(&[0x03])).unwrap());
    }
}
