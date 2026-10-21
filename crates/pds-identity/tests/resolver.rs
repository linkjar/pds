//! The resolver against a cache in memory, an injected clock and a local
//! HTTP server: the two lifetimes of a cached document, `force_refresh`,
//! the answers of the directory, and the one refresh in flight per DID.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pds_identity::resolver::MAX_DID_DOC_LEN;
use pds_identity::{DidCache, DidResolver, ResolveError, ResolverConfig};
use pds_types::Did;
use serde_json::{Value, json};

const DID: &str = "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "did:plc:bbbbbbbbbbbbbbbbbbbbbbbb";
/// Nothing listens on the discard port, so a fetch from here fails.
const CLOSED: &str = "http://127.0.0.1:9";
const STALE_MS: i64 = 1_000;
const MAX_MS: i64 = 10_000;

/// A cache in memory that counts its calls.
#[derive(Default)]
struct MemoryCache {
    docs: Mutex<HashMap<String, (String, i64)>>,
    puts: AtomicUsize,
    clears: AtomicUsize,
}

impl MemoryCache {
    fn with(did: &str, doc: &Value, stored_at: i64) -> Arc<Self> {
        let cache = Self::default();
        cache
            .docs
            .lock()
            .unwrap()
            .insert(did.to_owned(), (doc.to_string(), stored_at));
        Arc::new(cache)
    }

    fn entry(&self, did: &str) -> Option<(String, i64)> {
        self.docs.lock().unwrap().get(did).cloned()
    }
}

impl DidCache for MemoryCache {
    fn get(&self, did: &str) -> Option<(String, i64)> {
        self.entry(did)
    }

    fn put(&self, did: &str, doc: &str, now_ms: i64) {
        self.puts.fetch_add(1, Ordering::SeqCst);
        self.docs
            .lock()
            .unwrap()
            .insert(did.to_owned(), (doc.to_owned(), now_ms));
    }

    fn clear(&self, did: &str) {
        self.clears.fetch_add(1, Ordering::SeqCst);
        self.docs.lock().unwrap().remove(did);
    }
}

fn document(did: &str, handle: &str) -> Value {
    json!({
        "@context": ["https://www.w3.org/ns/did/v1"],
        "id": did,
        "alsoKnownAs": [format!("at://{handle}")],
    })
}

fn did() -> Did {
    Did::parse(DID).unwrap()
}

/// A resolver with a clock the test moves.
fn resolver(plc_url: &str, cache: &Arc<MemoryCache>, clock: &Arc<AtomicI64>) -> DidResolver {
    resolver_with_mode(plc_url, cache, clock, false)
}

fn resolver_with_mode(
    plc_url: &str,
    cache: &Arc<MemoryCache>,
    clock: &Arc<AtomicI64>,
    dev_mode: bool,
) -> DidResolver {
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let clock = Arc::clone(clock);
    let cache: Arc<dyn DidCache> = Arc::clone(cache) as _;
    DidResolver::new(
        ResolverConfig {
            plc_url: plc_url.to_owned(),
            timeout: Duration::from_secs(5),
            stale_ttl: Duration::from_millis(STALE_MS.unsigned_abs()),
            max_ttl: Duration::from_millis(MAX_MS.unsigned_abs()),
            dev_mode,
        },
        http,
        Some(cache),
        move || clock.load(Ordering::SeqCst),
    )
}

/// A local HTTP server that answers every request with one response.
struct Server {
    url: String,
    /// The request targets, in the order they arrived.
    requests: Arc<Mutex<Vec<String>>>,
    /// While false, the server holds its responses back.
    open: Arc<AtomicBool>,
}

impl Server {
    fn start(status: &str, body: Vec<u8>) -> Self {
        Self::with_body(status, |_| body)
    }

    fn with_body(status: &str, body: impl FnOnce(&str) -> Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let body = body(&url);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let open = Arc::new(AtomicBool::new(true));
        let head = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        let (seen, gate) = (Arc::clone(&requests), Arc::clone(&open));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (seen, gate) = (Arc::clone(&seen), Arc::clone(&gate));
                let (head, body) = (head.clone(), body.clone());
                std::thread::spawn(move || {
                    let mut request = Vec::new();
                    let mut byte = [0u8; 1];
                    while !request.ends_with(b"\r\n\r\n") {
                        if stream.read(&mut byte).unwrap_or(0) == 0 {
                            return;
                        }
                        request.push(byte[0]);
                    }
                    let text = String::from_utf8_lossy(&request);
                    let target = text.split(' ').nth(1).unwrap_or_default().to_owned();
                    seen.lock().unwrap().push(target);
                    while !gate.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    // The client may close before it has read everything.
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&body);
                });
            }
        });
        Self {
            url,
            requests,
            open,
        }
    }

    fn json(status: &str, body: &Value) -> Self {
        Self::start(status, body.to_string().into_bytes())
    }

    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[tokio::test]
