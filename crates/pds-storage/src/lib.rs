//! Storage over the Reference's data directory (`docs/SPEC.md` §8).
//!
//! Schema version 1 is the Reference schema, so this crate reads the SQLite
//! files a Reference server wrote: `account.sqlite`, `sequencer.sqlite`,
//! `did_cache.sqlite` and one `store.sqlite` per actor. Every call is
//! synchronous; the server runs them on its blocking pool (§3.2).
//!
//! Unit 3 reads. The write half of each seam arrives with the write path
//! (`docs/plan.md`, unit 4).
//!
//! A read changes no row, but it can write files. The Reference's databases
//! are in WAL journal mode (§8.3), and a read connection to such a database
//! creates its `-shm` and `-wal` files when they are absent. The `layout`
//! module states what a read connection needs.

pub mod account;
pub mod actor;
pub mod actor_txn;
pub mod did_cache;
pub mod error;
pub mod fail;
pub mod io;
pub mod layout;
pub mod oauth;
pub mod ops;
pub mod schema;
pub mod schema_shared;
pub mod sequencer;

pub use account::{
    Account, AccountDb, Actor, AppPassword, Availability, InviteCode, Invites, NewAccount,
    RefreshToken, Refused, RepoListing,
};
pub use actor::{
    ActorDb, BLOCK_BATCH, BlobMeta, BlockRow, FoundBlocks, RecentRecord, RecordRow, RepoRoot,
};
pub use actor_txn::{ActorTxn, Backlink, MissingBlob, PendingUpload, StoredBlob};
pub use did_cache::{CachedDoc, DidCacheDb};
pub use error::StorageError;
pub use layout::DataLayout;
pub use ops::{OpsDb, PendingCommit};
pub use sequencer::{SeqGap, SeqInsert, SeqRow, SequencerDb};
