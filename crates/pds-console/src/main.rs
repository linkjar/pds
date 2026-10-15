//! `linkjar-pds-console`: the operations console of LinkJar PDS.

use std::process::ExitCode;

use pds_console::config::Config;
use pds_sidecar::process::{init_logging, shutdown};

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
    axum::serve(listener, pds_console::router(console))
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|error| error.to_string())
}

#[allow(clippy::print_stderr)] // a command reports its failure on standard error
#[tokio::main]
async fn main() -> ExitCode {
    init_logging();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("linkjar-pds-console: {error}");
            ExitCode::from(2)
        }
    }
}
