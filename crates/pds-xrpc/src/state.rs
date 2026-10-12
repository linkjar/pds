//! What every handler shares.

use std::sync::Arc;
use std::time::Duration;

use pds_blob::DiskBlobStore;
use pds_identity::DidResolver;
use pds_storage::DataLayout;

use crate::auth::AuthConfig;
use crate::error::XrpcError;

/// The part of the configuration that the HTTP surface reads.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// `PDS_VERSION`, reported by the health route.
    pub version: Option<String>,
    /// The service DID.
    pub service_did: String,
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
    /// The bound on one request to the AppView.
    pub proxy_timeout: Duration,
    /// How credentials are checked.
    pub auth: AuthConfig,
}

impl ServerConfig {
    /// Whether `handle` is under a hosted suffix, or is a suffix without its
    /// dot.
    pub fn is_hosted_handle(&self, handle: &str) -> bool {
        self.handle_domains.iter().any(|domain| {
            handle.ends_with(domain.as_str()) || Some(handle) == domain.strip_prefix('.')
        })
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
    blobs: Option<DiskBlobStore>,
    resolver: DidResolver,
    http: reqwest::Client,
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
    /// Builds the state. `now_ms` is the clock (SPEC §3.2).
    pub fn new(
        config: ServerConfig,
        layout: DataLayout,
        blobs: Option<DiskBlobStore>,
        resolver: DidResolver,
        http: reqwest::Client,
        now_ms: impl Fn() -> i64 + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                layout,
                blobs,
                resolver,
                http,
                now_ms: Box::new(now_ms),
            }),
        }
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
    pub fn blobs(&self) -> Option<&DiskBlobStore> {
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
