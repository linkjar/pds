//! HTTP requests to destinations supplied by untrusted documents.
//!
//! Address checks run in the connector's resolver: the connector receives
//! exactly the checked addresses, without a second DNS lookup. URL checks
//! also cover IP literals, which bypass DNS. Redirects and environment
//! proxies are disabled. Explicitly configured services use the caller's
//! trusted client instead. Every caller must also bound the response body.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::{Client, Method, RequestBuilder, Response, Url};

/// The network destinations a request may reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// HTTPS on port 443, with only public internet addresses.
    Public,
    /// Local development only: HTTP, other ports, and private addresses.
    /// Callers must gate this on the server's development configuration.
    Development,
}

/// An outbound request was refused or failed.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// The destination is not permitted.
    #[error("{0}")]
    Destination(&'static str),
    /// The HTTP client could not be constructed.
    #[error("could not construct the outbound client: {0}")]
    Client(String),
    /// A network operation failed.
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    /// The response exceeds the caller's byte budget.
    #[error("the response is over {0} bytes")]
    TooLarge(usize),
}

/// Whether an IP address is a public unicast destination.
///
/// Special-use ranges, including translated and transition IPv6 ranges,
/// are excluded. An IPv4-mapped IPv6 address follows the IPv4 policy.
fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [a, b, c, _] = address.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..128).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..32).contains(&b))
                || (a == 192 && b == 0 && (c == 0 || c == 2))
                || (a == 192 && b == 88 && c == 99)
                || (a == 192 && b == 168)
                || (a == 198 && (18..20).contains(&b))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(address) => address.to_ipv4_mapped().map_or_else(
            || {
                let [first, second, ..] = address.segments();
                (first & 0xe000) == 0x2000
                    && !(first == 0x2001 && (second < 0x200 || second == 0x0db8))
                    && first != 0x2002
                    && !(first == 0x3fff && second < 0x1000)
            },
            |v4| is_public(IpAddr::V4(v4)),
        ),
    }
}

struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((name.as_str(), 0)).await?;
            Ok(Box::new(addresses.collect::<Vec<_>>().into_iter()) as Addrs)
        })
    }
}

struct CheckedResolver {
    source: Arc<dyn Resolve>,
    policy: Policy,
}

impl Resolve for CheckedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let resolving = self.source.resolve(name);
        let policy = self.policy;
        Box::pin(async move {
            let addresses: Vec<SocketAddr> = resolving.await?.collect();
            if addresses.is_empty()
                || (policy == Policy::Public
                    && addresses.iter().any(|address| !is_public(address.ip())))
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "the host is not on the public internet",
                )
                .into());
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

fn client(policy: Policy, source: Arc<dyn Resolve>) -> Result<Client, reqwest::Error> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(CheckedResolver { source, policy }))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
}

fn destination(url: &str, policy: Policy) -> Result<Url, FetchError> {
    let url = Url::parse(url).map_err(|_| FetchError::Destination("invalid destination URL"))?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(FetchError::Destination(
            "destination URL contains credentials or a fragment",
        ));
    }
    if !matches!(url.scheme(), "http" | "https") {
        return Err(FetchError::Destination(
            "destination URL must use HTTP or HTTPS",
        ));
    }
    let host = url
        .host_str()
        .ok_or(FetchError::Destination("destination URL has no host"))?;
    if policy == Policy::Public {
        if url.scheme() != "https" || url.port_or_known_default() != Some(443) {
            return Err(FetchError::Destination(
                "destination URL must use HTTPS on port 443",
            ));
        }
        let host = host.trim_end_matches('.');
        if host
            .rsplit('.')
            .next()
            .is_some_and(|label| matches!(label, "localhost" | "local"))
        {
            return Err(FetchError::Destination(
                "the host is not on the public internet",
            ));
        }
        // Url normalizes alternative IPv4 forms; IPv6 host strings have brackets.
        if let Ok(address) = host.trim_start_matches('[').trim_end_matches(']').parse()
            && !is_public(address)
        {
            return Err(FetchError::Destination(
                "the host is not on the public internet",
            ));
        }
    }
    Ok(url)
}

/// Builds a request with connector-level address validation.
///
/// The reusable client has a ten-second default deadline; a caller may
/// set a different finite deadline with [`RequestBuilder::timeout`].
pub fn request(method: Method, url: &str, policy: Policy) -> Result<RequestBuilder, FetchError> {
    static PUBLIC: OnceLock<Result<Client, String>> = OnceLock::new();
    static DEVELOPMENT: OnceLock<Result<Client, String>> = OnceLock::new();
    let url = destination(url, policy)?;
    let clients = match policy {
        Policy::Public => &PUBLIC,
        Policy::Development => &DEVELOPMENT,
    };
    let http =
        clients.get_or_init(|| client(policy, Arc::new(SystemResolver)).map_err(|e| e.to_string()));
    match http {
        Ok(http) => Ok(http.request(method, url)),
        Err(error) => Err(FetchError::Client(error.clone())),
    }
}

/// Reads a response while enforcing a byte limit, including responses
/// without Content-Length. The request's timeout also covers body reads.
pub async fn read_bounded(mut response: Response, limit: usize) -> Result<Vec<u8>, FetchError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(FetchError::TooLarge(limit));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(FetchError::TooLarge(limit));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
