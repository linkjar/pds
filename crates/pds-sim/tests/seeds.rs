//! A few seeds on every test run, and the meta-test of determinism
//! (`docs/SPEC.md` §19.2): a seed run twice gives the same trace, byte for
//! byte. The nightly workflow runs many more seeds.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use pds_sim::{Options, run_seed_blocking};

#[test]
fn the_first_seeds_pass_every_oracle() {
    let mut stops = 0;
    for seed in 1..=6 {
        let outcome = run_seed_blocking(seed, Options::default())
            .unwrap_or_else(|failure| panic!("seed {seed}: {failure}"));
        assert!(
            outcome.commits > 20,
            "seed {seed}: {} commits",
            outcome.commits
        );
        stops += outcome.crashes;
    }
    assert!(stops > 0, "no seed stopped the server at a fail point");
}

#[test]
fn a_seed_run_twice_gives_the_same_trace() {
    let options = Options {
        steps: 120,
        ..Options::default()
    };
    let first = run_seed_blocking(11, options).unwrap();
    let second = run_seed_blocking(11, options).unwrap();
    assert!(
        first.crashes > 0,
        "seed 11 stops the server at a fail point"
    );
    assert_eq!(first.trace.join("\n"), second.trace.join("\n"));
    assert_eq!(first, second);
    // Another seed is another plan.
    let other = run_seed_blocking(12, options).unwrap();
    assert_ne!(first.trace, other.trace);
}
