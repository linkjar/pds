//! Builds the stylesheet of the console: the standalone Tailwind CLI reads
//! `src/theme.css` and the classes of `src/*.rs`, and writes
//! `$OUT_DIR/tailwind.css`, which the pages link as an asset.
//!
//! The release of the CLI is pinned, and so is the hash of its binary for
//! each platform the console is built on, the way the workflows pin
//! actions by commit. `TAILWIND_CLI` names a local executable instead, for a
//! build without network.

// A build script speaks to Cargo on standard output.
#![allow(clippy::print_stdout)]

use std::error::Error;

use topcoat::tailwind::{BuildConfig, ExecutableSource};

/// The release of the standalone Tailwind CLI, without the leading `v`.
const VERSION: &str = "4.3.2";

/// The variable that names a local Tailwind CLI executable.
const CLI_VAR: &str = "TAILWIND_CLI";

/// The SHA-256 of the release binary per host platform, from
/// `sha256sums.txt` of
/// <https://github.com/tailwindlabs/tailwindcss/releases/tag/v4.3.2>, read
/// on 2026-10-15. The pair is `std::env::consts::{OS, ARCH}` of the build
/// host; the CLI runs there, not on the target.
const CHECKSUMS: &[(&str, &str, &str)] = &[
    (
        "linux",
        "x86_64",
        "5036c4fb4328e0bcdbb6065c70d8ac9452e0d4c947113a788a8f94fd390425c1",
    ),
    (
        "linux",
        "aarch64",
        "394ddccc2402cfa3abd97dfba56f3587781a3d6e6ce66e65ceada14beb7664b8",
    ),
    (
        "macos",
        "aarch64",
        "b800b0659dc64b9f03ede5660244d9415d777d5739ae2889280877ca37be742a",
    ),
    (
        "macos",
        "x86_64",
        "cef8f110471e889c3c4409055cf8aff33076f58a081867b0dfc6534b290bfbb0",
    ),
];

/// Where the CLI comes from: the variable when it is set, else the pinned
/// download for this host.
fn executable() -> Result<ExecutableSource, String> {
    if std::env::var_os(CLI_VAR).is_some() {
        return Ok(ExecutableSource::Env(CLI_VAR.to_owned()));
    }
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    CHECKSUMS
        .iter()
        .find(|(host_os, host_arch, _)| *host_os == os && *host_arch == arch)
        .map(|(_, _, sum)| ExecutableSource::Github {
            version: VERSION.to_owned(),
            checksum: Some(format!("sha256:{sum}")),
        })
        .ok_or_else(|| {
            format!(
                "no pinned Tailwind CLI for {os} {arch}; set {CLI_VAR} to a tailwindcss {VERSION} executable"
            )
        })
}

fn main() -> Result<(), Box<dyn Error>> {
    // A rerun directive replaces Cargo's own change detection, so every
    // input of the stylesheet is named: the sources Tailwind scans, this
    // script, and the variable.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-env-changed={CLI_VAR}");
    BuildConfig::new()
        .executable_source(executable()?)
        .input("src/theme.css")
        .render()?;
    Ok(())
}
