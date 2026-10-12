//! The error type of the repository engine.

use cid::Cid;
use pds_types::CborError;

use crate::block::StoreError;

/// Why a tree is not a valid Merkle search tree.
///
/// Each variant is a defect of the tree's shape. A tree with any of them is
/// invalid as a whole (SPEC §6.1): the engine does not repair it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TreeDefect {
    /// A node is reachable by more than one path. A tree has one path to
    /// each node; a second one makes it a graph whose walk can cost far
    /// more than its blocks.
    #[error("a node is referenced more than once")]
    SharedNode,
    /// A key is at another layer than its hash gives it.
    #[error("a key is not at the layer of its hash")]
    KeyLayer,
    /// Keys are not in ascending order across the tree.
    #[error("keys are not in ascending order")]
    KeyOrder,
    /// A node below the root has neither a key nor a subtree.
    #[error("a node below the root is empty")]
    EmptyNode,
    /// The root has no key and one subtree. The canonical tree would start
    /// at that subtree.
    #[error("the root holds only a subtree")]
    LeaflessRoot,
    /// A subtree hangs below layer zero.
    #[error("a subtree below layer zero")]
    BelowLayerZero,
    /// A pointer to a node or a record is not a CIDv1 of `dag-cbor` with
    /// SHA-256.
    #[error("a pointer is not a dag-cbor SHA-256 CID")]
    PointerCid,
}

/// A failure of the caller's signer: a key that is unavailable, a KMS
/// that did not answer.
#[derive(Debug, thiserror::Error)]
#[error("signing failed: {0}")]
pub struct SignError(#[from] pub Box<dyn std::error::Error + Send + Sync>);

/// An error of the repository engine.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    /// The caller's signer failed.
    #[error(transparent)]
    Sign(#[from] SignError),
    /// The block store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A block that the tree or the commit refers to is absent.
    #[error("missing block {0}")]
    MissingBlock(Box<Cid>),
    /// A block does not have the content its CID promises.
    #[error("block {0} does not match its CID")]
    CidMismatch(Box<Cid>),
    /// A key is not `<collection>/<rkey>` in the allowed characters, or is
    /// longer than 1024 bytes.
    #[error("invalid repository key {0:?}")]
    InvalidKey(String),
    /// A create for a key that exists.
    #[error("a record exists at {0}")]
    KeyExists(String),
    /// An update or delete for a key that does not exist.
    #[error("no record at {0}")]
    KeyNotFound(String),
    /// A tree node that does not decode.
    #[error("tree node {cid}: {reason}")]
    Node {
        /// The node.
        cid: Box<Cid>,
        /// What is wrong with it.
        reason: NodeError,
    },
    /// A tree that is not a valid Merkle search tree.
    #[error("invalid tree at node {cid}: {defect}")]
    Tree {
        /// The node where the defect was found.
        cid: Box<Cid>,
        /// The defect.
        defect: TreeDefect,
    },
    /// A tree in memory that is not a valid Merkle search tree, found
    /// where no stored node can be named.
    #[error("invalid tree: {0}")]
    Shape(TreeDefect),
    /// A traversal entered more nodes than its input can hold. This bound
    /// stands even if a traversal fails to detect a shared node.
    #[error("traversal exceeded its work limit of {0} nodes")]
    WorkLimit(usize),
    /// A commit that does not decode or breaks a rule of SPEC §6.1.
    #[error("invalid commit: {0}")]
    Commit(&'static str),
    /// A commit signature that does not verify.
    #[error("commit signature does not verify")]
    Signature,
    /// A CAR file that does not parse.
    #[error("invalid CAR: {0}")]
    Car(&'static str),
    /// A record block that is not acceptable DAG-CBOR.
    #[error("record {cid}: {reason}")]
    Record {
        /// The record block.
        cid: Box<Cid>,
        /// What is wrong with it.
        reason: CborError,
    },
    /// A tree node larger than a block may be.
    #[error("a tree node would exceed the block size limit")]
    NodeTooLarge,
    /// A commit whose blocks exceed the size a commit may have (SPEC §6.3).
    #[error("commit exceeds the size limit: {0}")]
    TooLarge(&'static str),
}

/// Why a tree node block does not decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NodeError {
    /// Not DAG-CBOR in the shape of a node.
    #[error("{0}")]
    Cbor(#[from] CborError),
    /// A field other than the ones a node has, or a field out of place.
    #[error("unexpected field")]
    Field,
    /// A key prefix length that is longer than the previous key, or not
    /// the longest common prefix.
    #[error("key prefix length is not canonical")]
    Prefix,
    /// A key that is not a valid repository key.
    #[error("invalid key")]
    Key,
    /// Keys inside the node are not in ascending order.
    #[error("keys are not in ascending order")]
    Order,
}

impl RepoError {
    // CIDs are boxed so that the error stays small: every tree operation
    // returns one of these in its `Result`.
    pub(crate) fn tree(cid: Cid, defect: TreeDefect) -> Self {
        Self::Tree {
            cid: Box::new(cid),
            defect,
        }
    }

    pub(crate) fn missing(cid: Cid) -> Self {
        Self::MissingBlock(Box::new(cid))
    }

    pub(crate) fn node(cid: Cid, reason: NodeError) -> Self {
        Self::Node {
            cid: Box::new(cid),
            reason,
        }
    }

    /// Whether the error says that the input is invalid, as opposed to a
    /// failure of the store. An import answers an invalid input with 400.
    pub const fn is_invalid_input(&self) -> bool {
        !matches!(self, Self::Store(_) | Self::Sign(_))
    }
}
