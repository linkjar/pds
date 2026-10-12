//! DID resolution with a cache (`docs/SPEC.md` §9.3).
//!
//! A `did:plc` document comes from the PLC directory and a `did:web`
//! document from the host's `/.well-known/did.json`. A cached document is
//! served until it is older than the maximum lifetime; once it is older
//! than the stale lifetime it is still served, and refreshed in the
//! background. These are the Reference's rules
//! (`packages/identity/src/did/base-resolver.ts` at the pin). At most one
//! background refresh runs for a DID at a time.
//!
//! The cache is synchronous, so the resolver calls it on the blocking pool
//! (SPEC §3.2).
//!
//! # Not done: safe fetch
//!
//! SPEC §15.1 requires every outbound request to resolve the host and to
//! reject loopback, private, link-local and multicast addresses. The
//! resolver does not do this. It checks the syntax of a `did:web` host name
//! ([`crate::method`]) and not the address the name resolves to, so it
//! fetches the document of a `did:web` whose name resolves to such an
//! address. The resolver bounds the time and the size of a fetch. It does
//! not forbid redirects: that is a property of the client the caller passes
//! to [`DidResolver::new`].

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use pds_types::Did;
use serde_json::Value;

use crate::did_doc::{DidDocError, DidDocument};
use crate::method::{DidError, SupportedDid};

/// The largest DID document the resolver reads.
pub const MAX_DID_DOC_LEN: usize = 512 * 1024;

/// A store of DID documents by DID. `did_cache.sqlite` is one.
///
/// A cache that fails answers as if it held nothing: resolution then goes
/// to the network, as in the Reference.
///
/// The calls may block: the resolver runs them on the blocking pool.
pub trait DidCache: Send + Sync {
    /// The cached document as JSON text, and when it was stored in
    /// milliseconds since the Unix epoch.
    fn get(&self, did: &str) -> Option<(String, i64)>;
    /// Stores a document.
    fn put(&self, did: &str, doc: &str, now_ms: i64);
    /// Removes a document.
    fn clear(&self, did: &str);
}

/// How the resolver reaches the directory and how long documents live.
#[derive(Debug, Clone)]
pub struct ResolverConfig {
    /// The PLC directory, without a trailing slash (`PDS_DID_PLC_URL`).
    pub plc_url: String,
    /// The bound on one fetch (`PDS_ID_RESOLVER_TIMEOUT`).
    pub timeout: Duration,
    /// After this age a cached document is refreshed in the background
    /// (`PDS_DID_CACHE_STALE_TTL`).
    pub stale_ttl: Duration,
    /// After this age a cached document is not served
    /// (`PDS_DID_CACHE_MAX_TTL`).
    pub max_ttl: Duration,
    /// Admits `did:web:localhost` (`PDS_DEV_MODE`).
    pub dev_mode: bool,
}

/// Resolution failed.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// The DID has no document.
    #[error("Could not resolve DID: {0}")]
    NotFound(String),
    /// The DID is of a method or form this server does not resolve.
    #[error("Unsupported DID: {0}")]
    Unsupported(#[from] DidError),
    /// The directory or host could not be reached, or answered with an
    /// error.
    #[error("Could not fetch the DID document of {did}: {reason}")]
    Fetch {
        /// The DID.
        did: String,
        /// What went wrong.
        reason: String,
    },
    /// The answer is not a DID document about this DID.
    #[error("Poorly formatted DID document of {did}: {source}")]
    Document {
        /// The DID.
        did: String,
        /// What is wrong with it.
        source: DidDocError,
    },
}

/// Resolves DIDs to their documents.
#[derive(Clone)]
pub struct DidResolver {
    inner: Arc<Inner>,
}

struct Inner {
    config: ResolverConfig,
    http: reqwest::Client,
    cache: Option<Arc<dyn DidCache>>,
    now_ms: Box<dyn Fn() -> i64 + Send + Sync>,
    /// The DIDs with a background refresh in flight.
    refreshing: Mutex<HashSet<String>>,
}

/// Marks a DID as being refreshed. Dropping the value removes the mark, so
/// the mark goes on every exit of the refresh task, a panic included.
struct Refreshing {
    inner: Arc<Inner>,
    did: String,
}

impl Refreshing {
    /// Marks `did`, or returns `None` when a refresh of it is in flight.
    fn begin(inner: &Arc<Inner>, did: &Did) -> Option<Self> {
        let did = did.to_string();
        let mut marked = inner
            .refreshing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        marked.insert(did.clone()).then(|| Self {
            inner: Arc::clone(inner),
            did,
        })
    }
}

impl Drop for Refreshing {
    fn drop(&mut self) {
        self.inner
            .refreshing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.did);
    }
}

impl std::fmt::Debug for DidResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DidResolver")
            .field("config", &self.inner.config)
            .field("cache", &self.inner.cache.is_some())
            .finish_non_exhaustive()
    }
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

