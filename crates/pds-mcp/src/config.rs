//! The configuration of the MCP server, from `MCP_*` variables.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use pds_identity::{Algorithm, Keypair};
use serde_json::{Value, json};
use url::Url;

/// The path of the MCP endpoint.
pub const MCP_PATH: &str = "/mcp";

/// The path the PDS sends the browser back to.
pub const PDS_CALLBACK_PATH: &str = "/oauth/pds-callback";

/// The path of the client metadata document.
pub const CLIENT_METADATA_PATH: &str = "/oauth-client-metadata.json";

/// The variable with the key of the server as a client of the PDS.
const KEY: &str = "MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX";

/// Why the configuration is not usable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// A variable that the server needs is not set.
    #[error("{0} is not set")]
    Missing(&'static str),
    /// A variable has a value the server cannot use.
    #[error("{name}: {reason}")]
    Invalid {
        /// The variable.
        name: &'static str,
        /// What is wrong with it.
        reason: &'static str,
    },
}

/// What the MCP server needs to know.
pub struct Config {
    /// `MCP_ADDRESS`: where the server listens. Default `127.0.0.1:2586`.
    pub address: SocketAddr,
    /// `MCP_PUBLIC_URL`: the address AI clients use, such as
    /// `https://mcp.example.com`.
    pub public_url: String,
    /// `MCP_PDS_PUBLIC_URL`: the public address of the PDS, its issuer.
    pub pds_public_url: String,
    /// `MCP_PDS_INTERNAL_URL`: where the server reaches the PDS. Default:
    /// the public address.
    pub pds_internal_url: String,
    /// `MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX`: the key of the server as an
    /// OAuth client of the PDS.
    pub client_key: Keypair,
    /// `MCP_WRITE_COLLECTIONS`: the collections an AI client may ask to
    /// write one at a time, as NSIDs separated by commas. The PDS takes a
    /// scope only when the client metadata declares it, so the list is
    /// part of the metadata.
    pub write_collections: Vec<String>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("address", &self.address)
            .field("public_url", &self.public_url)
            .field("pds_public_url", &self.pds_public_url)
            .field("pds_internal_url", &self.pds_internal_url)
            .finish_non_exhaustive()
    }
}

/// An origin without a path. `https`, or `http` for a loopback address.
fn origin(name: &'static str, value: &str) -> Result<String, ConfigError> {
    let invalid = |reason| ConfigError::Invalid { name, reason };
    let origin = pds_sidecar::config::origin(value).map_err(invalid)?;
    let allowed = Url::parse(&origin).is_ok_and(|url| {
        url.scheme() == "https"
            || matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"))
    });
    if !allowed {
        return Err(invalid("expected https, or http on a loopback address"));
    }
    Ok(origin)
}

/// An origin on a private network: `http` or `https`, any host.
fn internal(name: &'static str, value: &str) -> Result<String, ConfigError> {
    pds_sidecar::config::origin(value).map_err(|reason| ConfigError::Invalid { name, reason })
}

