//! The LinkJar PDS binary (`docs/SPEC.md` §14.2).
//!
//! This unit has `serve` (the read-only server), `verify`, `export-car`,
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

  serve              serve the data directory (read-only in this build)
  verify [--json]    check the data directory; exit 1 when it finds a difference
  export-car <did>   write one repository to standard output as a CAR
  check-config       validate the configuration and exit
  health             ask the local server's health route; exit 0 when it answers 200
  version            print the version

Configuration comes from the environment (PDS_*).";

/// The failure of a command, as the message for standard error.
type Failure = Box<dyn std::error::Error>;

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("LOG_LEVEL")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    // JSON lines on standard error; standard output belongs to the commands.
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

/// Writes one repository as the tree-ordered CAR of `Repo::export`.
fn export_car(config: &Config, did: &str) -> Result<(), Failure> {
    let store = ActorDb::open(&config.layout.actor_db(did))?;
    let root = store.root()?.ok_or("the store has no repository root")?;
    let budget = usize::try_from(store.block_count()?).unwrap_or(usize::MAX);
    let repo = Repo::load(&store, root.cid)?;
    let out = std::io::BufWriter::new(std::io::stdout().lock());
    let mut out = repo.export(out, budget)?;
    out.flush()?;
    Ok(())
}

/// Asks the server on this host for its health route.
fn health(config: &Config) -> Result<bool, Failure> {
    let url = format!("http://127.0.0.1:{}/xrpc/_health", config.port);
    runtime()?.block_on(async {
        let response = reqwest::Client::new()
            .get(url)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await?;
        Ok(response.status().is_success())
    })
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
    let config = Config::from_env()?;
    match (command, args.get(1).map(String::as_str)) {
        (Some("serve"), None) => {
            init_logging();
            runtime()?.block_on(serve::serve(&config))?;
            Ok(ExitCode::SUCCESS)
        }
        (Some("verify"), flag @ (None | Some("--json"))) => {
            let report = verify::verify(&config)?;
            if flag.is_some() {
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
        (Some("export-car"), Some(did)) => {
            export_car(&config, did)?;
            Ok(ExitCode::SUCCESS)
        }
        (Some("check-config"), None) => {
            config.serve_secrets()?;
            println!("the configuration is valid");
            Ok(ExitCode::SUCCESS)
        }
        (Some("health"), None) => Ok(if health(&config)? {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }),
        _ => Err(USAGE.into()),
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
