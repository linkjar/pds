//! Clients of the authorization server (`docs/SPEC.md` §11.2): a loopback
//! client, whose metadata the server writes, or a client whose `client_id`
//! is the URL of its metadata.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use axum::http::header;
use pds_identity::safe_fetch::{self, Policy};
use pds_oauth::client::{Client, ClientError, ClientId, validate};
use pds_oauth::token::{
    REFRESH_LIFETIME_EXTENDED_MS, REFRESH_LIFETIME_MS, SESSION_LIFETIME_EXTENDED_MS,
    SESSION_LIFETIME_MS,
};
use serde_json::Value;
use url::Url;

use super::OAuthError;
use crate::state::AppState;

/// How long the server keeps a client's metadata.
const CACHE_TTL_MS: i64 = 10 * 60 * 1000;
/// The largest metadata document.
const DOCUMENT_LIMIT: usize = 256 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// A client with what the server learned of it.
#[derive(Debug, Clone)]
pub struct LoadedClient {
    /// The metadata after the checks.
    pub client: Client,
    /// The key set of a confidential client.
    pub jwks: Option<Value>,
    /// Whether `PDS_OAUTH_TRUSTED_CLIENTS` lists it.
    pub trusted: bool,
}

impl LoadedClient {
    /// Whether the client authenticates with a key.
    pub fn confidential(&self) -> bool {
        self.client.auth_method() == "private_key_jwt"
    }

    /// How long a session of the client may live.
    pub fn session_lifetime_ms(&self) -> i64 {
        if self.confidential() || self.trusted {
            SESSION_LIFETIME_EXTENDED_MS
        } else {
            SESSION_LIFETIME_MS
        }
    }

    /// How long a refresh token of the client is good.
    pub fn refresh_lifetime_ms(&self) -> i64 {
        if self.confidential() || self.trusted {
            REFRESH_LIFETIME_EXTENDED_MS
        } else {
            REFRESH_LIFETIME_MS
        }
    }

    /// Whether `uri` is a redirect URI of the client. A loopback URI
    /// matches on any port.
    pub fn has_redirect_uri(&self, uri: &str) -> bool {
        let Ok(given) = Url::parse(uri) else {
            return false;
        };
        self.client.redirect_uris().iter().any(|own| {
            if *own == uri {
                return true;
            }
            let Ok(own) = Url::parse(own) else {
                return false;
            };
            let loopback = matches!(own.host_str(), Some("127.0.0.1" | "[::1]"));
            loopback
                && own.scheme() == "http"
                && given.scheme() == "http"
                && own.host_str() == given.host_str()
                && own.path() == given.path()
                && own.query() == given.query()
        })
    }
}

impl From<ClientError> for OAuthError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::Metadata(message) => Self::new("invalid_client_metadata", message),
            ClientError::RedirectUri(message) => Self::new("invalid_redirect_uri", message),
            ClientError::ClientId(message) => Self::new("invalid_client_id", message),
        }
    }
}

fn metadata_error(message: impl Into<String>) -> OAuthError {
    OAuthError::new("invalid_client_metadata", message)
}

