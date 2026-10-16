//! The `serve` command: the server over a data directory.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pds_identity::{DidCache, DidResolver, ResolverConfig};
use pds_sequencer::{Crawlers, Sequencer};
use pds_storage::{DidCacheDb, ReadPool};
use pds_types::TidGenerator;
use pds_xrpc::auth::AuthConfig;
use pds_xrpc::{AppState, ServerConfig, WriteSettings, WriteState};
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
    /// A database of the data directory could not be opened or repaired.
    #[error("storage: {0}")]
    Storage(String),
    /// `PDS_FAIL_POINTS` does not parse.
    #[error("PDS_FAIL_POINTS: {0}")]
    FailPoints(#[from] pds_storage::fail::BadSetting),
    /// The listener could not be bound, or the server failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// The wall clock in milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
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

/// Creates the shared databases that a new data directory lacks, as the
/// Reference creates them at the pin. A database that exists is left as it
/// is.
pub fn create_missing_databases(config: &Config) -> Result<(), ServeError> {
    let now = pds_types::iso_millis(now_ms());
    let layout = &config.layout;
    let created = layout
        .create_missing(&now)
        .map_err(|error| ServeError::Storage(error.to_string()))?;
    for database in created {
        tracing::info!(database, "created a database of the shared tier");
    }
    if config.profile == crate::config::ProfileName::Linkjar {
        let applied = layout
            .accounts_writable()
            .and_then(|mut accounts| accounts.apply_linkjar_migrations(&now))
            .map_err(|error| ServeError::Storage(error.to_string()))?;
        if applied > 0 {
            tracing::info!(applied, "applied migrations of the linkjar profile");
        }
    }
    Ok(())
}

/// The profile that `PDS_PROFILE` names (SPEC §12.1).
fn profile(config: &Config) -> Arc<dyn pds_ext::Profile> {
    if config.profile == crate::config::ProfileName::Linkjar {
        Arc::new(pds_ext_linkjar::Linkjar {
            providers: config.external_providers.clone(),
        })
    } else {
        Arc::new(pds_ext::Stock)
    }
}

/// The settings of the HTTP surface, from the configuration.
fn server_config(config: &Config, jwt_secret: &str, admin_password: &str) -> ServerConfig {
    // The key of an entryway. The configuration refused a value that is
    // not a key.
    let entryway_key = config.member_host.as_ref().and_then(|host| {
        pds_identity::Keypair::from_secret_bytes(
            pds_identity::Algorithm::Secp256k1,
            &host.signing_key,
        )
        .ok()
        .map(Arc::new)
    });
    let profile = profile(config);
    ServerConfig {
        hostname: config.hostname.clone(),
        version: config.version.clone(),
        handle_domains: config.handle_domains.clone(),
        invite_required: config.invite_required,
        blob_upload_limit: config.blob_upload_limit,
        privacy_policy_url: config.privacy_policy_url.clone(),
        terms_of_service_url: config.terms_of_service_url.clone(),
        branding: pds_xrpc::state::Branding {
            service_name: config.mail.service_name.clone(),
            logo_url: config.mail.logo_url.clone(),
            home_url: config.mail.home_url.clone(),
            support_url: config.support_url.clone(),
        },
        contact_email: config.contact_email.clone(),
        appview_url: config.appview_url.clone(),
        appview_did: config.appview_did.clone(),
        mod_service: config.mod_service.clone(),
        report_service: config.report_service.clone(),
        proxy_timeout: config.proxy_timeout,
        auth: AuthConfig {
            jwt_secret: jwt_secret.as_bytes().to_vec(),
            admin_password: admin_password.to_owned(),
            service_did: config.service_did.clone(),
            entryway: config
                .entryway
                .as_ref()
                .map(|entryway| pds_xrpc::auth::EntrywayAuth {
                    did: entryway.did.clone(),
                    key: entryway.jwt_key,
                })
                // An entryway takes its own tokens as its member hosts do.
                .or_else(|| {
                    entryway_key
                        .as_ref()
                        .map(|key| pds_xrpc::auth::EntrywayAuth {
                            did: config.service_did.clone(),
                            key: key.public_key(),
                        })
                }),
            signer: config
                .member_host
                .as_ref()
                .zip(entryway_key)
                .map(|(host, key)| pds_xrpc::auth::EntrywaySigner {
                    key,
                    legacy_audience: Some(host.did.clone()),
                }),
        },
        mail: pds_xrpc::mail::MailConfig {
            templates: pds_xrpc::mail::Templates::of(profile.as_ref()),
            ..config.mail.clone()
        },
        rate_limits: config.rate_limits.clone(),
        oauth: config.oauth.clone(),
        ops_client_ids: config.ops_client_ids.clone(),
        mcp_client_ids: config.mcp_client_ids.clone(),
        member_host: config
            .member_host
            .as_ref()
            .map(|host| pds_xrpc::state::MemberHost {
                url: host.url.clone(),
                internal_url: host.internal_url.clone(),
                admin_password: host.admin_password.clone(),
            }),
        entryway: config
            .entryway
            .as_ref()
            .map(|entryway| pds_xrpc::state::Entryway {
                url: entryway.url.clone(),
                did: entryway.did.clone(),
                plc_rotation_key: entryway.plc_rotation_key.clone(),
                admin_token: entryway.admin_token.clone(),
            }),
        audit_repo: config.audit_repo.clone(),
        profile,
        appview_cdn_url_pattern: config.appview_cdn_url_pattern.clone(),
        // A member host has no rotation key: its entryway signs the PLC
        // operations. It still knows the directory and its own address.
        identity: config
            .plc_rotation_key
            .clone()
            .or_else(|| config.entryway.as_ref().map(|_| Vec::new()))
            .map(|rotation_key| pds_xrpc::state::IdentityConfig {
                plc_url: config.plc_url.trim_end_matches('/').to_owned(),
                public_url: config.public_url(),
                rotation_key,
                recovery_did_key: config.recovery_did_key.clone(),
                dev_mode: config.dev_mode,
            }),
    }
}

/// Builds the server state from the configuration.
pub fn app_state(config: &Config) -> Result<AppState, ServeError> {
    let (jwt_secret, admin_password) = config.serve_secrets()?;
    let http = reqwest::Client::builder()
        .user_agent(concat!("linkjar-pds/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let cache: Option<Arc<dyn DidCache>> = match config.layout.did_cache() {
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
    let server = server_config(config, jwt_secret, admin_password);
    let storage = |error: pds_storage::StorageError| ServeError::Storage(error.to_string());
    // An entryway keeps no repository: its member host has the firehose
    // and asks the crawlers itself (SPEC §21.3).
    let crawlers = Crawlers::new(
        config.hostname.clone(),
        if config.member_host.is_some() {
            Vec::new()
        } else {
            config.crawlers.clone()
        },
        http.clone(),
    );
    let sequencer = Sequencer::open_in(&config.layout, crawlers, now_ms).map_err(storage)?;
    let writes = WriteState::open(
        &config.layout,
        Arc::new(sequencer),
        TidGenerator::new(),
        WriteSettings {
            accepting_imports: config.accepting_imports,
            max_import_size: config.max_import_size,
            backfill_limit_ms: i64::try_from(config.repo_backfill_limit.as_millis())
                .unwrap_or(i64::MAX),
            max_subscription_buffer: config.max_subscription_buffer,
            actor_store_cache_size: config.actor_store_cache_size,
        },
    )
    .map_err(storage)?;
    let blobs = config
        .blob_store(&tokio::runtime::Handle::current())
        .map_err(|error| ServeError::Storage(format!("blob store: {error}")))?;
    let state = AppState::new(
        server,
        ReadPool::new(config.layout.clone(), config.actor_store_cache_size),
        blobs,
        resolver,
        http,
        Some(writes),
        now_ms,
    );
    open_audit_log(config, &state)?;
    Ok(state)
}

/// Takes the lock file of the data directory, and refuses when another
/// server of this kind holds it. The Reference takes no such lock: against
/// it the runbook stops one server before it starts the other.
pub fn lock_data_directory(config: &Config) -> Result<std::fs::File, ServeError> {
    let dir = config
        .layout
        .account_db
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(dir)?;
    let path = dir.join("linkjar-pds.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)?;
    file.try_lock().map_err(|_| {
        ServeError::Storage(format!(
            "another server holds {}; one server serves a data directory",
            path.display()
        ))
    })?;
    Ok(file)
}

/// Starts the private metrics listener (SPEC §14.4): `GET /metrics` in the
/// Prometheus text format. It is a listener of its own, so that the
/// public port never serves it.
async fn serve_metrics(address: SocketAddr, state: AppState) -> Result<(), ServeError> {
    let listener = TcpListener::bind(address).await?;
    tracing::info!(address = %listener.local_addr()?, "serving metrics");
    let app = axum::Router::new().route(
        "/metrics",
        axum::routing::get(move || {
            let state = state.clone();
            async move {
                let text = tokio::task::spawn_blocking(move || pds_xrpc::metrics::render(&state))
                    .await
                    .unwrap_or_default();
                (
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "text/plain; version=0.0.4; charset=utf-8",
                    )],
                    text,
                )
            }
        }),
    );
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::error!(%error, "the metrics listener stopped");
        }
    });
    Ok(())
}

