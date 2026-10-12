//! Evidence for decision D2: the repository engine of `crates/pds-repo`
//! and the `rsky-repo` crate behind one interface, driven by one workload.
//!
//! The workload is the one the parity harness ran against the Reference
//! for SPEC §16.1, S1 to S3 (`parity/scenarios/perf/01-repository`): the
//! same collection, record keys and record bodies, written in the same
//! order. See `README.md` for what a sample includes and what it leaves
//! out.

pub mod alloc;
pub mod inhouse;

/// The `rsky-repo` crate as published on crates.io, version 0.0.6, through
/// its public API.
///
/// `rsky-repo` is asynchronous and its storage trait is behind a tokio
/// `RwLock`. Each operation runs to completion on a current-thread
/// runtime, which is the cheapest way to drive it.
pub mod rsky {
    use ::rsky_repo as rr;
    /// The engine's name in reports.
    pub const NAME: &str = "rsky-repo 0.0.6 (crates.io)";
    include!("rsky_impl.rs");
}

/// `rsky-repo` from the main branch of its repository at `7964ffa3`
/// (2026-10-10), version 0.1.4, not published.
pub mod rsky_main {
    use ::rsky_repo_main as rr;
    /// The engine's name in reports.
    pub const NAME: &str = "rsky-repo 0.1.4 (main at 7964ffa3)";
    include!("rsky_impl.rs");
}

use std::time::Duration;

/// The collection of the harness's performance scenario.
pub const NOTE: &str = "com.example.parity.note";
/// A `did:plc` of the usual length, so that commit blocks have the size
/// they have on the Reference.
pub const DID: &str = "did:plc:ewvi7nxzyoun6zhxrhs64oiz";
/// A fixed secp256k1 key, the first of upstream's `w3c_didkey_K256` vectors.
pub const KEY_HEX: &str = "9085d2bef69286a6cbb51623c8fa258629945cd55ca705cc4e66700396894e0c";

/// The record key of record `n`, as the harness writes it.
pub fn rkey(n: usize) -> String {
    format!("r{n:07}")
}

/// The body of record `n` as JSON, as the harness sends it.
pub fn record_json(n: usize) -> serde_json::Value {
    serde_json::json!({
        "$type": NOTE,
        "n": n,
        "text": format!("record {n} of a repository that grows"),
    })
}

/// One timed commit.
#[derive(Debug, Clone, Copy)]
pub struct WriteSample {
    /// Wall time of the commit.
    pub elapsed: Duration,
    /// Length of the CAR of the commit's firehose blocks.
    pub event_car_len: usize,
    /// Blocks the commit adds to the store.
    pub new_blocks: usize,
    /// Blocks the commit removes from the store.
    pub removed_blocks: usize,
}

/// One export.
#[derive(Debug, Clone, Copy)]
pub struct ExportSample {
    /// Wall time.
    pub elapsed: Duration,
    /// Length of the CAR.
    pub car_len: usize,
    /// Peak heap in use during the export, above the heap in use before it.
    pub peak_heap: usize,
}

/// A repository engine under test: one repository in a memory block store.
pub trait Engine {
    /// The engine's name in reports.
    fn name(&self) -> &'static str;
    /// Records written so far.
    fn records(&self) -> usize;
    /// Writes the next `count` records as one commit and stores it.
    fn write(&mut self, count: usize) -> WriteSample;
    /// Exports the repository as a CAR into a sink that counts bytes.
    fn export(&mut self) -> ExportSample;
    /// The root of the tree, for comparison between engines.
    fn tree_root(&mut self) -> String;
    /// Blocks in the store.
    fn stored_blocks(&mut self) -> usize;
}

/// The value at percentile `p` of `values`, by the nearest-rank method the
/// harness uses.
pub fn percentile(values: &[Duration], p: f64) -> Duration {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// A writer that counts and discards, as a socket would take the bytes.
#[derive(Debug, Default)]
pub struct CountingSink(pub usize);

impl std::io::Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
