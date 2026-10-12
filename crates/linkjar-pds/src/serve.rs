//! The `serve` command: the read-only server over a data directory.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use pds_blob::DiskBlobStore;
use pds_identity::{DidCache, DidResolver, ResolverConfig};
use pds_storage::DidCacheDb;
use pds_xrpc::auth::AuthConfig;
use pds_xrpc::{AppState, ServerConfig};

use crate::config::Config;

/// Why the server did not start, or stopped.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The configuration lacks something `serve` needs.
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    /// The HTTP client could not be built.
    #[error("http client: {0}")]
    Http(#[from] reqwest::Error),
    /// The listener could not be bound, or the server failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// The wall clock in milliseconds since the Unix epoch.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// `did_cache.sqlite` as the resolver's cache. A failing cache answers as
/// an empty one.
struct SqliteDidCache(Mutex<DidCacheDb>);

impl SqliteDidCache {
    fn with<T>(
        &self,
        call: impl FnOnce(&DidCacheDb) -> Result<T, pds_storage::StorageError>,
    ) -> Option<T> {
        let db = self.0.lock().ok()?;
        call(&db)
            .inspect_err(|error| tracing::warn!(%error, "the DID cache failed"))
            .ok()
    }
}

impl DidCache for SqliteDidCache {
    fn get(&self, did: &str) -> Option<(String, i64)> {
        self.with(|db| db.get(did))?
            .map(|cached| (cached.doc, cached.updated_at_ms))
    }

    fn put(&self, did: &str, doc: &str, now_ms: i64) {
        self.with(|db| db.put(did, doc, now_ms));
    }

    fn clear(&self, did: &str) {
        self.with(|db| db.clear(did));
    }
}

/// Builds the server state from the configuration.
pub fn app_state(config: &Config) -> Result<AppState, ServeError> {
    let (jwt_secret, admin_password) = config.serve_secrets()?;
    let http = reqwest::Client::builder()
        .user_agent(concat!("linkjar-pds/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let cache: Option<Arc<dyn DidCache>> = match DidCacheDb::open(&config.layout.did_cache_db) {
        Ok(db) => Some(Arc::new(SqliteDidCache(Mutex::new(db)))),
        Err(error) => {
            tracing::warn!(%error, "the DID cache is unavailable; every DID is resolved on the network");
            None
        }
    };
    let resolver = DidResolver::new(
        ResolverConfig {
            plc_url: config.plc_url.clone(),
            timeout: config.resolver_timeout,
            stale_ttl: config.did_cache_stale_ttl,
            max_ttl: config.did_cache_max_ttl,
            dev_mode: config.dev_mode,
        },
        http.clone(),
        cache,
        now_ms,
    );
    let server = ServerConfig {
        version: config.version.clone(),
        service_did: config.service_did.clone(),
        handle_domains: config.handle_domains.clone(),
        invite_required: config.invite_required,
        blob_upload_limit: config.blob_upload_limit,
        privacy_policy_url: config.privacy_policy_url.clone(),
        terms_of_service_url: config.terms_of_service_url.clone(),
        contact_email: config.contact_email.clone(),
        appview_url: config.appview_url.clone(),
        proxy_timeout: config.proxy_timeout,
        auth: AuthConfig {
            jwt_secret: jwt_secret.as_bytes().to_vec(),
            admin_password: admin_password.to_owned(),
            service_did: config.service_did.clone(),
        },
    };
    Ok(AppState::new(
        server,
        config.layout.clone(),
        config.blob_dir.clone().map(DiskBlobStore::new),
        resolver,
        http,
        now_ms,
    ))
}

/// Serves until the process is told to stop.
pub async fn serve(config: &Config) -> Result<(), ServeError> {
    let state = app_state(config)?;
    let address = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, did = %config.service_did, "serving");
    axum::serve(listener, pds_xrpc::router(state))
        .with_graceful_shutdown(shutdown())
        .await?;
    tracing::info!("stopped");
    Ok(())
}

/// Resolves on `SIGTERM` or an interrupt (SPEC §14.3).
async fn shutdown() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}
