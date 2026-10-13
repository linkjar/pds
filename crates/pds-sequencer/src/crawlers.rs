//! Crawler notification (`docs/SPEC.md` §7.6).
//!
//! The server asks every configured host to crawl it when it sequences the
//! first event after a start, and then at most once in twenty minutes
//! (`crawlers.ts` at the pin). It sends nothing at startup.

use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;

/// The least time between two notifications.
pub const NOTIFY_INTERVAL_MS: i64 = 20 * 60 * 1000;

/// The bound on one request to a crawler.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The hosts to notify and when they were last notified.
pub struct Crawlers {
    hostname: String,
    crawlers: Vec<String>,
    http: reqwest::Client,
    last_notified_ms: Mutex<Option<i64>>,
}

impl std::fmt::Debug for Crawlers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Crawlers")
            .field("hostname", &self.hostname)
            .field("crawlers", &self.crawlers)
            .finish_non_exhaustive()
    }
}

impl Crawlers {
    /// `hostname` is the bare host name of this server (`PDS_HOSTNAME`);
    /// `crawlers` are service URLs without a trailing slash
    /// (`PDS_CRAWLERS`).
    pub fn new(hostname: String, crawlers: Vec<String>, http: reqwest::Client) -> Self {
        Self {
            hostname,
            crawlers,
            http,
            last_notified_ms: Mutex::new(None),
        }
    }

    /// Whether a notification is due at `now_ms`. When it is, the time is
    /// recorded.
    fn due(&self, now_ms: i64) -> bool {
        let mut last = self
            .last_notified_ms
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|last| last >= now_ms - NOTIFY_INTERVAL_MS) {
            return false;
        }
        *last = Some(now_ms);
        true
    }

    /// Notes that an event was sequenced at `now_ms`. When a notification is
    /// due, each crawler gets one request in the background; a failure is
    /// logged and not retried.
    ///
    /// Call it inside a Tokio runtime. Outside one it does nothing.
    pub fn notify_of_update(&self, now_ms: i64) {
        if self.crawlers.is_empty() || !self.due(now_ms) {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        for crawler in &self.crawlers {
            let url = format!("{crawler}/xrpc/com.atproto.sync.requestCrawl");
            let body = serde_json::json!({ "hostname": self.hostname }).to_string();
            let request = self
                .http
                .post(&url)
                .header("content-type", "application/json")
                .timeout(TIMEOUT)
                .body(body);
            let crawler = crawler.clone();
            runtime.spawn(async move {
                match request.send().await {
                    Ok(response) if response.status().is_success() => {}
                    Ok(response) => {
                        tracing::warn!(%crawler, status = %response.status(), "failed to request crawl");
                    }
                    Err(error) => tracing::warn!(%crawler, %error, "failed to request crawl"),
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notification_is_due_once_in_twenty_minutes() {
        let crawlers = Crawlers::new(
            "pds.test".to_owned(),
            vec!["https://relay.test".to_owned()],
            reqwest::Client::new(),
        );
        assert!(crawlers.due(1_000));
        assert!(!crawlers.due(1_001));
        assert!(!crawlers.due(1_000 + NOTIFY_INTERVAL_MS));
        assert!(crawlers.due(1_001 + NOTIFY_INTERVAL_MS));
    }
}
