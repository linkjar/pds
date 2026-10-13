//! Tamper-evident audit log: entries, hash chain, signed checkpoints, verifier.
//!
//! Specified in `docs/SPEC.md` §20. An entry names an account or operator
//! action and the hash of the entry before it. A checkpoint signs the
//! Merkle root of a range of entries. [`verify`] recomputes all of it from
//! an export, and shares with the writer only the entry format of
//! [`entry`].

pub mod checkpoint;
pub mod entry;
pub mod merkle;
pub mod verify;

pub use checkpoint::{Checkpoint, Signer};
pub use entry::{Draft, Entry};
