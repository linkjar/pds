//! The LinkJar PDS binary (`docs/SPEC.md` §14.2).
//!
//! It has `serve`, `verify`, `export-car`, the `audit` commands of §20,
//! `check-config`, `health` and `version`. The other commands of §14.2
//! arrive with their units (`docs/plan.md`).

use std::io::Write;
use std::process::ExitCode;

use linkjar_pds::config::Config;
use linkjar_pds::{serve, verify};
use pds_repo::Repo;
use pds_storage::ActorDb;

const USAGE: &str = "\
Usage: linkjar-pds <command>

  serve              serve the data directory
  verify [--json]    check the data directory; exit 1 when it finds a difference
  export-car <did>   write one repository to standard output as a CAR
  audit export [<from> [<to>]]
                     write the audit log, or a range of its entries, to standard
                     output as a bundle
  audit seal         begin the audit log with a signed snapshot of the accounts and
                     their repository heads, for the history before the log
  audit verify <file>
                     check a bundle offline; exit 1 when it finds a problem
  migrate-storage --to postgres|sqlite
                     copy the shared databases between the files of the data
                     directory and PDS_POSTGRES_URL, and compare the two
  reconcile [--remove-orphans]
                     at an entryway, compare the accounts with the repositories of
                     the member host; exit 1 when they differ. --remove-orphans
                     removes the repositories without an account
  check-config       validate the configuration and exit
  health             ask the local server's health route; exit 0 when it answers 200,
                     and 1 otherwise
  version            print the version

Configuration comes from the environment (PDS_*).";

/// The failure of a command, as the message for standard error.
type Failure = Box<dyn std::error::Error>;

/// The runtime of `serve`.
fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
}

/// Writes one repository as the tree-ordered CAR of `Repo::export`.
fn export_car(config: &Config, did: &str) -> Result<(), Failure> {
    // The argument becomes a part of a path, so it is a DID first.
    let did = did.parse::<pds_types::Did>()?;
    let did = did.as_str();
    let store = ActorDb::open(&config.layout.actor_db(did))?;
    let root = store.root()?.ok_or("the store has no repository root")?;
    let budget = usize::try_from(store.block_count()?).unwrap_or(usize::MAX);
    let repo = Repo::load(&store, root.cid)?;
    let out = std::io::BufWriter::new(std::io::stdout().lock());
    let mut out = repo.export(out, budget)?;
    out.flush()?;
    Ok(())
}

/// Asks the server on this host for its health route. Only the status 200
/// is healthy (SPEC §14.2).
fn health() -> Result<(), Failure> {
    let config = Config::from_env()?;
    let url = format!("http://127.0.0.1:{}/xrpc/_health", config.port);
    // The request goes to this host, never to a proxy from the environment.
    let client = reqwest::Client::builder().no_proxy().build()?;
    let status = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            client
                .get(url)
                .timeout(std::time::Duration::from_secs(3))
                .send()
                .await
        })?
        .status();
    if status == reqwest::StatusCode::OK {
        Ok(())
    } else {
        Err(format!("the health route answered {status}").into())
    }
}

/// The exit code of `health`: 0 when the server is healthy and 1 for every
/// other outcome, no answer and an invalid configuration among them. A
/// container health check has no other exit code.
#[allow(clippy::print_stderr)] // the reason is for the health check's log
fn health_exit() -> ExitCode {
    match health() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("linkjar-pds: health: {error}");
            ExitCode::FAILURE
        }
    }
}

