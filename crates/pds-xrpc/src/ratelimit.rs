//! Rate limits (`docs/SPEC.md` §4.3), as the Reference configures them at
//! the pin (`rate-limits.ts`, the `rateLimit` of each route, and the
//! limiter of `@atproto/xrpc-server`).
//!
//! A limiter is a fixed window that starts at its first request. A
//! request spends points in every limiter that applies to it, and the
//! answer names the one with the fewest points left. A request with the
//! bypass key, or from a bypass address, spends nothing and gets no
//! header.

// A request holds the bucket table for one lookup and one addition.
#![allow(clippy::significant_drop_tightening)]

use std::cell::RefCell;
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;

use crate::error::XrpcError;
use crate::state::AppState;
use crate::table::Swept;

/// The settings: `PDS_RATE_LIMITS_ENABLED`, `PDS_RATE_LIMIT_BYPASS_KEY`
/// and `PDS_RATE_LIMIT_BYPASS_IPS`.
#[derive(Clone, Default)]
pub struct RateLimitConfig {
    /// Whether any limiter runs.
    pub enabled: bool,
    /// The value of `x-ratelimit-bypass` that lifts every limit.
    pub bypass_key: Option<String>,
    /// Addresses that no limiter counts.
    pub bypass_ips: Vec<String>,
}

impl std::fmt::Debug for RateLimitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimitConfig")
            .field("enabled", &self.enabled)
            .field("bypass_ips", &self.bypass_ips)
            .finish_non_exhaustive()
    }
}

/// One limiter: its name, its window in seconds and its points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limit {
    /// The name that keeps its buckets apart from another limiter's.
    pub name: &'static str,
    /// The window in seconds.
    pub window_s: i64,
    /// The points of one window.
    pub points: i64,
}

/// Every request, by client address: 3,000 in five minutes.
pub const GLOBAL_IP: Limit = Limit {
    name: "global-ip",
    window_s: 300,
    points: 3000,
};
/// Repository writes by account: 5,000 points an hour.
pub const REPO_WRITE_HOUR: Limit = Limit {
    name: "repo-write-hour",
    window_s: 3600,
    points: 5000,
};
/// Repository writes by account: 35,000 points a day.
pub const REPO_WRITE_DAY: Limit = Limit {
    name: "repo-write-day",
    window_s: 86_400,
    points: 35_000,
};

/// What a limiter says after a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// The points of one window.
    pub limit: i64,
    /// The window in seconds.
    pub window_s: i64,
    /// The points left in this window.
    pub remaining: i64,
    /// When the window ends, in milliseconds since the epoch.
    pub resets_at_ms: i64,
    /// Whether the request went over the limit.
    pub exceeded: bool,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    spent: i64,
    resets_at_ms: i64,
}

/// How many buckets the table holds before it drops the ended ones.
const BUCKETS_SWEEP_AT: usize = 100_000;

/// The buckets of every limiter.
#[derive(Debug, Default)]
pub struct Limiter {
    buckets: Mutex<Swept<(&'static str, String), Bucket, BUCKETS_SWEEP_AT>>,
}

impl Limiter {
    /// Spends `points` of `limit` for `key`.
    pub fn consume(&self, limit: Limit, key: &str, points: i64, now_ms: i64) -> Status {
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        // The table stays small: a bucket goes when its window is over.
        buckets.sweep(|bucket| bucket.resets_at_ms > now_ms);
        let bucket = buckets
            .entry((limit.name, key.to_owned()))
            .or_insert(Bucket {
                spent: 0,
                resets_at_ms: now_ms + limit.window_s * 1000,
            });
        if bucket.resets_at_ms <= now_ms {
            *bucket = Bucket {
                spent: 0,
                resets_at_ms: now_ms + limit.window_s * 1000,
            };
        }
        bucket.spent += points;
        Status {
            limit: limit.points,
            window_s: limit.window_s,
            remaining: (limit.points - bucket.spent).max(0),
            resets_at_ms: bucket.resets_at_ms,
            exceeded: bucket.spent > limit.points,
        }
    }
}

/// What the limiters know of the request that this task serves.
#[derive(Debug, Clone, Default)]
struct Current {
    ip: String,
    bypass: bool,
    lowest: Option<Status>,
}

tokio::task_local! {
    static CURRENT: RefCell<Current>;
}

/// The address of the client of the request this task serves, as the
/// limiter counts it. `None` outside a request.
pub fn current_ip() -> Option<String> {
    CURRENT.try_with(|current| current.borrow().ip.clone()).ok()
}

fn trusted(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_link_local() || v4.is_private(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| trusted(IpAddr::V4(v4)))
        }
    }
}

