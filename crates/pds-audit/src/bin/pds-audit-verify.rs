//! The standalone verifier of the audit log (`docs/SPEC.md` §20.4).
//!
//! `pds-audit-verify <bundle.json>` recomputes the chain, the Merkle roots
//! and the signatures of an export, offline. It needs no server, no data
//! directory and no configuration. It exits 0 when the bundle verifies, 1
//! when it finds a problem and 2 when it cannot read the bundle.

#![allow(clippy::print_stdout, clippy::print_stderr)] // a command-line tool

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [file] = args.as_slice() else {
        eprintln!("usage: pds-audit-verify <bundle.json>");
        return ExitCode::from(2);
    };
    let bundle = std::fs::read(file)
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| error.to_string())
        });
    let bundle = match bundle {
        Ok(bundle) => bundle,
        Err(error) => {
            eprintln!("pds-audit-verify: {file}: {error}");
            return ExitCode::from(2);
        }
    };
    let report = pds_audit::verify::verify(&bundle);
    println!(
        "{} entries, {} checkpoints ({} with every entry in the bundle), {} proofs",
        report.entries, report.checkpoints, report.roots_recomputed, report.proofs
    );
    for problem in &report.problems {
        println!("problem: {problem}");
    }
    if report.ok() {
        println!("the bundle verifies");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
