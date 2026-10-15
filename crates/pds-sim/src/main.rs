//! The seed runner: `pds-sim [--seeds N] [--from S] [--steps N] [--accounts N]
//! [--faults N] [--power-loss N] [--trace]`, and `pds-sim --network` for the
//! runs on a simulated network.
//!
//! It runs the plans of seeds `S` to `S + N - 1`, prints one line for each
//! and a summary, and exits 1 when a seed fails. A failing seed is the
//! whole report of a failure: run it again with `--from <seed> --seeds 1
//! --trace` and the same steps happen (`docs/SPEC.md` §19.1, §19.6).

#![allow(clippy::print_stdout, clippy::print_stderr)] // a command-line tool

use std::process::ExitCode;

use pds_sim::net::{NetOptions, run_network_seed};
use pds_sim::{Options, run_seed_blocking};

struct Args {
    from: u64,
    seeds: u64,
    options: Options,
    trace: bool,
    network: bool,
}

fn parse() -> Result<Args, String> {
    let mut args = Args {
        from: 1,
        seeds: 20,
        options: Options::default(),
        trace: false,
        network: false,
    };
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        let mut number = |name: &str| -> Result<u64, String> {
            words
                .next()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| format!("{name} takes a whole number"))
        };
        match word.as_str() {
            "--from" => args.from = number("--from")?,
            "--seeds" => args.seeds = number("--seeds")?,
            "--steps" => {
                args.options.steps = usize::try_from(number("--steps")?).unwrap_or(usize::MAX);
            }
            "--accounts" => {
                args.options.accounts = usize::try_from(number("--accounts")?)
                    .unwrap_or(1)
                    .clamp(1, 26);
            }
            "--faults" => args.options.fault_percent = number("--faults")?.min(100),
            "--power-loss" => {
                args.options.power_loss_percent = number("--power-loss")?.min(100);
            }
            "--trace" => args.trace = true,
            "--network" => args.network = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

/// The network runs: the server, a relay and a client on a simulated
/// network with partitions and held links.
fn network(args: &Args) -> ExitCode {
    let defaults = NetOptions::default();
    let options = NetOptions {
        accounts: args.options.accounts,
        // The plan of a network run is shorter unless `--steps` says more.
        steps: if args.options.steps == Options::default().steps {
            defaults.steps
        } else {
            args.options.steps
        },
        buffer: std::env::var("PDS_SIM_BUFFER")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(defaults.buffer),
    };
    let mut failed = Vec::new();
    for seed in args.from..args.from.saturating_add(args.seeds) {
        match run_network_seed(seed, options) {
            Ok(outcome) => {
                println!(
                    "seed {seed}: ok, {} commits, {} events, {} partitions, {} holds, {} connections, {} closed as too slow",
                    outcome.commits,
                    outcome.events,
                    outcome.partitions,
                    outcome.holds,
                    outcome.connections,
                    outcome.too_slow
                );
                if args.trace {
                    for line in &outcome.trace {
                        println!("  {line}");
                    }
                }
            }
            Err(failure) => {
                println!("seed {seed}: FAILED, {failure}");
                failed.push(seed);
            }
        }
    }
    println!(
        "{} network seeds from {}, {} steps each: {} failed",
        args.seeds,
        args.from,
        options.steps,
        failed.len()
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        println!("failed seeds: {failed:?}");
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(args) => args,
        Err(message) => {
            eprintln!(
                "{message}\nusage: pds-sim [--seeds N] [--from S] [--steps N] [--accounts N] [--faults PERCENT] [--power-loss PERCENT] [--trace]\n       pds-sim --network [--seeds N] [--from S] [--steps N] [--accounts N] [--trace]"
            );
            return ExitCode::from(2);
        }
    };
    if args.network {
        return network(&args);
    }
    let mut failed = Vec::new();
    let (mut commits, mut crashes, mut power, mut events) = (0_u64, 0_u64, 0_u64, 0_i64);
    for seed in args.from..args.from.saturating_add(args.seeds) {
        match run_seed_blocking(seed, args.options) {
            Ok(outcome) => {
                println!(
                    "seed {seed}: ok, {} commits, {} stops, {} power losses ({} writes lost, {} interrupted commits stored), {} starts, {} events, {} repairs with #sync",
                    outcome.commits,
                    outcome.crashes,
                    outcome.power_losses,
                    outcome.writes_lost,
                    outcome.interrupted_stored,
                    outcome.starts,
                    outcome.events,
                    outcome.relay_resyncs
                );
                if args.trace {
                    for line in &outcome.trace {
                        println!("  {line}");
                    }
                }
                commits += u64::from(outcome.commits);
                crashes += u64::from(outcome.crashes);
                power += u64::from(outcome.power_losses);
                events += outcome.events;
            }
            Err(failure) => {
                println!("seed {seed}: FAILED, {failure}");
                failed.push(seed);
            }
        }
    }
    println!(
        "{} seeds from {}, {} steps each: {} failed; {commits} commits, {crashes} stops, {power} power losses, {events} events",
        args.seeds,
        args.from,
        args.options.steps,
        failed.len()
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        println!("failed seeds: {failed:?}");
        ExitCode::FAILURE
    }
}
