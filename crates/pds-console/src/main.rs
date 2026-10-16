//! `linkjar-pds-console`: the operations console of LinkJar PDS.
//!
//! `linkjar-pds-console bundle-assets [DIR]` writes the assets of the
//! pages next to the executable (or into `DIR`) and exits; the image does
//! this in its build stage. `linkjar-pds-console health` is the health
//! check of the image. Without an argument the console serves.

use std::{path::PathBuf, process::ExitCode};

use pds_console::config::Config;
use pds_sidecar::process::{init_logging, shutdown};

/// Asks the console on this host for its health route. The exit code is a
/// container health check's: 0 when the route answers 200, 1 for every
/// other outcome, an invalid configuration among them.
#[allow(clippy::print_stderr)] // the reason is for the health check's log
async fn health() -> ExitCode {
    let vars = std::env::vars().collect();
    let checked = match Config::from_vars(&vars) {
        Ok(config) => pds_sidecar::process::health(config.address).await,
        Err(error) => Err(error.to_string()),
    };
    match checked {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("linkjar-pds-console: health: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Writes the asset bundle of this executable.
fn bundle(dir: Option<PathBuf>) -> Result<(), String> {
    let dir = match dir {
        Some(dir) => dir,
        None => std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("assets")))
            .ok_or_else(|| "cannot find the directory of the executable".to_owned())?,
    };
    pds_console::assets::bundle(&dir).map_err(|error| error.to_string())?;
    tracing::info!(dir = %dir.display(), "the assets are bundled");
    Ok(())
}

async fn run() -> Result<(), String> {
    let vars = std::env::vars().collect();
    let config = Config::from_vars(&vars).map_err(|error| error.to_string())?;
    let address = config.address;
    tracing::info!(
        client_id = %config.client_id(),
        "the OAuth client id of the console; PDS_OPS_CLIENT_IDS of the server must hold it"
    );
    let console = pds_console::Console::new(config).map_err(|error| error.to_string())?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| format!("cannot listen on {address}: {error}"))?;
    tracing::info!(%address, "serving the console");
    pds_console::serve(listener, pds_console::router(console), shutdown())
        .await
        .map_err(|error| error.to_string())
}

#[allow(clippy::print_stderr)] // a command reports its failure on standard error
#[tokio::main]
async fn main() -> ExitCode {
    init_logging();
    let mut args = std::env::args().skip(1);
    let outcome = match args.next().as_deref() {
        Some("bundle-assets") => bundle(args.next().map(PathBuf::from)),
        Some("health") => return health().await,
        Some(other) => Err(format!(
            "unknown command {other:?}; the console takes none, bundle-assets [DIR] or health"
        )),
        None => run().await,
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("linkjar-pds-console: {error}");
            ExitCode::from(2)
        }
    }
}
