//! A repository: a signed commit over a tree of records.
//!
//! [`Repo::format_commit`] turns a batch of writes into the blocks of the
//! next commit without writing anything; the caller stores them in the
//! transaction that also sequences the event (SPEC §6.3). [`Repo::export`]
//! streams the repository as a CAR. [`verify_repo_car`] and
//! [`verify_diff_car`] read an untrusted CAR and say what it holds.

use std::collections::{BTreeMap, HashSet};
use std::io;

use bytes::Bytes;
use cid::Cid;
use pds_types::content::cid_for_cbor;
use pds_types::{Did, Tid};

use crate::block::{BlockMap, BlockStore, Overlay};
use crate::car::{Car, CarWriter, read_car};
use crate::commit::{Commit, UnsignedCommit};
use crate::error::{RepoError, SignError};
use crate::mst::{Guard, Tree, TreeContents, export_blocks, read_tree};

/// The most operations in one commit (SPEC §6.3).
pub const MAX_WRITES: usize = 200;
/// The most bytes of blocks in one commit (SPEC §6.3).
pub const MAX_COMMIT_BLOCKS_LEN: usize = 2_000_000;
/// The most bytes in one record block (SPEC §6.3).
pub const MAX_RECORD_BLOCK_LEN: usize = 1_000_000;

/// One write of a batch. A record is the DAG-CBOR the caller validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write<'a> {
    /// Adds a record at a key that has none.
    Create {
        /// `<collection>/<rkey>`.
        key: &'a str,
        /// The record block.
        record: &'a [u8],
    },
    /// Replaces the record at a key.
    Update {
        /// `<collection>/<rkey>`.
        key: &'a str,
        /// The record block.
        record: &'a [u8],
    },
    /// Removes the record at a key.
    Delete {
        /// `<collection>/<rkey>`.
        key: &'a str,
    },
}

impl Write<'_> {
    /// The key the write is about.
    pub const fn key(&self) -> &str {
        match self {
            Self::Create { key, .. } | Self::Update { key, .. } | Self::Delete { key } => key,
        }
    }
}

/// What a write did to one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordChange {
    /// `<collection>/<rkey>`.
    pub key: String,
    /// The record before, when there was one.
    pub prev: Option<Cid>,
    /// The record after, when there is one.
    pub cid: Option<Cid>,
}

/// The blocks and facts of one commit, ready to store and to sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitData {
    /// The CID of the new commit.
    pub cid: Cid,
    /// The new revision.
    pub rev: Tid,
    /// The revision before, when there was a commit.
    pub since: Option<Tid>,
    /// The commit before, when there was one.
    pub prev: Option<Cid>,
    /// The root of the tree after the commit.
    pub data: Cid,
    /// The root of the tree before the commit, when there was one. The
    /// firehose sends it as `prevData`.
    pub prev_data: Option<Cid>,
    /// Blocks to store: new tree nodes, new records and the commit.
    pub new_blocks: BlockMap,
    /// Blocks of the firehose event: the proof for every operation, the new
    /// records and the commit.
    pub relevant_blocks: BlockMap,
    /// Stored blocks that the repository no longer uses. A record that
    /// another key still holds is the caller's to keep.
    pub removed_cids: HashSet<Cid>,
    /// The net change of every key the batch touched, in key order.
    pub changes: Vec<RecordChange>,
}

/// The record sets of a change, by the Reference's rules: a record that is
/// both removed and added again is in neither set.
#[derive(Debug, Default)]
struct LeafSets {
    new: Vec<Cid>,
    new_set: HashSet<Cid>,
    removed: HashSet<Cid>,
}

impl LeafSets {
    fn added(&mut self, cid: Cid) {
        if !self.removed.remove(&cid) && self.new_set.insert(cid) {
            self.new.push(cid);
        }
    }

    fn deleted(&mut self, cid: Cid) {
        if self.new_set.remove(&cid) {
            self.new.retain(|c| *c != cid);
        } else {
            self.removed.insert(cid);
        }
    }

    fn apply(&mut self, change: &RecordChange) {
        match (change.prev, change.cid) {
            (None, Some(cid)) => self.added(cid),
            (Some(prev), None) => self.deleted(prev),
            (Some(prev), Some(cid)) if prev != cid => {
                self.removed.insert(prev);
                if self.new_set.insert(cid) {
                    self.new.push(cid);
                }
            }
            _ => {}
        }
    }
}