/// The address of the client: the last address of the chain, from the
/// connection back through `X-Forwarded-For`, that is not a proxy of a
/// loopback, link-local or private address.
pub fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>) -> String {
    let forwarded: Vec<&str> = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(',').map(str::trim).collect())
        .unwrap_or_default();
    // A connection without an address is one inside a test process.
    let peer_trusted = peer.is_none_or(trusted);
    let peer_text = peer.map_or_else(|| "127.0.0.1".to_owned(), |peer| peer.to_string());
    if !peer_trusted {
        return peer_text;
    }
    let mut client = peer_text;
    for hop in forwarded.iter().rev() {
        (*hop).clone_into(&mut client);
        if !hop.parse::<IpAddr>().is_ok_and(trusted) {
            break;
        }
    }
    client
}

/// Serves one request with its limiter state, and adds the limiter's
/// headers to the answer.
pub async fn scope<F>(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    serve: F,
) -> Response
where
    F: Future<Output = Response>,
{
    let config = &state.config().rate_limits;
    let ip = client_ip(headers, peer);
    let bypass = !config.enabled
        || config.bypass_ips.contains(&ip)
        || config.bypass_key.as_deref().is_some_and(|key| {
            headers
                .get("x-ratelimit-bypass")
                .and_then(|value| value.to_str().ok())
                == Some(key)
        });
    let current = RefCell::new(Current {
        ip,
        bypass,
        lowest: None,
    });
    CURRENT
        .scope(current, async {
            let mut response = serve.await;
            let lowest = CURRENT.with(|current| current.borrow().lowest);
            if let Some(status) = lowest {
                let now_ms = state.now_ms();
                let headers = response.headers_mut();
                let mut expose =
                    "RateLimit-Limit, RateLimit-Reset, RateLimit-Remaining, RateLimit-Policy"
                        .to_owned();
                headers.insert("ratelimit-limit", HeaderValue::from(status.limit));
                headers.insert(
                    "ratelimit-reset",
                    HeaderValue::from(status.resets_at_ms / 1000),
                );
                headers.insert("ratelimit-remaining", HeaderValue::from(status.remaining));
                if let Ok(policy) =
                    HeaderValue::from_str(&format!("{};w={}", status.limit, status.window_s))
                {
                    headers.insert("ratelimit-policy", policy);
                }
                if status.exceeded {
                    let seconds = (status.resets_at_ms - now_ms + 999).max(0) / 1000;
                    headers.insert("retry-after", HeaderValue::from(seconds));
                    expose.push_str(", Retry-After");
                }
                if let Ok(expose) = HeaderValue::from_str(&expose) {
                    headers.insert("access-control-expose-headers", expose);
                }
            }
            response
        })
        .await
}