impl DidResolver {
    /// A resolver that fetches with `http`. `now_ms` is the clock: time
    /// enters the server through it (SPEC §3.2).
    ///
    /// `http` must not follow redirects: build it with
    /// `reqwest::redirect::Policy::none()`. The resolver does not check
    /// this, and it does not check the address a host resolves to (see the
    /// module documentation).
    pub fn new(
        config: ResolverConfig,
        http: reqwest::Client,
        cache: Option<Arc<dyn DidCache>>,
        now_ms: impl Fn() -> i64 + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                http,
                cache,
                now_ms: Box::new(now_ms),
                refreshing: Mutex::new(HashSet::new()),
            }),
        }
    }

    /// The document of `did`, or `None` when the DID has none. With
    /// `force_refresh` the cache is not consulted.
    pub async fn resolve(
        &self,
        did: &Did,
        force_refresh: bool,
    ) -> Result<Option<Value>, ResolveError> {
        if !force_refresh && let Some(doc) = self.cached(did).await {
            return Ok(Some(doc));
        }
        let fetched = self.fetch(did).await?;
        if let Some(cache) = &self.inner.cache {
            let cache = Arc::clone(cache);
            let key = did.to_string();
            let text = fetched.as_ref().map(Value::to_string);
            let now = (self.inner.now_ms)();
            let stored = tokio::task::spawn_blocking(move || match text {
                Some(text) => cache.put(&key, &text, now),
                None => cache.clear(&key),
            })
            .await;
            if let Err(error) = stored {
                tracing::warn!(%did, %error, "storing in the DID cache failed");
            }
        }
        Ok(fetched)
    }

    /// The document of `did`. A DID without one is an error.
    pub async fn ensure_resolve(
        &self,
        did: &Did,
        force_refresh: bool,
    ) -> Result<Value, ResolveError> {
        self.resolve(did, force_refresh)
            .await?
            .ok_or_else(|| ResolveError::NotFound(did.to_string()))
    }

    /// A cached document that is still served. A stale one starts a
    /// refresh that does not hold up the caller. A cache read that fails is
    /// a miss.
    async fn cached(&self, did: &Did) -> Option<Value> {
        let cache = Arc::clone(self.inner.cache.as_ref()?);
        let key = did.to_string();
        let (text, updated_at) = tokio::task::spawn_blocking(move || cache.get(&key))
            .await
            .ok()??;
        let age = (self.inner.now_ms)().saturating_sub(updated_at);
        if age > millis(self.inner.config.max_ttl) {
            return None;
        }
        let doc: Value = serde_json::from_str(&text).ok()?;
        if age > millis(self.inner.config.stale_ttl) {
            self.refresh_in_background(did);
        }
        Some(doc)
    }

    /// Starts a refresh of `did`, unless one is in flight.
    fn refresh_in_background(&self, did: &Did) {
        let Some(mark) = Refreshing::begin(&self.inner, did) else {
            return;
        };
        let resolver = self.clone();
        let did = did.clone();
        tokio::spawn(async move {
            let _mark = mark;
            if let Err(error) = resolver.resolve(&did, true).await {
                tracing::warn!(%did, %error, "refreshing the DID cache failed");
            }
        });
    }

    async fn fetch(&self, did: &Did) -> Result<Option<Value>, ResolveError> {
        let url = match SupportedDid::classify(did.clone(), self.inner.config.dev_mode)? {
            SupportedDid::Plc(did) => {
                // A valid did:plc has no character that a URL path escapes
                // besides the colons.
                format!(
                    "{}/{}",
                    self.inner.config.plc_url,
                    did.as_str().replace(':', "%3A")
                )
            }
            web @ SupportedDid::Web { .. } => web.web_document_url().unwrap_or_default(),
        };
        let fail = |reason: String| ResolveError::Fetch {
            did: did.to_string(),
            reason,
        };
        let response = self
            .inner
            .http
            .get(&url)
            .timeout(self.inner.config.timeout)
            .send()
            .await
            .map_err(|e| fail(e.to_string()))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(fail(format!("status {status}")));
        }
        let mut body = Vec::new();
        let mut response = response;
        while let Some(chunk) = response.chunk().await.map_err(|e| fail(e.to_string()))? {
            if body.len() + chunk.len() > MAX_DID_DOC_LEN {
                return Err(fail(format!(
                    "the document is over {MAX_DID_DOC_LEN} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        let document = |source| ResolveError::Document {
            did: did.to_string(),
            source,
        };
        DidDocument::from_json_for(&body, MAX_DID_DOC_LEN, did).map_err(document)?;
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|e| document(DidDocError::Parse(e.to_string())))
    }
}

/// The handle a raw DID document claims.
///
/// It is the first `alsoKnownAs` entry that starts with `at://`, without
/// that prefix. This is the Reference's reading for `describeRepo`; it does
/// not check that the value is a handle.
pub fn claimed_handle(doc: &Value) -> Option<&str> {
    doc.get("alsoKnownAs")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|entry| entry.strip_prefix("at://"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_claimed_handle_is_the_first_at_entry() {
        let doc = serde_json::json!({"alsoKnownAs": ["https://example.com", "at://alice.test", "at://bob.test"]});
        assert_eq!(claimed_handle(&doc), Some("alice.test"));
        assert_eq!(claimed_handle(&serde_json::json!({})), None);
    }
}