/// Applies writes to a tree and records the net change per key.
fn apply_writes<S: BlockStore>(
    tree: &mut Tree<S>,
    writes: &[Write<'_>],
    records: &mut BlockMap,
) -> Result<Vec<RecordChange>, RepoError> {
    if writes.len() > MAX_WRITES {
        return Err(RepoError::TooLarge("more than 200 writes"));
    }
    // Key to (record before the batch, record now).
    let mut touched: BTreeMap<String, (Option<Cid>, Option<Cid>)> = BTreeMap::new();
    let mut note = |key: &str, before: Option<Cid>, after: Option<Cid>| {
        touched
            .entry(key.to_owned())
            .and_modify(|state| state.1 = after)
            .or_insert((before, after));
    };
    for write in writes {
        match *write {
            Write::Create { key, record } => {
                if record.len() > MAX_RECORD_BLOCK_LEN {
                    return Err(RepoError::TooLarge("a record block over 1,000,000 bytes"));
                }
                let cid = records.insert_cbor(Bytes::copy_from_slice(record));
                tree.add(key, cid)?;
                note(key, None, Some(cid));
            }
            Write::Update { key, record } => {
                if record.len() > MAX_RECORD_BLOCK_LEN {
                    return Err(RepoError::TooLarge("a record block over 1,000,000 bytes"));
                }
                let cid = records.insert_cbor(Bytes::copy_from_slice(record));
                let prev = tree.update(key, cid)?;
                note(key, Some(prev), Some(cid));
            }
            Write::Delete { key } => {
                let prev = tree.delete(key)?;
                note(key, Some(prev), None);
            }
        }
    }
    Ok(touched
        .into_iter()
        .map(|(key, (prev, cid))| RecordChange { key, prev, cid })
        .collect())
}

/// Everything about the commit before, for [`build_commit`].
struct Previous {
    cid: Cid,
    rev: Tid,
    data: Cid,
}

fn build_commit<S: BlockStore>(
    mut tree: Tree<S>,
    did: &Did,
    rev: Tid,
    previous: Option<&Previous>,
    writes: &[Write<'_>],
    sign: impl FnOnce(&[u8]) -> Result<Vec<u8>, SignError>,
) -> Result<CommitData, RepoError> {
    let mut records = BlockMap::new();
    let changes = apply_writes(&mut tree, writes, &mut records)?;
    let tree_changes = tree.changes()?;

    let mut leaves = LeafSets::default();
    for change in &changes {
        leaves.apply(change);
    }
    let mut new_blocks = tree_changes.new_blocks;
    let mut removed_cids = tree_changes.removed;
    removed_cids.extend(leaves.removed.iter().copied());

    let mut relevant_blocks = BlockMap::new();
    for write in writes {
        tree.covering_proof(write.key(), &mut relevant_blocks)?;
    }
    for cid in &leaves.new {
        let bytes = records
            .get_shared(cid)
            .ok_or_else(|| RepoError::missing(*cid))?;
        new_blocks.insert(*cid, bytes.clone());
        relevant_blocks.insert(*cid, bytes);
    }

    let unsigned = UnsignedCommit::new(did.clone(), rev.clone(), tree_changes.root);
    let commit = unsigned.sign(sign)?;
    let commit_bytes = Bytes::from(commit.to_bytes());
    let cid = cid_for_cbor(&commit_bytes);
    if previous.is_none_or(|p| p.cid != cid) {
        new_blocks.insert(cid, commit_bytes.clone());
        relevant_blocks.insert(cid, commit_bytes);
        if let Some(p) = previous {
            removed_cids.insert(p.cid);
        }
    }
    if previous.is_none() {
        // The first commit of a repository carries all of it. The
        // Reference lists the records first, then the tree, then the
        // commit, and sends the same blocks on the firehose.
        let mut all = BlockMap::new();
        for record in records.cids() {
            if let Some(bytes) = new_blocks.get_shared(record) {
                all.insert(*record, bytes);
            }
        }
        all.extend(&new_blocks);
        relevant_blocks = all.clone();
        new_blocks = all;
    }
    if relevant_blocks.byte_len() > MAX_COMMIT_BLOCKS_LEN {
        return Err(RepoError::TooLarge("more than 2,000,000 bytes of blocks"));
    }
    Ok(CommitData {
        cid,
        rev,
        since: previous.map(|p| p.rev.clone()),
        prev: previous.map(|p| p.cid),
        data: tree_changes.root,
        prev_data: previous.map(|p| p.data),
        new_blocks,
        relevant_blocks,
        removed_cids,
        changes,
    })
}

/// A repository at one commit.
#[derive(Debug)]
pub struct Repo<S> {
    store: S,
    cid: Cid,
    commit: Commit,
    /// The commit block, as stored.
    bytes: Bytes,
}

impl<S: BlockStore> Repo<S> {
    /// Opens the repository at the commit `cid`. Reads the commit block
    /// only.
    pub fn load(store: S, cid: Cid) -> Result<Self, RepoError> {
        let bytes = store.get(&cid)?.ok_or_else(|| RepoError::missing(cid))?;
        let commit = Commit::decode(&bytes)?;
        Ok(Self {
            store,
            cid,
            commit,
            bytes: bytes.into(),
        })
    }

    /// The commit's CID.
    pub const fn cid(&self) -> &Cid {
        &self.cid
    }

    /// The commit.
    pub const fn commit(&self) -> &Commit {
        &self.commit
    }

    /// The record at `key`: its CID and its block.
    pub fn get_record(&self, key: &str) -> Result<Option<(Cid, Vec<u8>)>, RepoError> {
        let mut tree = Tree::load(&self.store, self.commit.data)?;
        let Some(cid) = tree.get(key)? else {
            return Ok(None);
        };
        let bytes = self
            .store
            .get(&cid)?
            .ok_or_else(|| RepoError::missing(cid))?;
        Ok(Some((cid, bytes)))
    }

    /// The blocks that prove the record at `key`, or its absence: the
    /// commit, the record when it is present, and the tree nodes on the path
    /// to the key from the root down.
    ///
    /// The order is the one the Reference writes into the CAR of
    /// `com.atproto.sync.getRecord`.
    pub fn record_proof(&self, key: &str) -> Result<BlockMap, RepoError> {
        let mut tree = Tree::load(&self.store, self.commit.data)?;
        let path = tree.path_to(key)?;
        let mut blocks = BlockMap::new();
        blocks.insert(self.cid, self.bytes.clone());
        if let Some(cid) = path.value {
            let bytes = self
                .store
                .get(&cid)?
                .ok_or_else(|| RepoError::missing(cid))?;
            blocks.insert(cid, bytes);
        }
        for (cid, bytes) in path.nodes {
            blocks.insert(cid, bytes);
        }
        Ok(blocks)
    }

    /// Builds the first commit of a repository from `writes`. `sign`
    /// receives the bytes to sign and returns the signature.
    pub fn format_init_commit(
        store: S,
        did: &Did,
        rev: Tid,
        writes: &[Write<'_>],
        sign: impl FnOnce(&[u8]) -> Result<Vec<u8>, SignError>,
    ) -> Result<CommitData, RepoError> {
        build_commit(Tree::empty(store), did, rev, None, writes, sign)
    }

    /// Builds the next commit from `writes`. Nothing is stored.
    ///
    /// `rev` must be greater than the current revision (SPEC §6.2); the
    /// caller takes it from its clock. `sign` receives the bytes to sign
    /// and returns the signature. A batch that changes nothing still gives
    /// a commit with a new revision; the caller decides whether to keep it
    /// (SPEC §6.3).
    pub fn format_commit(
        &self,
        rev: Tid,
        writes: &[Write<'_>],
        sign: impl FnOnce(&[u8]) -> Result<Vec<u8>, SignError>,
    ) -> Result<CommitData, RepoError> {
        if rev <= self.commit.rev {
            return Err(RepoError::Commit(
                "rev does not follow the current revision",
            ));
        }
        let tree = Tree::load(&self.store, self.commit.data)?;
        let previous = Previous {
            cid: self.cid,
            rev: self.commit.rev.clone(),
            data: self.commit.data,
        };
        build_commit(tree, &self.commit.did, rev, Some(&previous), writes, sign)
    }

    /// Writes the repository as a CAR: the commit, the tree nodes layer by
    /// layer, then the records, in the Reference's order.
    ///
    /// `max_nodes` bounds the walk; pass the number of blocks the
    /// repository has in the store.
    pub fn export<W: io::Write>(&self, out: W, max_nodes: usize) -> Result<W, ExportError> {
        let commit = self
            .store
            .get(&self.cid)
            .map_err(RepoError::from)?
            .ok_or_else(|| RepoError::missing(self.cid))?;
        let mut writer = CarWriter::new(out, Some(&self.cid))?;
        writer.block(&self.cid, &commit)?;
        let mut guard = Guard::new(max_nodes);
        let mut failed: Option<io::Error> = None;
        let walked = export_blocks(&self.store, &self.commit.data, &mut guard, |cid, bytes| {
            writer.block(cid, bytes).map_err(|e| {
                failed = Some(e);
                RepoError::Car("the output failed")
            })
        });
        match (walked, failed) {
            (_, Some(e)) => Err(ExportError::Io(e)),
            (Err(e), None) => Err(e.into()),
            (Ok(()), None) => Ok(writer.finish()),
        }
    }
}

/// Why an export stopped.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// The repository could not be read.
    #[error(transparent)]
    Repo(Box<RepoError>),
    /// The output could not be written.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl From<RepoError> for ExportError {
    fn from(e: RepoError) -> Self {
        Self::Repo(Box::new(e))
    }
}

/// A repository read from an untrusted CAR and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRepo {
    /// The commit's CID.
    pub cid: Cid,
    /// The commit.
    pub commit: Commit,
    /// Every key with its record, in key order.
    pub records: Vec<(String, Cid)>,
    /// The tree nodes, parents first.
    pub nodes: Vec<Cid>,
    /// The blocks of the CAR.
    pub blocks: BlockMap,
}

/// A signature check: it receives the signed bytes and the signature.
pub type VerifyFn<'a> = &'a dyn Fn(&[u8], &[u8]) -> bool;

/// What the caller requires of the commit in a CAR.
pub struct Expect<'a> {
    /// The account the commit must name, when given.
    pub did: Option<&'a Did>,
    /// The signature check, when given.
    pub verify: Option<VerifyFn<'a>>,
}

impl std::fmt::Debug for Expect<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Expect")
            .field("did", &self.did)
            .field("verify", &self.verify.is_some())
            .finish()
    }
}