/// The routes of the ops listener (SPEC §22.1): the methods of
/// `pds_xrpc::ops`, and `internal.ops.verify`, which this crate has.
pub fn ops_router(state: AppState, config: &Config) -> axum::Router {
    let config = Arc::new(config.clone());
    axum::Router::new()
        .route(
            "/xrpc/internal.ops.verify",
            axum::routing::get(
                move |axum::extract::State(state): axum::extract::State<AppState>,
                      request: axum::extract::Request| {
                    let config = Arc::clone(&config);
                    async move { ops_verify(state, config, request).await }
                },
            ),
        )
        .fallback(pds_xrpc::ops::serve)
        .with_state(state)
}

/// Starts the private ops listener.
async fn serve_ops(
    address: SocketAddr,
    state: AppState,
    config: &Config,
) -> Result<(), ServeError> {
    let listener = TcpListener::bind(address).await?;
    tracing::info!(address = %listener.local_addr()?, "serving the ops listener");
    let app = ops_router(state, config);
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::error!(%error, "the ops listener stopped");
        }
    });
    Ok(())
}

/// `internal.ops.verify`: the checks of the `verify` command on the data
/// directory that the server serves (SPEC §18.2). It writes nothing.
async fn ops_verify(
    state: AppState,
    config: Arc<Config>,
    request: axum::extract::Request,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let headers = request.headers().clone();
    let path = request.uri().path().to_owned();
    let allowed = pds_xrpc::ops::authenticate(&state, request.method(), &path, &headers).await;
    let response = if let Err(refusal) = allowed {
        *refusal
    } else {
        let blobs = state.clone();
        let report =
            tokio::task::spawn_blocking(move || crate::verify::verify_with(&config, blobs.blobs()))
                .await;
        match report {
            Ok(Ok(report)) => {
                pds_xrpc::error::json_response(axum::http::StatusCode::OK, &report.to_json())
            }
            Ok(Err(error)) => {
                tracing::error!(%error, "verify failed on the ops listener");
                axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
            Err(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    };
    pds_xrpc::ops::finish(&state, &headers, response)
}

/// Opens the audit log of the data directory (SPEC §20). Without a key the
/// log chains its entries and signs no checkpoint.
fn open_audit_log(config: &Config, state: &AppState) -> Result<(), ServeError> {
    let signer = match &config.audit_key {
        Some(seed) => Some(pds_audit::Signer::from_seed(seed).ok_or_else(|| {
            ServeError::Storage("the audit checkpoint key is not an Ed25519 seed".to_owned())
        })?),
        None => None,
    };
    let audit = pds_xrpc::audit::AuditLog::open_in(&config.layout, signer)
        .map_err(|error| ServeError::Storage(format!("audit log: {error}")))?;
    state.set_audit(audit);
    Ok(())
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
    for name in &config.unread_variables {
        tracing::warn!(variable = %name, "the variable is set and this version does not read it");
    }
    if let Some(setting) = &config.fail_points {
        tracing::warn!(%setting, "fail points are armed");
        pds_storage::fail::arm_from(setting)?;
    }
    // The binary applied it for every command; a server run in process
    // (the tests, the simulation) arrives here directly.
    if config.disable_wal_auto_checkpoint {
        pds_storage::io::disable_wal_auto_checkpoint();
    }
    // One server for a data directory (SPEC §8.3, §18.4). The lock goes
    // when the process ends, however it ends.
    let _lock = lock_data_directory(config)?;
    // An entryway says what its member host must be configured with.
    if let Some(host) = &config.member_host
        && let Ok(key) = pds_identity::Keypair::from_secret_bytes(
            pds_identity::Algorithm::Secp256k1,
            &host.signing_key,
        )
    {
        tracing::info!(
            entryway_did = %config.service_did,
            jwt_verify_key_k256_public_key_hex =
                %data_encoding::HEXLOWER.encode(&key.public_key().to_compressed()),
            member_host = %host.url,
            "serving as an entryway; the member host needs these two values"
        );
    }
    // On Postgres one process appends to the event log (SPEC §21.2).
    let _lease = match &config.layout.shared {
        Some(tier) => Some(
            tier.sequencer_lease()
                .map_err(|error| ServeError::Storage(error.to_string()))?
                .ok_or_else(|| {
                    ServeError::Storage(
                        "another server holds the lease of the event log in this Postgres database"
                            .to_owned(),
                    )
                })?,
        ),
        None => None,
    };
    create_missing_databases(config)?;
    let state = app_state(config)?;
    // Commits that were in flight at the last stop are settled before the
    // first request (SPEC §7.4).
    let repairing = state.clone();
    let repaired = tokio::task::spawn_blocking(move || pds_xrpc::write::repair(&repairing))
        .await
        .map_err(|error| ServeError::Storage(error.to_string()))?
        .map_err(|error| ServeError::Storage(error.message))?;
    if repaired != pds_xrpc::write::Repaired::default() {
        tracing::warn!(
            settled = repaired.settled,
            synced = repaired.synced,
            "settled the commits that were in flight at the last stop"
        );
    }
    // An entryway keeps no repository and so no upload: its member host
    // collects them.
    if config.member_host.is_none() {
        tokio::spawn(collect_uploads(state.clone()));
    }
    if let Some(address) = config.metrics_address {
        serve_metrics(address, state.clone()).await?;
    }
    if let Some(address) = config.ops_address {
        serve_ops(address, state.clone(), config).await?;
    }
    {
        // A checkpoint every five minutes (SPEC §20.3). The first tick of
        // an interval is immediate: it covers what a stop left open.
        let state = state.clone();
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(pds_xrpc::audit::CHECKPOINT_INTERVAL);
            loop {
                ticks.tick().await;
                pds_xrpc::audit::checkpoint(&state).await;
            }
        });
    }
    if state.config().profile.produces_notices() {
        let state = state.clone();
        tokio::spawn(async move {
            // One dispatcher for each process (SPEC §12.6).
            let mut ticks = tokio::time::interval(pds_xrpc::oauth::notices::INTERVAL);
            loop {
                ticks.tick().await;
                pds_xrpc::oauth::notices::dispatch_once(&state).await;
            }
        });
    }
    let listener = bind(config.port)?;
    let address = listener.local_addr()?;
    tracing::info!(%address, did = %config.service_did, "serving");
    let served = serve_until(
        listener,
        state.clone(),
        pds_sidecar::process::shutdown(),
        SHUTDOWN_LIMIT,
    )
    .await;
    // The entries since the last checkpoint are signed before the process
    // ends.
    pds_xrpc::audit::checkpoint(&state).await;
    served
}

/// How often the server looks for uploads that no record references.
const UPLOAD_SWEEP: Duration = Duration::from_mins(10);

/// Collects unreferenced uploads for as long as the server runs
/// (SPEC §6.4). A failed sweep is logged, and the next one runs.
async fn collect_uploads(state: AppState) {
    let mut sweep = tokio::time::interval(UPLOAD_SWEEP);
    sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        sweep.tick().await;
        let sweeping = state.clone();
        match tokio::task::spawn_blocking(move || pds_xrpc::write::collect_uploads(&sweeping)).await
        {
            Ok(Ok(0)) => {}
            Ok(Ok(collected)) => tracing::info!(collected, "collected unreferenced uploads"),
            Ok(Err(error)) => {
                tracing::error!(error = %error.message, "the upload sweep failed");
            }
            Err(error) => tracing::error!(%error, "the upload sweep failed"),
        }
    }
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
    let server = axum::serve(
        listener,
        pds_xrpc::router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
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
