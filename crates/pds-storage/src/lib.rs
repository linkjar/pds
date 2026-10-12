//! Storage over the Reference's data directory (`docs/SPEC.md` §8).
//!
//! Schema version 1 is the Reference schema, so this crate reads the SQLite
//! files a Reference server wrote: `account.sqlite`, `sequencer.sqlite`,
//! `did_cache.sqlite` and one `store.sqlite` per actor. Every call is
//! synchronous; the server runs them on its blocking pool (§3.2).
//!
//! This unit reads. The write half of each seam arrives with the write path
//! (`docs/plan.md`, unit 4).

pub mod account;
pub mod actor;
pub mod did_cache;
pub mod error;
pub mod layout;
pub mod sequencer;

pub use account::{AccountDb, Actor, Availability, RepoListing};
pub use actor::{ActorDb, BlobMeta, BlockRow, FoundBlocks, RecordRow, RepoRoot};
pub use did_cache::{CachedDoc, DidCacheDb};
pub use error::StorageError;
pub use layout::DataLayout;
pub use sequencer::{SeqGap, SequencerDb};
