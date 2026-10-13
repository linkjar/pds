//! `linkjar-pds-console`: the operations console of LinkJar PDS.

use std::process::ExitCode;

use pds_console::config::Config;

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("LOG_LEVEL")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .try_init();
}

/// Resolves at SIGINT or SIGTERM.
async fn stop() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
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
    axum::serve(listener, pds_console::router(console))
        .with_graceful_shutdown(stop())
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
