//! Metrics in the Prometheus text format (`docs/SPEC.md` §14.4).
//!
//! The server counts and times every request by route and status. The
//! other values are read when the metrics are asked for. A label never
//! holds a value that a client chose freely: a method the server does not
//! handle is `proxied`, and a path it does not know is `other`.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::state::AppState;

/// The upper bounds of the latency buckets, in seconds.
const BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

#[derive(Debug, Clone, Default)]
struct Timing {
    /// How many requests each bucket holds, without the ones below it.
    buckets: [u64; BUCKETS.len()],
    count: u64,
    sum_s: f64,
}

/// What the server counts as it serves.
#[derive(Debug, Default)]
pub struct Metrics {
    requests: Mutex<BTreeMap<(String, u16), u64>>,
    timings: Mutex<BTreeMap<String, Timing>>,
}

/// The paths outside `/xrpc/` that have a label of their own.
const PATHS: [&str; 14] = [
    "/",
    "/robots.txt",
    "/tls-check",
    "/account",
    "/oauth/authorize",
    "/oauth/par",
    "/oauth/token",
    "/oauth/revoke",
    "/oauth/jwks",
    "/.well-known/atproto-did",
    "/.well-known/oauth-authorization-server",
    "/.well-known/oauth-protected-resource",
    "/xrpc/_health",
    "/metrics",
];

/// The label of a request: the method of an XRPC call that the server
/// handles, a known path, or a class of paths.
pub fn route_label(path: &str) -> String {
    if let Some(nsid) = path.strip_prefix("/xrpc/") {
        return if path == "/xrpc/_health" {
            path.to_owned()
        } else if crate::api::is_handled(nsid) {
            nsid.to_owned()
        } else {
            "proxied".to_owned()
        };
    }
    if PATHS.contains(&path) {
        path.to_owned()
    } else if path.starts_with("/oauth/authorize/") {
        "/oauth/authorize/*".to_owned()
    } else if path.starts_with("/oauth/external/") {
        "/oauth/external/*".to_owned()
    } else if path.starts_with("/oauth/assets/") {
        "/oauth/assets/*".to_owned()
    } else if path.starts_with("/account/") {
        "/account/*".to_owned()
    } else {
        "other".to_owned()
    }
}

impl Metrics {
    /// Counts one request and its duration.
    pub fn observe(&self, route: &str, status: u16, took: Duration) {
        *self
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry((route.to_owned(), status))
            .or_default() += 1;
        let seconds = took.as_secs_f64();
        let mut timings = self.timings.lock().unwrap_or_else(PoisonError::into_inner);
        let timing = timings.entry(route.to_owned()).or_default();
        if let Some(at) = BUCKETS.iter().position(|bound| seconds <= *bound) {
            timing.buckets[at] += 1;
        }
        timing.count += 1;
        timing.sum_s += seconds;
        drop(timings);
    }

    fn render_requests(&self, out: &mut String) {
        out.push_str("# HELP pds_http_requests_total Requests by route and status.\n");
        out.push_str("# TYPE pds_http_requests_total counter\n");
        let requests = self
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for ((route, status), count) in requests {
            let _ = writeln!(
                out,
                "pds_http_requests_total{{route=\"{route}\",status=\"{status}\"}} {count}"
            );
        }
        out.push_str("# HELP pds_http_request_duration_seconds Time to answer, by route.\n");
        out.push_str("# TYPE pds_http_request_duration_seconds histogram\n");
        let timings = self
            .timings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for (route, timing) in timings {
            let mut below = 0;
            for (bound, held) in BUCKETS.iter().zip(timing.buckets) {
                below += held;
                let _ = writeln!(
                    out,
                    "pds_http_request_duration_seconds_bucket{{route=\"{route}\",le=\"{bound}\"}} {below}"
                );
            }
            let _ = writeln!(
                out,
                "pds_http_request_duration_seconds_bucket{{route=\"{route}\",le=\"+Inf\"}} {}",
                timing.count
            );
            let _ = writeln!(
                out,
                "pds_http_request_duration_seconds_sum{{route=\"{route}\"}} {}",
                timing.sum_s
            );
            let _ = writeln!(
                out,
                "pds_http_request_duration_seconds_count{{route=\"{route}\"}} {}",
                timing.count
            );
        }
    }
}

