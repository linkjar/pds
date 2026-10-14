//! Repository engine: the Merkle search tree, commits, CAR files and the
//! verification of repositories from elsewhere (`docs/SPEC.md` §6).
//!
//! The engine is written in this crate (the decision of 2026-10-12 on the
//! repository engine, recorded in
//! `docs/research/2026-10-12-repository-crate.md`). It is synchronous and
//! does no I/O of its own: blocks come from a [`BlockStore`], signatures
//! from the caller's key, and the revision from the caller's clock, so the
//! same code runs over SQLite, over an uploaded CAR, and under simulation.
//!
//! - [`mst`]: the tree. [`mst::Tree`] applies writes along one path and
//!   reports the blocks that changed. [`mst::read_tree`] reads a whole tree
//!   and rejects one that is not valid.
//! - [`commit`]: the signed commit block.
//! - [`car`]: CAR version 1, read and written.
//! - [`repo`]: commits from writes, export, and the checks on an imported
//!   CAR.
//!
//! A tree that reaches one node by two paths is invalid everywhere it is
//! read: import, comparison and export. See [`mst::walk`].

pub mod block;
pub mod car;
pub mod commit;
pub mod error;
pub mod mst;
pub mod repo;

pub use block::{BlockMap, BlockStore, MemoryBlockStore, Overlay, StoreError};
pub use car::{Car, CarWriter, read_car, write_car};
pub use commit::{Commit, UnsignedCommit};
pub use error::{NodeError, RepoError, SignError, TreeDefect};
pub use repo::{
    CommitData, Expect, ExportError, RecordChange, Repo, VerifiedDiff, VerifiedRepo, VerifyFn,
    Write, verify_diff_car, verify_repo_car,
};