/// How a limiter finds its bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key<'a> {
    /// The client address.
    Ip,
    /// A text and the client address: a sign-in counts by identifier and
    /// address.
    WithIp(&'a str),
    /// A text: an account.
    Text(&'a str),
}

/// Spends `points` of each limiter for the request that this task serves.
/// The first limiter that is over its limit refuses the request.
pub fn consume(state: &AppState, limits: &[(Limit, Key<'_>, i64)]) -> Result<(), XrpcError> {
    let Ok(context) = CURRENT.try_with(|current| current.borrow().clone()) else {
        return Ok(());
    };
    if context.bypass {
        return Ok(());
    }
    let now_ms = state.now_ms();
    let mut refused = false;
    let mut lowest = context.lowest;
    for (limit, key, points) in limits {
        if *points < 1 {
            continue;
        }
        let key = match key {
            Key::Ip => context.ip.clone(),
            Key::WithIp(text) => format!("{text}-{}", context.ip),
            Key::Text(text) => (*text).to_owned(),
        };
        let status = state.limiter().consume(*limit, &key, *points, now_ms);
        refused |= status.exceeded;
        if status.exceeded
            || lowest.is_none_or(|lowest| !lowest.exceeded && status.remaining < lowest.remaining)
        {
            lowest = Some(status);
        }
        if status.exceeded {
            break;
        }
    }
    let _ = CURRENT.try_with(|current| current.borrow_mut().lowest = lowest);
    if refused {
        return Err(XrpcError {
            status: StatusCode::TOO_MANY_REQUESTS,
            error: "RateLimitExceeded".to_owned(),
            message: "Rate Limit Exceeded".to_owned(),
        });
    }
    Ok(())
}

/// A limiter of one route: `index` tells the limiters of a route apart.
pub const fn of_route(name: &'static str, window_s: i64, points: i64) -> Limit {
    Limit {
        name,
        window_s,
        points,
    }
}

/// The two limiters of a mail that an account asks for: 15 a day and 5 an
/// hour.
pub fn mail_request(
    state: &AppState,
    day: &'static str,
    hour: &'static str,
    did: &str,
) -> Result<(), XrpcError> {
    route(
        state,
        &[
            (of_route(day, 86_400, 15), Key::Text(did), 1),
            (of_route(hour, 3600, 5), Key::Text(did), 1),
        ],
    )
}

/// The points of a batch of writes: 3 for a create, 2 for an update and 1
/// for a delete.
pub fn batch_points(input: &serde_json::Map<String, serde_json::Value>) -> i64 {
    let Some(writes) = input.get("writes").and_then(serde_json::Value::as_array) else {
        return 0;
    };
    writes
        .iter()
        .map(
            |write| match write.get("$type").and_then(serde_json::Value::as_str) {
                Some("com.atproto.repo.applyWrites#create") => 3,
                Some("com.atproto.repo.applyWrites#update") => 2,
                _ => 1,
            },
        )
        .sum()
}

/// The limiters of a repository write of `points` by the account `did`.
pub fn repo_write(state: &AppState, did: &str, points: i64) -> Result<(), XrpcError> {
    route(
        state,
        &[
            (REPO_WRITE_HOUR, Key::Text(did), points),
            (REPO_WRITE_DAY, Key::Text(did), points),
        ],
    )
}

/// The limiters of a route with the global one: every request spends one
/// point of the address bucket.
pub fn route(state: &AppState, limits: &[(Limit, Key<'_>, i64)]) -> Result<(), XrpcError> {
    let mut all = limits.to_vec();
    all.push((GLOBAL_IP, Key::Ip, 1));
    consume(state, &all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_starts_at_its_first_request_and_ends() {
        let limiter = Limiter::default();
        let limit = Limit {
            name: "test",
            window_s: 10,
            points: 3,
        };
        let first = limiter.consume(limit, "a", 1, 1_000);
        assert_eq!(
            (first.remaining, first.resets_at_ms, first.exceeded),
            (2, 11_000, false)
        );
        assert_eq!(limiter.consume(limit, "b", 1, 2_000).remaining, 2);
        assert_eq!(limiter.consume(limit, "a", 2, 3_000).remaining, 0);
        let over = limiter.consume(limit, "a", 1, 4_000);
        assert!(over.exceeded && over.remaining == 0 && over.resets_at_ms == 11_000);
        let next = limiter.consume(limit, "a", 1, 11_000);
        assert_eq!(
            (next.remaining, next.resets_at_ms, next.exceeded),
            (2, 21_000, false)
        );
    }

    #[test]
    fn the_client_address_is_the_last_one_that_is_not_a_proxy() {
        let headers = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert("x-forwarded-for", HeaderValue::from_str(value).unwrap());
            headers
        };
        let local: IpAddr = "172.19.0.3".parse().unwrap();
        let public: IpAddr = "198.51.100.7".parse().unwrap();
        assert_eq!(
            client_ip(&headers("203.0.113.5, 172.19.0.1"), Some(local)),
            "203.0.113.5"
        );
        assert_eq!(
            client_ip(&headers("10.0.0.1, 203.0.113.9, 192.168.1.1"), Some(local)),
            "203.0.113.9"
        );
        // A connection from a public address is the client, whatever it says.
        assert_eq!(
            client_ip(&headers("203.0.113.5"), Some(public)),
            "198.51.100.7"
        );
        assert_eq!(client_ip(&HeaderMap::new(), Some(local)), "172.19.0.3");
        assert_eq!(client_ip(&headers("203.0.113.5"), None), "203.0.113.5");
    }
}
