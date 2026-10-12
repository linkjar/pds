//! Decision D2: both options on the same vectors.
//!
//! Every test runs one group of vectors against the engine in
//! `crates/pds-repo` and against `rsky-repo` with its sibling crates
//! `rsky-crypto` and `rsky-common`, twice: as published on crates.io
//! (0.0.6) and from the main branch of its repository (0.1.4). The in-house engine must pass. What
//! `rsky-repo` does is asserted as it was observed on 2026-10-12, so that
//! this file is the record of the comparison and fails if either side
//! changes. The findings are explained in
//! `docs/research/2026-10-12-repository-crate.md`.
//!
//!   cargo test --release -- --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// `rsky-repo` 0.0.6 with `rsky-crypto` 0.2.0 and `rsky-common` 0.1.3, as
/// published on crates.io.
mod published {
    use pds_bench::rsky as adapter;
    use rsky_common as rc;
    use rsky_crypto as rcr;
    use rsky_repo as rr;

    const VERSION: &str = "0.0.6";
    /// Proofs right of six, and proof blocks left out in all.
    const RSKY_COMMIT_PROOFS: (usize, usize) = (1, 16);
    /// Record CIDs that differ; then of thirteen commits: commit CIDs,
    /// tree roots, new blocks, removed blocks, firehose blocks that match.
    const RSKY_COMMITS: (usize, usize, usize, usize, usize, usize) = (0, 13, 13, 13, 13, 9);
    /// Whether the export of the Reference's repository is the same bytes.
    const RSKY_EXPORT_SAME_BYTES: bool = false;
    /// Whether a commit signed with P-256 verifies.
    const RSKY_VERIFIES_P256_COMMITS: bool = false;
    /// Signature fixtures right of six.
    const RSKY_SIGNATURE_FIXTURES: usize = 6;
    /// Data-model fixtures right of three; whether a float is encoded.
    const RSKY_DATA_MODEL: (usize, bool) = (1, true);
    /// Shared-node CARs refused on import, of six.
    const RSKY_REFUSES_SHARED_NODE_CARS: usize = 0;

    /// What 0.0.6 puts in a commit for a key: the nodes on the path to
    /// it. It has no covering proof.
    async fn proof_blocks(mst: &mut rr::mst::MST, key: &str, out: &mut rr::block_map::BlockMap) {
        mst.add_blocks_for_path(key.to_owned(), out).await.unwrap();
    }

    include!("shared/vectors_impl.rs");
}

/// `rsky-repo` 0.1.4 from the main branch at `7964ffa3` (2026-10-10).
mod main_branch {
    use pds_bench::rsky_main as adapter;
    use rsky_common_main as rc;
    use rsky_crypto_main as rcr;
    use rsky_repo_main as rr;

    const VERSION: &str = "0.1.4 (main)";
    const RSKY_COMMIT_PROOFS: (usize, usize) = (6, 0);
    const RSKY_COMMITS: (usize, usize, usize, usize, usize, usize) = (0, 13, 13, 13, 13, 13);
    const RSKY_EXPORT_SAME_BYTES: bool = false;
    const RSKY_VERIFIES_P256_COMMITS: bool = true;
    const RSKY_SIGNATURE_FIXTURES: usize = 6;
    const RSKY_DATA_MODEL: (usize, bool) = (3, true);
    const RSKY_REFUSES_SHARED_NODE_CARS: usize = 0;

    /// What main puts in a commit for a key: the covering proof.
    async fn proof_blocks(mst: &mut rr::mst::MST, key: &str, out: &mut rr::block_map::BlockMap) {
        out.add_map(mst.get_covering_proof(key).await.unwrap()).unwrap();
    }

    include!("shared/vectors_impl.rs");
}
