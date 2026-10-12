//! The Merkle search tree: the key-value index of a repository.
//!
//! [`node`] is the wire form, [`tree`] the structure that writes change,
//! and [`walk`] the traversals of a whole tree with their guard.

pub mod node;
pub mod tree;
pub mod walk;

pub use node::{MAX_KEY_LEN, MAX_LAYER, WireEntry, WireNode, is_valid_key, layer_of};
pub use tree::{Tree, TreeChanges};
pub use walk::{Guard, TreeContents, export_blocks, read_tree};