#[allow(clippy::print_stdout)] // a command reports on standard output
fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let command = args.first().map(String::as_str);
    if matches!(command, Some("version" | "--version")) {
        println!("linkjar-pds {}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    if matches!(command, None | Some("help" | "--help")) {
        println!("{USAGE}");
        return Ok(if command.is_none() {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        });
    }
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    if words == ["health"] {
        return Ok(health_exit());
    }
    // The verifier needs no configuration: it reads a file (SPEC §20.4).
    if let ["audit", "verify", file] = words.as_slice() {
        return audit_verify(file);
    }
    let config = Config::from_env()?;
    // A command takes its own arguments and no others.
    match words.as_slice() {
        ["serve"] => {
            pds_sidecar::process::init_logging();
            let runtime = runtime()?;
            let served = runtime.block_on(serve::serve(&config));
            // A read still running on the blocking pool does not hold the
            // exit: the limit of SPEC §14.3 has passed or nothing is open.
            runtime.shutdown_timeout(std::time::Duration::from_secs(1));
            served?;
            Ok(ExitCode::SUCCESS)
        }
        ["verify"] | ["verify", "--json"] => {
            warn_unknown(&config);
            // The S3-compatible store sends its requests on a runtime.
            let runtime = runtime()?;
            let blobs = config.blob_store(runtime.handle())?;
            let report = verify::verify_with(&config, blobs.as_ref())?;
            if words.len() == 2 {
                println!("{}", report.to_json());
            } else {
                print!("{report}");
            }
            Ok(if report.findings.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        ["export-car", did] => {
            export_car(&config, did)?;
            Ok(ExitCode::SUCCESS)
        }
        ["audit", "export", range @ ..] if range.len() <= 2 => {
            let bound = |at: usize, default: u64| match range.get(at) {
                Some(text) => text.parse::<u64>().map_err(|_| Failure::from(USAGE)),
                None => Ok(default),
            };
            let log = pds_xrpc::audit::AuditLog::open_in(&config.layout, None)?;
            println!("{}", log.export(bound(0, 1)?, bound(1, u64::MAX)?)?);
            Ok(ExitCode::SUCCESS)
        }
        ["audit", "seal"] => {
            audit_seal(&config)?;
            Ok(ExitCode::SUCCESS)
        }
        ["migrate-storage", "--to", target] => {
            migrate_storage(&config, target)?;
            Ok(ExitCode::SUCCESS)
        }
        ["reconcile"] | ["reconcile", "--remove-orphans"] => {
            let runtime = runtime()?;
            let report =
                runtime.block_on(linkjar_pds::reconcile::reconcile(&config, words.len() == 2))?;
            print!("{report}");
            Ok(if report.is_clean() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        ["check-config"] => {
            warn_unknown(&config);
            config.serve_secrets()?;
            println!("the configuration is valid");
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(USAGE.into()),
    }
}

/// `migrate-storage --to postgres|sqlite` (SPEC §8.4): copies the shared
/// tier to the other backend and compares the two. The configuration
/// names both: the files of the data directory, and `PDS_POSTGRES_URL`.
/// The target must be empty. No server may run: the command takes the
/// lock of the data directory and the lease of the event log.
#[allow(clippy::print_stdout)] // a command reports on standard output
fn migrate_storage(config: &Config, target: &str) -> Result<(), Failure> {
    let postgres = config.layout.clone();
    if postgres.shared.is_none() {
        return Err("migrate-storage needs PDS_POSTGRES_URL: the Postgres side of the copy".into());
    }
    let files = pds_storage::DataLayout {
        shared: None,
        ..postgres.clone()
    };
    let (from, to) = match target {
        "postgres" => (&files, &postgres),
        "sqlite" => (&postgres, &files),
        _ => return Err(USAGE.into()),
    };
    let _lock = serve::lock_data_directory(config)?;
    let _lease = postgres
        .shared
        .as_ref()
        .map(pds_storage::SharedTier::sequencer_lease)
        .transpose()?
        .flatten()
        .ok_or(
            "a server holds the lease of the event log in this Postgres database; stop it first",
        )?;
    let now = pds_sequencer::iso_millis(serve::now_ms());
    let copied = pds_storage::migrate::copy_shared(from, to, &now)?;
    for table in &copied {
        println!("{}.{}: {} rows", table.database, table.table, table.rows);
    }
    println!(
        "copied {} tables to {target}; the row counts, the head of the event log and the last checkpoint are the same",
        copied.len()
    );
    Ok(())
}

/// `audit seal` (SPEC §20.4): the one entry that stands for the history
/// before the log. It names how many accounts the server has and the
/// SHA-256 of their DIDs with their repository heads, and says that
/// nothing before it is in the log. It is the first entry or it is
/// refused.
#[allow(clippy::print_stdout)] // a command reports on standard output
fn audit_seal(config: &Config) -> Result<(), Failure> {
    use sha2::{Digest, Sha256};
    let signer = match &config.audit_key {
        Some(seed) => Some(
            pds_audit::Signer::from_seed(seed)
                .ok_or("the audit checkpoint key is not an Ed25519 seed")?,
        ),
        None => None,
    };
    let has_key = signer.is_some();
    let log = pds_xrpc::audit::AuditLog::open_in(&config.layout, signer)?;
    if !log.is_empty()? {
        return Err("the audit log has entries already; a seal is its first entry".into());
    }
    let accounts = config.layout.accounts()?;
    let mut digest = Sha256::new();
    let mut count = 0_u64;
    for actor in accounts.all_actors()? {
        let (cid, rev) = accounts.repo_root(&actor.did)?.unwrap_or_default();
        digest.update(format!("{} {rev} {cid}\n", actor.did));
        count += 1;
    }
    let snapshot = data_encoding::HEXLOWER.encode(&digest.finalize());
    let time = pds_sequencer::iso_millis(serve::now_ms());
    let what = pds_audit::Draft {
        actor: "operator".to_owned(),
        action: "audit.sealed".to_owned(),
        subject: Some(format!("sha256:{snapshot}")),
        credential: Some("server".to_owned()),
        outcome: "ok".to_owned(),
        reason: Some(format!(
            "The history before this entry is not in the log. This entry stands for {count} accounts and their repository heads."
        )),
        ..pds_audit::Draft::default()
    };
    log.record(&time, what)?;
    let checkpoint = log.checkpoint(&time)?;
    println!("sealed {count} accounts as sha256:{snapshot}");
    match checkpoint {
        Some(checkpoint) => println!("signed by {}", checkpoint.key_id),
        None if has_key => println!("no checkpoint was signed"),
        None => println!("no checkpoint key is configured, so the seal is not signed"),
    }
    Ok(())
}

/// `audit verify <file>`: recomputes the chain, the roots and the
/// signatures of a bundle.
#[allow(clippy::print_stdout)] // a command reports on standard output
fn audit_verify(file: &str) -> Result<ExitCode, Failure> {
    let bundle: serde_json::Value = serde_json::from_slice(&std::fs::read(file)?)?;
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
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

/// Warns on standard error about each `PDS_*` variable that SPEC Appendix C
/// does not name (SPEC §14.1). `serve` writes the same warning to its log.
#[allow(clippy::print_stderr)] // standard output belongs to the command's result
fn warn_unknown(config: &Config) {
    for name in &config.unknown_variables {
        eprintln!("linkjar-pds: warning: unknown variable {name}");
    }
}

#[allow(clippy::print_stderr)] // a command reports its failure on standard error
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("linkjar-pds: {error}");
            ExitCode::from(2)
        }
    }
}
