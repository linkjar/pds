//! What every handler shares.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use pds_blob::BlobStore;
use pds_identity::DidResolver;
use pds_sequencer::Sequencer;
use pds_storage::{AccountDb, ActorDb, DataLayout, OpsDb, StorageError};
use pds_types::TidGenerator;

use crate::auth::AuthConfig;
use crate::error::XrpcError;

/// The part of the configuration that the HTTP surface reads.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// `PDS_HOSTNAME`.
    pub hostname: String,
    /// `PDS_VERSION`, reported by the health route.
    pub version: Option<String>,
    /// `PDS_SERVICE_HANDLE_DOMAINS`: hosted handle suffixes, each with its
    /// leading dot.
    pub handle_domains: Vec<String>,
    /// `PDS_INVITE_REQUIRED`.
    pub invite_required: bool,
    /// `PDS_BLOB_UPLOAD_LIMIT` in bytes.
    pub blob_upload_limit: u64,
    /// `PDS_PRIVACY_POLICY_URL`.
    pub privacy_policy_url: Option<String>,
    /// `PDS_TERMS_OF_SERVICE_URL`.
    pub terms_of_service_url: Option<String>,
    /// `PDS_CONTACT_EMAIL_ADDRESS`.
    pub contact_email: Option<String>,
    /// `PDS_BSKY_APP_VIEW_URL`, without a trailing slash.
    pub appview_url: Option<String>,
    /// `PDS_BSKY_APP_VIEW_DID`.
    pub appview_did: Option<String>,
    /// `PDS_MOD_SERVICE_URL` and `PDS_MOD_SERVICE_DID`.
    pub mod_service: Option<(String, String)>,
    /// `PDS_REPORT_SERVICE_URL` and `PDS_REPORT_SERVICE_DID`.
    pub report_service: Option<(String, String)>,
    /// The bound on one request to the AppView.
    pub proxy_timeout: Duration,
    /// How credentials are checked.
    pub auth: AuthConfig,
    /// What the server needs to create an identity. Without it no account
    /// is created.
    pub identity: Option<IdentityConfig>,
    /// How the server sends mail and what its mails look like.
    pub mail: crate::mail::MailConfig,
    /// The settings of the rate limits.
    pub rate_limits: crate::ratelimit::RateLimitConfig,
    /// The repository that publishes the audit checkpoints (SPEC §20.3):
    /// the DID of an account of the operator on this server.
    pub audit_repo: Option<String>,
    /// The operator's behaviour on top of the protocol (SPEC §12).
    pub profile: Arc<dyn pds_ext::Profile>,
    /// The OAuth settings.
    pub oauth: crate::oauth::OAuthConfig,
    /// `PDS_OPS_CLIENT_IDS`: the OAuth clients whose sessions the ops
    /// listener takes (SPEC §22.1).
    pub ops_client_ids: Vec<String>,
    /// `PDS_MCP_CLIENT_IDS`: the OAuth clients that are MCP sidecars of
    /// this server (SPEC §22.2). A write of one is an audit entry.
    pub mcp_client_ids: Vec<String>,
    /// `PDS_BSKY_APP_VIEW_CDN_URL_PATTERN`: the URL of an image, with `%s`
    /// for the preset, the DID and the CID.
    pub appview_cdn_url_pattern: Option<String>,
    /// The entryway of this member host (SPEC §21.3). `None`: the server
    /// keeps its accounts itself.
    pub entryway: Option<Entryway>,
    /// The member host of this entryway (SPEC §21.3). `None`: the server
    /// keeps its repositories itself.
    pub member_host: Option<MemberHost>,
}

/// The member host of an entryway: the server that keeps the
/// repositories of its accounts.
#[derive(Clone)]
pub struct MemberHost {
    /// `PDS_MEMBER_URL`: the public address, which a DID document names.
    pub url: String,
    /// `PDS_MEMBER_INTERNAL_URL`: where the entryway reaches the host.
    pub internal_url: String,
    /// `PDS_MEMBER_ADMIN_PASSWORD`: the admin password of the host.
    pub admin_password: String,
}

impl std::fmt::Debug for MemberHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberHost")
            .field("url", &self.url)
            .field("internal_url", &self.internal_url)
            .finish_non_exhaustive()
    }
}

/// The entryway of a member host: the service that owns the credentials
/// and the sessions of its accounts.
#[derive(Clone)]
pub struct Entryway {
    /// `PDS_ENTRYWAY_URL`.
    pub url: String,
    /// `PDS_ENTRYWAY_DID`.
    pub did: String,
    /// `PDS_ENTRYWAY_PLC_ROTATION_KEY`: the `did:key` that signs the PLC
    /// operation of a new account.
    pub plc_rotation_key: String,
    /// `PDS_ENTRYWAY_ADMIN_TOKEN`: the admin password of the entryway,
    /// for the admin methods the server passes on.
    pub admin_token: Option<String>,
}

impl std::fmt::Debug for Entryway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entryway")
            .field("url", &self.url)
            .field("did", &self.did)
            .finish_non_exhaustive()
    }
}