/// The domains that RFC 2606 keeps for documentation. Nothing serves a
/// client there.
fn is_documentation_host(host: &str) -> bool {
    ["example.com", "example.net", "example.org"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

/// Fetches a JSON document of a client: HTTPS, no redirect, status 200,
/// `application/json`, and a public address. Disabling address checks
/// requires both development mode and the explicit SSRF override.
async fn fetch_json(state: &AppState, url: &Url, what: &str) -> Result<Value, OAuthError> {
    let failed = |why: &str| metadata_error(format!("Unable to obtain {what} \"{url}\": {why}"));
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https" {
        return Err(failed("the URL must use HTTPS"));
    }
    if is_documentation_host(host) {
        return Err(failed("the host name is reserved for documentation"));
    }
    let policy = if state.config().dev_mode && state.config().oauth.disable_ssrf_protection {
        Policy::Development
    } else {
        Policy::Public
    };
    let response = safe_fetch::request(reqwest::Method::GET, url.as_str(), policy)
        .map_err(|_| failed("the destination is not allowed"))?
        .header(header::ACCEPT, "application/json")
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|_| failed("the request failed"))?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(failed(&format!(
            "unexpected status {}",
            response.status().as_u16()
        )));
    }
    let json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"));
    if !json {
        return Err(failed("unexpected content type"));
    }
    let bytes = safe_fetch::read_bounded(response, DOCUMENT_LIMIT)
        .await
        .map_err(|_| failed("the request failed or the document is too large"))?;
    serde_json::from_slice(&bytes).map_err(|_| failed("the document is not JSON"))
}

async fn discover(
    state: &AppState,
    client_id: &str,
    url: &Url,
) -> Result<LoadedClient, OAuthError> {
    let document = fetch_json(state, url, "client metadata for").await?;
    let client = validate(client_id, &document)?;
    let jwks = match (
        client.metadata.get("jwks"),
        client.metadata.get("jwks_uri").and_then(Value::as_str),
    ) {
        (Some(jwks), _) if !jwks.is_null() => Some(jwks.clone()),
        (_, Some(uri)) => {
            let uri = Url::parse(uri).map_err(|_| metadata_error("jwks_uri is not a URL"))?;
            Some(fetch_json(state, &uri, "the key set").await?)
        }
        _ => None,
    };
    Ok(LoadedClient {
        client,
        jwks,
        trusted: state
            .config()
            .oauth
            .trusted_clients
            .iter()
            .any(|trusted| trusted == client_id),
    })
}

/// The clients of `PDS_OAUTH_KNOWN_CLIENTS_FILE` as entries of the cache
/// that do not end. A document that does not pass the checks of a
/// fetched one is left out; the configuration refuses it at start.
pub(super) fn known(config: &super::OAuthConfig) -> HashMap<String, (Arc<LoadedClient>, i64)> {
    config
        .known_clients
        .iter()
        .filter_map(|(client_id, document)| {
            let client = validate(client_id, document).ok()?;
            let jwks = client
                .metadata
                .get("jwks")
                .filter(|jwks| !jwks.is_null())
                .cloned();
            let loaded = LoadedClient {
                client,
                jwks,
                trusted: config.trusted_clients.contains(client_id),
            };
            Some((client_id.clone(), (Arc::new(loaded), i64::MAX)))
        })
        .collect()
}

/// The client `client_id`, from the cache or from the network.
pub async fn get(state: &AppState, client_id: &str) -> Result<Arc<LoadedClient>, OAuthError> {
    let url = match ClientId::parse(client_id)? {
        ClientId::Loopback {
            redirect_uris,
            scope,
        } => {
            let document = ClientId::loopback_metadata(client_id, &redirect_uris, scope.as_deref());
            return Ok(Arc::new(LoadedClient {
                client: validate(client_id, &document)?,
                jwks: None,
                trusted: false,
            }));
        }
        ClientId::Discoverable(url) => url,
    };
    let now_ms = state.now_ms();
    let cache = &state.oauth().clients;
    let cached = cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(client_id)
        .filter(|(_, fetched)| now_ms - fetched < CACHE_TTL_MS)
        .map(|(client, _)| Arc::clone(client));
    if let Some(client) = cached {
        return Ok(client);
    }
    let client = Arc::new(discover(state, client_id, &url).await?);
    {
        let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.sweep(|(_, fetched)| now_ms - *fetched < CACHE_TTL_MS);
        cache.insert(client_id.to_owned(), (Arc::clone(&client), now_ms));
    }
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documentation_domains_are_reserved() {
        assert!(is_documentation_host("client.example.com"));
        assert!(!is_documentation_host("example.io"));
    }
}
