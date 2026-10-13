//! The network runs of the simulation (`docs/SPEC.md` §19.2).
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use pds_sim::net::{NetOptions, run_network_seed};

#[test]
fn the_first_network_seeds_pass_every_oracle() {
    for seed in 1..=12 {
        let outcome = run_network_seed(seed, NetOptions::default())
            .unwrap_or_else(|failure| panic!("seed {seed}: {failure}"));
        assert!(outcome.commits > 0, "seed {seed}");
        assert!(outcome.events > i64::from(outcome.commits), "seed {seed}");
        assert!(outcome.connections >= 1, "seed {seed}");
    }
}

/// With a buffer of two events the server closes a relay that fell behind,
/// and the relay still ends with the whole log.
#[test]
fn a_small_subscriber_buffer_still_converges() {
    let options = NetOptions {
        steps: 300,
        buffer: 2,
        ..NetOptions::default()
    };
    let mut faults = 0;
    for seed in 100..=105 {
        let outcome = run_network_seed(seed, options)
            .unwrap_or_else(|failure| panic!("seed {seed}: {failure}"));
        faults += outcome.partitions + outcome.holds;
    }
    assert!(faults > 0);
}

/// The plan and what the client was told are functions of the seed. The
/// count of connections is not: the server's storage work runs in real
/// time.
///
/// Under load this run failed with `missing block` (2026-10-14). The
/// server's storage work runs on real threads while the network's clock is
/// simulated, so load moves a write of the client between two reads of a
/// `getRepo` that the relay sent on `#sync`: the root was read, then the
/// write deleted blocks its new tree no longer references, then the blocks
/// were read. The export now reads the root and every block in one read
/// transaction (`ActorDb::open_snapshot`).
#[test]
fn a_network_seed_run_twice_gives_the_same_plan() {
    let first = run_network_seed(7, NetOptions::default()).unwrap();
    let second = run_network_seed(7, NetOptions::default()).unwrap();
    assert_eq!(first.trace, second.trace);
    assert_eq!(first.commits, second.commits);
    assert_eq!(first.events, second.events);
    assert_eq!(first.trace.len(), NetOptions::default().steps);
}