impl Config {
    /// Reads the configuration from the variables.
    pub fn from_vars(vars: &BTreeMap<String, String>) -> Result<Self, ConfigError> {
        let text = |name: &'static str| {
            vars.get(name)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
        };
        let required = |name: &'static str| text(name).ok_or(ConfigError::Missing(name));
        let address = text("MCP_ADDRESS")
            .unwrap_or("127.0.0.1:2586")
            .parse()
            .map_err(|_| ConfigError::Invalid {
                name: "MCP_ADDRESS",
                reason: "expected an address and a port, such as 127.0.0.1:2586",
            })?;
        let pds_public_url = internal("MCP_PDS_PUBLIC_URL", required("MCP_PDS_PUBLIC_URL")?)?;
        let pds_internal_url = match text("MCP_PDS_INTERNAL_URL") {
            Some(value) => internal("MCP_PDS_INTERNAL_URL", value)?,
            None => pds_public_url.clone(),
        };
        let bad_key = ConfigError::Invalid {
            name: KEY,
            reason: "expected the 64 hexadecimal digits of a P-256 private key",
        };
        let client_key = data_encoding::HEXLOWER_PERMISSIVE
            .decode(required(KEY)?.as_bytes())
            .ok()
            .and_then(|bytes| Keypair::from_secret_bytes(Algorithm::P256, &bytes).ok())
            .ok_or(bad_key)?;
        let write_collections: Vec<String> = text("MCP_WRITE_COLLECTIONS")
            .unwrap_or_default()
            .split(',')
            .map(|collection| collection.trim().to_owned())
            .filter(|collection| !collection.is_empty())
            .collect();
        if !write_collections
            .iter()
            .all(|collection| crate::store::is_nsid(collection))
        {
            return Err(ConfigError::Invalid {
                name: "MCP_WRITE_COLLECTIONS",
                reason: "expected NSIDs separated by commas",
            });
        }
        Ok(Self {
            address,
            write_collections,
            public_url: origin("MCP_PUBLIC_URL", required("MCP_PUBLIC_URL")?)?,
            pds_public_url,
            pds_internal_url,
            client_key,
        })
    }

    /// The address of the MCP endpoint, which is the OAuth resource.
    pub fn resource(&self) -> String {
        format!("{}{MCP_PATH}", self.public_url)
    }

    /// The client id of the server at the PDS: the address of its
    /// metadata document.
    pub fn client_id(&self) -> String {
        format!("{}{CLIENT_METADATA_PATH}", self.public_url)
    }

    /// Where the PDS sends the browser after the authorization.
    pub fn redirect_uri(&self) -> String {
        format!("{}{PDS_CALLBACK_PATH}", self.public_url)
    }

    /// The name of the client key in the key set: the first 16 characters
    /// of its thumbprint.
    pub fn key_id(&self) -> String {
        let jwk = pds_oauth::agent::public_jwk(&self.client_key);
        let thumbprint = pds_oauth::dpop::jwk_thumbprint(&jwk).unwrap_or_default();
        thumbprint.chars().take(16).collect()
    }

    /// The scopes an AI client may ask for.
    pub fn scopes(&self) -> Vec<String> {
        crate::store::SCOPES
            .iter()
            .map(|scope| (*scope).to_owned())
            .chain(
                self.write_collections
                    .iter()
                    .map(|collection| format!("repo:write:{collection}")),
            )
            .collect()
    }

    /// The metadata document of the server as a confidential OAuth client
    /// of the PDS (SPEC §22.2), with its key set inline.
    pub fn client_metadata(&self) -> Value {
        let mut jwk = pds_oauth::agent::public_jwk(&self.client_key);
        jwk["kid"] = json!(self.key_id());
        jwk["alg"] = json!("ES256");
        jwk["use"] = json!("sig");
        let mut scope = "atproto repo:*".to_owned();
        for collection in &self.write_collections {
            scope.push_str(" repo:");
            scope.push_str(collection);
        }
        json!({
            "client_id": self.client_id(),
            "client_name": "LinkJar MCP",
            "client_uri": self.public_url,
            "application_type": "web",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "redirect_uris": [self.redirect_uri()],
            "scope": scope,
            "token_endpoint_auth_method": "private_key_jwt",
            "token_endpoint_auth_signing_alg": "ES256",
            "dpop_bound_access_tokens": true,
            "jwks": { "keys": [jwk] },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    const GOOD: &[(&str, &str)] = &[
        ("MCP_PUBLIC_URL", "https://mcp.example.com"),
        ("MCP_PDS_PUBLIC_URL", "https://pds.example.com"),
        (
            "MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX",
            "0505050505050505050505050505050505050505050505050505050505050505",
        ),
    ];

    #[test]
    fn the_configuration_gives_the_client_the_pds_sees() {
        let config = Config::from_vars(&vars(GOOD)).unwrap();
        assert_eq!(config.resource(), "https://mcp.example.com/mcp");
        assert_eq!(config.pds_internal_url, "https://pds.example.com");
        let metadata = config.client_metadata();
        let id = "https://mcp.example.com/oauth-client-metadata.json";
        assert_eq!(metadata["client_id"], id);
        // The PDS takes the document as it takes a fetched one.
        let client = pds_oauth::client::validate(id, &metadata).unwrap();
        assert_eq!(client.auth_method(), "private_key_jwt");
        assert_eq!(metadata["jwks"]["keys"][0]["kid"], config.key_id().as_str());
        assert_eq!(metadata["scope"], "atproto repo:*");
        let mut with = vars(GOOD);
        with.insert(
            "MCP_WRITE_COLLECTIONS".to_owned(),
            "com.example.note, com.example.link".to_owned(),
        );
        let config = Config::from_vars(&with).unwrap();
        assert_eq!(
            config.client_metadata()["scope"],
            "atproto repo:* repo:com.example.note repo:com.example.link"
        );
        assert!(
            config
                .scopes()
                .contains(&"repo:write:com.example.link".to_owned())
        );
        pds_oauth::client::validate(id, &config.client_metadata()).unwrap();
        assert!(!format!("{config:?}").contains("client_key"));
    }

    #[test]
    fn a_missing_or_wrong_value_is_refused() {
        let mut missing = vars(GOOD);
        missing.remove("MCP_PDS_PUBLIC_URL");
        assert!(matches!(
            Config::from_vars(&missing),
            Err(ConfigError::Missing("MCP_PDS_PUBLIC_URL"))
        ));
        for (name, value) in [
            ("MCP_PUBLIC_URL", "http://mcp.example.com"),
            ("MCP_PUBLIC_URL", "https://mcp.example.com/path"),
            ("MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX", "abcd"),
            ("MCP_ADDRESS", "nowhere"),
            ("MCP_WRITE_COLLECTIONS", "note"),
        ] {
            let mut wrong = vars(GOOD);
            wrong.insert(name.to_owned(), value.to_owned());
            assert!(
                matches!(Config::from_vars(&wrong), Err(ConfigError::Invalid { name: n, .. }) if n == name),
                "{name}={value}"
            );
        }
    }
}
