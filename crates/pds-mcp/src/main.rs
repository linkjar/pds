//! `linkjar-pds-mcp`: the MCP server of LinkJar PDS.
//!
//! `linkjar-pds-mcp health` is the health check of the image. Without an
//! argument the server serves.

use std::process::ExitCode;

use pds_mcp::config::Config;
use pds_sidecar::process::{init_logging, shutdown};

/// Asks the server on this host for its health route. The exit code is a
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
            eprintln!("linkjar-pds-mcp: health: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let vars = std::env::vars().collect();
    let config = Config::from_vars(&vars).map_err(|error| error.to_string())?;
    let address = config.address;
    tracing::info!(
        client_id = %config.client_id(),
        "the OAuth client id of the server at the PDS; PDS_MCP_CLIENT_IDS must hold it"
    );
    let mcp = pds_mcp::Mcp::new(config).map_err(|error| error.to_string())?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| format!("cannot listen on {address}: {error}"))?;
    tracing::info!(%address, "serving MCP");
    axum::serve(listener, pds_mcp::router(mcp))
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|error| error.to_string())
}

#[allow(clippy::print_stderr)] // a command reports its failure on standard error
#[tokio::main]
async fn main() -> ExitCode {
    init_logging();
    let mut args = std::env::args().skip(1);
    let outcome = match args.next().as_deref() {
        Some("health") => return health().await,
        Some(other) => Err(format!(
            "unknown command {other:?}; the server takes none, or health"
        )),
        None => run().await,
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("linkjar-pds-mcp: {error}");
            ExitCode::from(2)
        }
    }
}