async fn untrusted_web_hosts_are_refused_before_network_access() {
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(AtomicI64::new(0));
    let resolver = resolver(CLOSED, &cache, &clock);
    let did = Did::parse("did:web:metadata.local").unwrap();
    assert!(matches!(resolver.resolve(&did, false).await,
        Err(ResolveError::Fetch { reason, .. }) if reason.contains("not on the public internet")));
    assert_eq!(cache.puts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn development_explicitly_allows_a_local_web_document() {
    let server = Server::with_body("200 OK", |url| {
        let port = url.rsplit(':').next().unwrap();
        document(&format!("did:web:localhost%3A{port}"), "alice.test")
            .to_string()
            .into_bytes()
    });
    let port = server.url.rsplit(':').next().unwrap();
    let did = Did::parse(&format!("did:web:localhost%3A{port}")).unwrap();
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(AtomicI64::new(0));
    let production = resolver(CLOSED, &cache, &clock);
    assert!(matches!(
        production.resolve(&did, false).await,
        Err(ResolveError::Unsupported(_))
    ));
    assert_eq!(server.count(), 0);
    let development = resolver_with_mode(CLOSED, &cache, &clock, true);
    assert_eq!(
        development.resolve(&did, false).await.unwrap(),
        Some(document(did.as_str(), "alice.test"))
    );
    assert_eq!(server.count(), 1);
}

/// Waits until `done` holds, for at most five seconds.
async fn until(what: &str, done: impl Fn() -> bool) {
    for _ in 0..1000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting until {what}");
}

#[tokio::test]
async fn a_fresh_entry_is_served_without_a_fetch() {
    let doc = document(DID, "alice.test");
    let cache = MemoryCache::with(DID, &doc, 0);
    let clock = Arc::new(AtomicI64::new(STALE_MS));
    let resolver = resolver(CLOSED, &cache, &clock);
    assert_eq!(resolver.resolve(&did(), false).await.unwrap(), Some(doc));
    assert_eq!(cache.puts.load(Ordering::SeqCst), 0);
    assert_eq!(cache.clears.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_entry_older_than_the_maximum_lifetime_is_not_served() {
    let doc = document(DID, "alice.test");
    let cache = MemoryCache::with(DID, &doc, 0);
    let clock = Arc::new(AtomicI64::new(MAX_MS));
    let resolver = resolver(CLOSED, &cache, &clock);
    // At the maximum lifetime exactly, the entry is still served.
    assert_eq!(
        resolver.resolve(&did(), false).await.unwrap(),
        Some(doc.clone())
    );
    clock.store(MAX_MS + 1, Ordering::SeqCst);
    assert!(matches!(
        resolver.resolve(&did(), false).await,
        Err(ResolveError::Fetch { .. })
    ));
    // A failed fetch leaves the entry in place.
    assert_eq!(cache.entry(DID), Some((doc.to_string(), 0)));
}

#[tokio::test]
async fn force_refresh_ignores_a_fresh_entry() {
    let cached = document(DID, "old.test");
    let cache = MemoryCache::with(DID, &cached, 0);
    let clock = Arc::new(AtomicI64::new(5));
    assert!(matches!(
        resolver(CLOSED, &cache, &clock).resolve(&did(), true).await,
        Err(ResolveError::Fetch { .. })
    ));

    let fetched = document(DID, "new.test");
    let server = Server::json("200 OK", &fetched);
    let resolver = resolver(&server.url, &cache, &clock);
    assert_eq!(
        resolver.resolve(&did(), true).await.unwrap(),
        Some(fetched.clone())
    );
    // The fetch asks the directory for the DID and stores the answer with
    // the time of the clock.
    assert_eq!(
        *server.requests.lock().unwrap(),
        ["/did%3Aplc%3Aaaaaaaaaaaaaaaaaaaaaaaaa"]
    );
    assert_eq!(cache.entry(DID), Some((fetched.to_string(), 5)));
}

#[tokio::test]
async fn a_did_without_a_document_resolves_to_none_and_leaves_the_cache() {
    let cache = MemoryCache::with(DID, &document(DID, "alice.test"), 0);
    let clock = Arc::new(AtomicI64::new(MAX_MS + 1));
    let server = Server::json("404 Not Found", &json!({"message": "DID not registered"}));
    let resolver = resolver(&server.url, &cache, &clock);
    assert_eq!(resolver.resolve(&did(), false).await.unwrap(), None);
    assert_eq!(cache.entry(DID), None);
    assert_eq!(cache.clears.load(Ordering::SeqCst), 1);
    assert!(matches!(
        resolver.ensure_resolve(&did(), false).await,
        Err(ResolveError::NotFound(_))
    ));
}

#[tokio::test]
async fn a_document_about_another_did_is_refused() {
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(AtomicI64::new(0));
    let server = Server::json("200 OK", &document(OTHER, "bob.test"));
    let resolver = resolver(&server.url, &cache, &clock);
    assert!(matches!(
        resolver.resolve(&did(), false).await,
        Err(ResolveError::Document { .. })
    ));
    assert_eq!(cache.entry(DID), None);
    assert_eq!(cache.puts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_error_status_is_a_failed_fetch() {
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(AtomicI64::new(0));
    let server = Server::json("500 Internal Server Error", &json!({}));
    let resolver = resolver(&server.url, &cache, &clock);
    assert!(matches!(
        resolver.resolve(&did(), false).await,
        Err(ResolveError::Fetch { .. })
    ));
    assert_eq!(cache.clears.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_document_over_the_size_cap_is_refused() {
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(AtomicI64::new(0));
    // A valid document, padded with spaces to one byte over the cap.
    let mut body = document(DID, "alice.test").to_string().into_bytes();
    let at_cap = {
        let mut padded = body.clone();
        padded.resize(MAX_DID_DOC_LEN, b' ');
        padded
    };
    body.resize(MAX_DID_DOC_LEN + 1, b' ');

    let server = Server::start("200 OK", body);
    let over = resolver(&server.url, &cache, &clock);
    match over.resolve(&did(), false).await {
        Err(ResolveError::Fetch { reason, .. }) => assert!(reason.contains("524288"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(cache.puts.load(Ordering::SeqCst), 0);

    // The same document at the cap exactly is read.
    let server = Server::start("200 OK", at_cap);
    let at = resolver(&server.url, &cache, &clock);
    assert_eq!(
        at.resolve(&did(), false).await.unwrap(),
        Some(document(DID, "alice.test"))
    );
}

#[tokio::test]
async fn a_stale_entry_is_served_and_refreshed_once() {
    let stale = document(DID, "old.test");
    let cache = MemoryCache::with(DID, &stale, 0);
    let clock = Arc::new(AtomicI64::new(STALE_MS + 1));
    let fresh = document(DID, "new.test");
    let server = Server::json("200 OK", &fresh);
    server.open.store(false, Ordering::SeqCst);
    let resolver = resolver(&server.url, &cache, &clock);

    // Every call is answered from the cache while the directory is slow.
    for _ in 0..8 {
        assert_eq!(
            resolver.resolve(&did(), false).await.unwrap(),
            Some(stale.clone())
        );
    }
    until("the refresh reaches the directory", || server.count() >= 1).await;
    // Time for a second refresh to arrive, if one had started.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(server.count(), 1);

    server.open.store(true, Ordering::SeqCst);
    until("the refresh is stored", || {
        cache.entry(DID) == Some((fresh.to_string(), STALE_MS + 1))
    })
    .await;
    assert_eq!(
        resolver.resolve(&did(), false).await.unwrap(),
        Some(fresh.clone())
    );
    assert_eq!(server.count(), 1);

    // When the entry is stale again, a new refresh starts: the first one
    // no longer counts as in flight.
    clock.store(2 * STALE_MS + 2, Ordering::SeqCst);
    let again = Some((fresh.to_string(), 2 * STALE_MS + 2));
    for _ in 0..1000 {
        assert!(resolver.resolve(&did(), false).await.unwrap().is_some());
        if cache.entry(DID) == again {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(cache.entry(DID), again);
    assert_eq!(server.count(), 2);
}