fn check_commit(car: &Car, expect: &Expect<'_>) -> Result<(Cid, Commit), RepoError> {
    let cid = car.root()?;
    let bytes = car
        .blocks
        .get(&cid)
        .ok_or_else(|| RepoError::missing(cid))?;
    let commit = Commit::decode(bytes)?;
    if expect.did.is_some_and(|did| *did != commit.did) {
        return Err(RepoError::Commit("the commit is for another account"));
    }
    if let Some(verify) = expect.verify {
        commit.verify(verify)?;
    }
    Ok((cid, commit))
}

/// Reads a whole repository from a CAR and checks it.
///
/// The checks: one root, a version 3 commit for the expected account with
/// a valid signature, and a tree that [`read_tree`] accepts with a budget
/// of the CAR's block count. With `require_records` every record block
/// must be in the CAR. Blocks that the commit does not reach are ignored.
pub fn verify_repo_car(
    bytes: &[u8],
    expect: &Expect<'_>,
    require_records: bool,
) -> Result<VerifiedRepo, RepoError> {
    let car = read_car(bytes)?;
    let (cid, commit) = check_commit(&car, expect)?;
    let mut guard = Guard::new(car.blocks.len());
    let TreeContents { leaves, nodes } = read_tree(&car.blocks, &commit.data, &mut guard)?;
    if require_records
        && let Some((_, missing)) = leaves.iter().find(|(_, cid)| !car.blocks.contains(cid))
    {
        return Err(RepoError::missing(*missing));
    }
    Ok(VerifiedRepo {
        cid,
        commit,
        records: leaves,
        nodes,
        blocks: car.blocks,
    })
}