fn gauge(out: &mut String, name: &str, help: &str, value: impl std::fmt::Display) {
    let _ = writeln!(
        out,
        "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {value}"
    );
}

/// The metrics of the server as text. This call reads the databases and
/// blocks.
pub fn render(state: &AppState) -> String {
    let mut out = String::new();
    let version = state.config().version.as_deref().unwrap_or("unknown");
    let _ = writeln!(
        out,
        "# HELP pds_build_info The version that runs.\n# TYPE pds_build_info gauge\npds_build_info{{version=\"{}\",profile=\"{}\"}} 1",
        version.replace(['"', '\\', '\n'], ""),
        state.config().profile.name()
    );
    state.metrics().render_requests(&mut out);
    gauge(
        &mut out,
        "pds_firehose_subscribers",
        "Connections to subscribeRepos.",
        state.subscribers().len(),
    );
    if let Some(log) = state.audit() {
        if let Ok(entries) = log.len() {
            gauge(
                &mut out,
                "pds_audit_entries",
                "Entries of the audit log.",
                entries,
            );
        }
        // An old checkpoint with entries after it means the signer stopped.
        if let Ok(Some(age_s)) = log.checkpoint_age_s(state.now_ms()) {
            gauge(
                &mut out,
                "pds_audit_checkpoint_age_seconds",
                "Time since the last signed checkpoint.",
                age_s,
            );
        }
        if let Ok(open) = log.unsigned() {
            gauge(
                &mut out,
                "pds_audit_unsigned_entries",
                "Entries after the last signed checkpoint.",
                open,
            );
        }
    }
    if state.config().profile.produces_notices()
        && let Some(writes) = state.writes()
        && let Ok(depth) = writes.with_accounts(|accounts| accounts.notice_count())
    {
        gauge(
            &mut out,
            "pds_notice_outbox_depth",
            "Sign-in notices that wait for delivery.",
            depth,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_is_a_route_the_server_knows() {
        assert_eq!(
            route_label("/xrpc/com.atproto.sync.getRepo"),
            "com.atproto.sync.getRepo"
        );
        assert_eq!(route_label("/xrpc/app.bsky.feed.getTimeline"), "proxied");
        assert_eq!(route_label("/xrpc/x\"}\n"), "proxied");
        assert_eq!(route_label("/oauth/token"), "/oauth/token");
        assert_eq!(
            route_label("/oauth/external/google/start"),
            "/oauth/external/*"
        );
        assert_eq!(route_label("/account/did:plc:abc/manage"), "/account/*");
        assert_eq!(route_label("/wp-login.php"), "other");
        assert_eq!(route_label("/xrpc/_health"), "/xrpc/_health");
    }

    #[test]
    fn requests_are_counted_and_timed_in_cumulative_buckets() {
        let metrics = Metrics::default();
        metrics.observe("/oauth/token", 200, Duration::from_millis(3));
        metrics.observe("/oauth/token", 200, Duration::from_millis(30));
        metrics.observe("/oauth/token", 400, Duration::from_secs(20));
        let mut out = String::new();
        metrics.render_requests(&mut out);
        for line in [
            "pds_http_requests_total{route=\"/oauth/token\",status=\"200\"} 2",
            "pds_http_requests_total{route=\"/oauth/token\",status=\"400\"} 1",
            "pds_http_request_duration_seconds_bucket{route=\"/oauth/token\",le=\"0.005\"} 1",
            "pds_http_request_duration_seconds_bucket{route=\"/oauth/token\",le=\"0.05\"} 2",
            "pds_http_request_duration_seconds_bucket{route=\"/oauth/token\",le=\"10\"} 2",
            "pds_http_request_duration_seconds_bucket{route=\"/oauth/token\",le=\"+Inf\"} 3",
            "pds_http_request_duration_seconds_count{route=\"/oauth/token\"} 3",
        ] {
            assert!(out.contains(line), "{line}\n{out}");
        }
    }
}