/// What the server needs to create and change `did:plc` identities.
#[derive(Clone)]
pub struct IdentityConfig {
    /// `PDS_DID_PLC_URL`, without a trailing slash.
    pub plc_url: String,
    /// The URL the server is reached at: the PDS endpoint of its accounts.
    pub public_url: String,
    /// `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX`, as bytes.
    pub rotation_key: Vec<u8>,
    /// `PDS_RECOVERY_DID_KEY`.
    pub recovery_did_key: Option<String>,
    /// `PDS_DEV_MODE`: handles under `.test` are taken.
    pub dev_mode: bool,
}

impl std::fmt::Debug for IdentityConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityConfig")
            .field("plc_url", &self.plc_url)
            .field("public_url", &self.public_url)
            .field("recovery_did_key", &self.recovery_did_key)
            .finish_non_exhaustive()
    }
}

impl ServerConfig {
    /// The public origin of the server: the OAuth issuer.
    pub fn public_url(&self) -> String {
        self.identity.as_ref().map_or_else(
            || format!("https://{}", self.hostname),
            |identity| identity.public_url.clone(),
        )
    }

    /// Whether `handle` is under a hosted suffix, or is a suffix without its
    /// dot.
    pub fn is_hosted_handle(&self, handle: &str) -> bool {
        self.handle_domains.iter().any(|domain| {
            handle.ends_with(domain.as_str()) || Some(handle) == domain.strip_prefix('.')
        })
    }
}

/// The settings of the write path and of the firehose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteSettings {
    /// `PDS_ACCEPTING_REPO_IMPORTS`.
    pub accepting_imports: bool,
    /// `PDS_MAX_REPO_IMPORT_SIZE` in bytes, when one is set.
    pub max_import_size: Option<u64>,
    /// `PDS_REPO_BACKFILL_LIMIT_MS`.
    pub backfill_limit_ms: i64,
    /// `PDS_MAX_SUBSCRIPTION_BUFFER`.
    pub max_subscription_buffer: u32,
    /// `PDS_ACTOR_STORE_CACHE_SIZE`: how many actor stores stay open for
    /// the next write.
    pub actor_store_cache_size: usize,
}

