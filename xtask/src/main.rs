//! Repository tasks. `cargo xtask codegen` generates `crates/pds-types/src/generated/mod.rs`
//! from `lexicons/`; `--check` fails when the checked-in output is stale.

#![allow(
    clippy::print_stderr,
    clippy::exit,
    clippy::too_many_lines,
    clippy::branches_sharing_code
)]

mod codegen;

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("codegen") => codegen(args.iter().any(|a| a == "--check")),
        _ => bail!("usage: cargo xtask codegen [--check]"),
    }
}

fn codegen(check: bool) -> Result<()> {
    let root = repo_root();
    let corpus = pds_lexicon::Corpus::load_dir(&root.join("lexicons")).context("load lexicons")?;
    for doc in corpus.docs() {
        doc.validate()
            .with_context(|| format!("lexicon {}", doc.id))?;
    }
    let unresolved = corpus.unresolved_refs();
    if !unresolved.is_empty() {
        bail!(
            "unresolved references:\n{}",
            unresolved
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    let generated = codegen::generate(&corpus)?;
    let formatted = rustfmt(&generated)?;
    let target = root.join("crates/pds-types/src/generated/mod.rs");
    if check {
        let current = std::fs::read_to_string(&target).unwrap_or_default();
        if current != formatted {
            bail!("{} is stale; run `cargo xtask codegen`", target.display());
        }
        eprintln!(
            "codegen: {} is current ({} lexicons)",
            target.display(),
            corpus.len()
        );
        return Ok(());
    }
    std::fs::write(&target, formatted).with_context(|| target.display().to_string())?;
    eprintln!(
        "codegen: wrote {} from {} lexicons",
        target.display(),
        corpus.len()
    );
    Ok(())
}

fn rustfmt(source: &str) -> Result<String> {
    use std::io::Write as _;
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout", "--quiet"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .context("spawn rustfmt")?;
    child
        .stdin
        .take()
        .context("rustfmt stdin")?
        .write_all(source.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("rustfmt failed; the generated source does not parse");
    }
    Ok(String::from_utf8(out.stdout)?)
}