/// The keys whose record differs between two trees, in key order. Both
/// lists are in key order.
fn record_changes(before: &[(String, Cid)], after: &[(String, Cid)]) -> Vec<RecordChange> {
    use std::cmp::Ordering;
    let deleted = |(key, prev): &(String, Cid)| RecordChange {
        key: key.clone(),
        prev: Some(*prev),
        cid: None,
    };
    let added = |(key, cid): &(String, Cid)| RecordChange {
        key: key.clone(),
        prev: None,
        cid: Some(*cid),
    };
    let mut changes = Vec::new();
    let (mut old, mut new) = (before.iter().peekable(), after.iter().peekable());
    loop {
        let order = match (old.peek(), new.peek()) {
            (None, None) => break,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some((old_key, _)), Some((new_key, _))) => old_key.cmp(new_key),
        };
        match order {
            Ordering::Less => changes.extend(old.next().map(deleted)),
            Ordering::Greater => changes.extend(new.next().map(added)),
            Ordering::Equal => {
                if let (Some((key, prev)), Some((_, cid))) = (old.next(), new.next())
                    && prev != cid
                {
                    changes.push(RecordChange {
                        key: key.clone(),
                        prev: Some(*prev),
                        cid: Some(*cid),
                    });
                }
            }
        }
    }
    changes
}

