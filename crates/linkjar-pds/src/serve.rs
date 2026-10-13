//! The `serve` command: the read-only server over a data directory.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pds_blob::DiskBlobStore;
use pds_identity::{DidCache, DidResolver, ResolverConfig};
use pds_storage::DidCacheDb;
use pds_xrpc::auth::AuthConfig;
use pds_xrpc::{AppState, ServerConfig};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::TcpListener;
use tokio::sync::Notify;

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

/// How long the server waits for open connections after it is told to stop
/// (SPEC §14.3): 8 seconds. `docker stop` kills a container after 10 seconds
/// by default, so the server exits by itself before that.
pub const SHUTDOWN_LIMIT: Duration = Duration::from_secs(8);

/// Binds the listener of `serve` on every address of the host.
///
/// It binds the IPv6 wildcard with `IPV6_V6ONLY` off, so the one socket
/// accepts IPv4 too, whatever the system default is. On a host without
/// IPv6 it binds the IPv4 wildcard. Any other failure, an address in use
/// among them, is the caller's error.
fn bind(port: u16) -> std::io::Result<TcpListener> {
    let open = |domain: Domain, address: SocketAddr| -> std::io::Result<Socket> {
        let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
        if domain == Domain::IPV6 {
            socket.set_only_v6(false)?;
        }
        socket.set_reuse_address(true)?;
        socket.set_nonblocking(true)?;
        socket.bind(&address.into())?;
        socket.listen(1024)?;
        Ok(socket)
    };
    let socket = match open(
        Domain::IPV6,
        SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)),
    ) {
        Ok(socket) => socket,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => return Err(error),
        Err(error) => {
            tracing::warn!(%error, "the IPv6 wildcard did not bind; listening on IPv4 only");
            open(
                Domain::IPV4,
                SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
            )?
        }
    };
    TcpListener::from_std(socket.into())
}

/// Serves until the process is told to stop.
pub async fn serve(config: &Config) -> Result<(), ServeError> {
    for name in &config.unknown_variables {
        tracing::warn!(variable = %name, "unknown variable: SPEC Appendix C does not name it");
    }
    let state = app_state(config)?;
    let listener = bind(config.port)?;
    let address = listener.local_addr()?;
    tracing::info!(%address, did = %config.service_did, "serving");
    serve_until(listener, state, shutdown(), SHUTDOWN_LIMIT).await
}

/// Serves `state` on `listener` until `stop` resolves, then waits at most
/// `limit` for the open connections to finish (SPEC §14.3).
pub async fn serve_until(
    listener: TcpListener,
    state: AppState,
    stop: impl Future<Output = ()> + Send + 'static,
    limit: Duration,
) -> Result<(), ServeError> {
    let stopping = Arc::new(Notify::new());
    let told = Arc::clone(&stopping);
    let server = axum::serve(listener, pds_xrpc::router(state))
        .with_graceful_shutdown(async move {
            stop.await;
            told.notify_one();
        })
        .into_future();
    // The limit starts when the server is told to stop, not when it starts.
    let expired = async {
        stopping.notified().await;
        tokio::time::sleep(limit).await;
    };
    tokio::select! {
        result = server => result?,
        () = expired => tracing::warn!(
            limit_ms = u64::try_from(limit.as_millis()).unwrap_or(u64::MAX),
            "connections were still open at the shutdown limit; the process ends without them"
        ),
    }
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