/// What the write path shares: the event log, the notes of commits in
/// flight, the revision clock and one lock per repository.
pub struct WriteState {
    sequencer: Arc<Sequencer>,
    ops: Mutex<OpsDb>,
    accounts: Mutex<AccountDb>,
    tids: TidGenerator,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Write connections to actor stores, the one used last at the back
    /// (SPEC §8.3).
    stores: Mutex<Vec<(String, ActorDb)>>,
    settings: WriteSettings,
    random: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl std::fmt::Debug for WriteState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteState")
            .field("sequencer", &self.sequencer)
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl WriteState {
    /// Opens the databases the write path writes besides the actor stores.
    /// `tids` is the revision clock (SPEC §3.2).
    pub fn open(
        layout: &DataLayout,
        sequencer: Arc<Sequencer>,
        tids: TidGenerator,
        settings: WriteSettings,
    ) -> Result<Self, StorageError> {
        Ok(Self {
            sequencer,
            ops: Mutex::new(OpsDb::open(&layout.ops_db)?),
            accounts: Mutex::new(layout.accounts_writable()?),
            tids,
            locks: Mutex::new(HashMap::new()),
            stores: Mutex::new(Vec::new()),
            settings,
            random: Box::new(rand::random::<u64>),
        })
    }

    /// The same state with another source of random numbers (SPEC §3.2).
    /// The simulation passes one that its seed drives.
    #[must_use]
    pub fn with_random(mut self, random: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        self.random = Box::new(random);
        self
    }

    /// A random number. The names of staged uploads come from here.
    pub fn random(&self) -> u64 {
        (self.random)()
    }

    /// The event log.
    pub fn sequencer(&self) -> &Arc<Sequencer> {
        &self.sequencer
    }

    /// The settings.
    pub fn settings(&self) -> &WriteSettings {
        &self.settings
    }

    /// The revision clock.
    pub fn tids(&self) -> &TidGenerator {
        &self.tids
    }

    /// Runs `call` with `ops.sqlite`.
    pub fn with_ops<T>(&self, call: impl FnOnce(&OpsDb) -> T) -> T {
        call(&self.ops.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Runs `call` with the writable connection to `account.sqlite`.
    pub fn with_accounts<T>(&self, call: impl FnOnce(&mut AccountDb) -> T) -> T {
        call(&mut self.accounts.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Drops the connection kept for the actor store of `did`: the store is
    /// about to be removed.
    pub fn forget_store(&self, did: &str) {
        self.stores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(owner, _)| owner != did);
    }

    /// The write connection to the actor store of `did`: the one kept from
    /// the last write, or a new one. The caller holds the repository lock
    /// and gives the connection back with [`WriteState::keep_store`].
    pub fn take_store(&self, did: &str, layout: &DataLayout) -> Result<ActorDb, StorageError> {
        let kept = {
            let mut stores = self.stores.lock().unwrap_or_else(PoisonError::into_inner);
            stores
                .iter()
                .position(|(owner, _)| owner == did)
                .map(|index| stores.remove(index).1)
        };
        match kept {
            Some(store) => Ok(store),
            None => ActorDb::open_writable(&layout.actor_db(did)),
        }
    }

    /// Keeps a write connection for the next write of `did`. The store
    /// that was used longest ago is closed when the cache is full.
    pub fn keep_store(&self, did: &str, store: ActorDb) {
        if self.settings.actor_store_cache_size == 0 {
            return;
        }
        let mut stores = self.stores.lock().unwrap_or_else(PoisonError::into_inner);
        stores.push((did.to_owned(), store));
        let closed =
            (stores.len() > self.settings.actor_store_cache_size).then(|| stores.remove(0));
        drop(stores);
        // The connection closes outside the lock: a close can checkpoint.
        drop(closed);
    }

    /// The lock of one repository: one writer at a time (SPEC §8.3).
    pub fn repo_lock(&self, did: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        // A lock nobody holds or waits for is dropped, so the map does not
        // grow with the number of accounts that ever wrote.
        if locks.len() > 1024 {
            locks.retain(|_, lock| Arc::strong_count(lock) > 1);
        }
        Arc::clone(locks.entry(did.to_owned()).or_default())
    }
}

/// The shared state of the server.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    config: ServerConfig,
    layout: DataLayout,
    blobs: Option<BlobStore>,
    resolver: DidResolver,
    http: reqwest::Client,
    writes: Option<WriteState>,
    limiter: crate::ratelimit::Limiter,
    oauth: crate::oauth::OAuthState,
    audit: std::sync::OnceLock<crate::audit::AuditLog>,
    metrics: crate::metrics::Metrics,
    subscribers: crate::subscribers::Subscribers,
    now_ms: Box<dyn Fn() -> i64 + Send + Sync>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("config", &self.inner.config)
            .field("layout", &self.inner.layout)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Builds the state. `now_ms` is the clock (SPEC §3.2). Without
    /// `writes` the server is read-only: it serves the reads of the data
    /// directory and has no handler for a write.
    pub fn new(
        config: ServerConfig,
        layout: DataLayout,
        blobs: Option<BlobStore>,
        resolver: DidResolver,
        http: reqwest::Client,
        writes: Option<WriteState>,
        now_ms: impl Fn() -> i64 + Send + Sync + 'static,
    ) -> Self {
        let oauth = crate::oauth::OAuthState::new(&config.oauth);
        Self {
            inner: Arc::new(Inner {
                oauth,
                audit: std::sync::OnceLock::new(),
                metrics: crate::metrics::Metrics::default(),
                subscribers: crate::subscribers::Subscribers::default(),
                config,
                layout,
                blobs,
                resolver,
                http,
                writes,
                limiter: crate::ratelimit::Limiter::default(),
                now_ms: Box::new(now_ms),
            }),
        }
    }

    /// The live subscribers of the firehose.
    pub fn subscribers(&self) -> &crate::subscribers::Subscribers {
        &self.inner.subscribers
    }

    /// The buckets of the rate limits.
    pub fn limiter(&self) -> &crate::ratelimit::Limiter {
        &self.inner.limiter
    }

    /// What the authorization server keeps in memory.
    pub fn oauth(&self) -> &crate::oauth::OAuthState {
        &self.inner.oauth
    }

    /// What the server counts as it serves.
    pub fn metrics(&self) -> &crate::metrics::Metrics {
        &self.inner.metrics
    }

    /// Gives the server its audit log. The first call counts.
    pub fn set_audit(&self, log: crate::audit::AuditLog) {
        let _ = self.inner.audit.set(log);
    }

    /// The audit log, when the server has one.
    pub fn audit(&self) -> Option<&crate::audit::AuditLog> {
        self.inner.audit.get()
    }

    /// The configuration.
    pub fn config(&self) -> &ServerConfig {
        &self.inner.config
    }

    /// The data directory.
    pub fn layout(&self) -> &DataLayout {
        &self.inner.layout
    }

    /// The disk blob store, when one is configured.
    pub fn blobs(&self) -> Option<&BlobStore> {
        self.inner.blobs.as_ref()
    }

    /// The DID resolver.
    pub fn resolver(&self) -> &DidResolver {
        &self.inner.resolver
    }

    /// The client for outbound requests.
    pub fn http(&self) -> &reqwest::Client {
        &self.inner.http
    }

    /// The write path, or `None` for a read-only server.
    pub fn writes(&self) -> Option<&WriteState> {
        self.inner.writes.as_ref()
    }

    /// The time in milliseconds since the Unix epoch.
    pub fn now_ms(&self) -> i64 {
        (self.inner.now_ms)()
    }

    /// The time in seconds since the Unix epoch.
    pub fn now_s(&self) -> i64 {
        (self.inner.now_ms)() / 1000
    }

    /// Runs a storage call on the blocking pool.
    pub async fn blocking<T, F>(&self, call: F) -> Result<T, XrpcError>
    where
        T: Send + 'static,
        F: FnOnce(&Self) -> Result<T, XrpcError> + Send + 'static,
    {
        let state = self.clone();
        tokio::task::spawn_blocking(move || call(&state)).await?
    }
}