/// A commit from an untrusted CAR, compared with the repository it
/// replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedDiff {
    /// The new commit's CID.
    pub cid: Cid,
    /// The new commit.
    pub commit: Commit,
    /// The keys that differ, in key order.
    pub changes: Vec<RecordChange>,
    /// Blocks to store: tree nodes and records the repository lacks, and
    /// the commit.
    pub new_blocks: BlockMap,
    /// Stored blocks the new tree no longer uses.
    pub removed_cids: HashSet<Cid>,
}

/// Reads a commit from a CAR on top of the repository `current` and
/// reports the difference.
///
/// The CAR may hold only the blocks that are new; the rest are read from
/// `current`'s store. Both trees are read in full and the new one is
/// checked like an import, so the cost is linear in the size of the
/// repository, not of the difference. `stored_blocks` is the number of
/// blocks the repository has in the store; with the CAR's block count it
/// bounds the walk.
pub fn verify_diff_car<S: BlockStore>(
    current: &Repo<S>,
    bytes: &[u8],
    expect: &Expect<'_>,
    stored_blocks: usize,
) -> Result<VerifiedDiff, RepoError> {
    let car = read_car(bytes)?;
    let (cid, commit) = check_commit(&car, expect)?;
    let budget = car.blocks.len().saturating_add(stored_blocks);
    let before = read_tree(
        &current.store,
        &current.commit.data,
        &mut Guard::new(budget),
    )?;
    let both = Overlay {
        staged: &car.blocks,
        base: &current.store,
    };
    let after = read_tree(&both, &commit.data, &mut Guard::new(budget))?;

    let changes = record_changes(&before.leaves, &after.leaves);
    let mut leaves = LeafSets::default();
    for change in &changes {
        leaves.apply(change);
    }

    let old_nodes: HashSet<Cid> = before.nodes.iter().copied().collect();
    let new_nodes: HashSet<Cid> = after.nodes.iter().copied().collect();
    let kept_records: HashSet<Cid> = after.leaves.iter().map(|(_, cid)| *cid).collect();
    let mut new_blocks = BlockMap::new();
    for node in after.nodes.iter().filter(|n| !old_nodes.contains(n)) {
        let block = both.get(node)?.ok_or_else(|| RepoError::missing(*node))?;
        new_blocks.insert(*node, block);
    }
    for record in &leaves.new {
        let block = car
            .blocks
            .get_shared(record)
            .ok_or_else(|| RepoError::missing(*record))?;
        new_blocks.insert(*record, block);
    }
    let mut removed_cids: HashSet<Cid> = before
        .nodes
        .iter()
        .filter(|n| !new_nodes.contains(n))
        .copied()
        .collect();
    removed_cids.extend(
        leaves
            .removed
            .iter()
            .filter(|cid| !kept_records.contains(cid))
            .copied(),
    );
    if cid != current.cid {
        new_blocks.insert(cid, commit.to_bytes());
        removed_cids.insert(current.cid);
    }
    Ok(VerifiedDiff {
        cid,
        commit,
        changes,
        new_blocks,
        removed_cids,
    })
}
