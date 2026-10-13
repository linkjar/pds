//! The configuration of the console, from `CONSOLE_*` variables.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use url::Url;

/// The scope the console asks for. It reads and writes no repository: the
/// session only names the operator.
pub const SCOPE: &str = "atproto";

/// The path of the redirect URI.
pub const CALLBACK_PATH: &str = "/oauth/callback";

/// Why the configuration is not usable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// A variable that the console needs is not set.
    #[error("{0} is not set")]
    Missing(&'static str),
    /// A variable has a value the console cannot use.
    #[error("{name} is {value:?}: {reason}")]
    Invalid {
        /// The variable.
        name: &'static str,
        /// What it holds.
        value: String,
        /// What is wrong with it.
        reason: &'static str,
    },
}

/// What the console needs to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `CONSOLE_ADDRESS`: where the console listens. Default
    /// `127.0.0.1:2585`.
    pub address: SocketAddr,
    /// `CONSOLE_PUBLIC_URL`: the address the operator's browser uses, a
    /// loopback address such as `http://127.0.0.1:2585`.
    pub public_url: String,
    /// `CONSOLE_PDS_PUBLIC_URL`: the public address of the PDS, which is
    /// its OAuth issuer.
    pub pds_public_url: String,
    /// `CONSOLE_PDS_INTERNAL_URL`: where the console reaches the PDS.
    /// Default: the public address.
    pub pds_internal_url: String,
    /// `CONSOLE_OPS_URL`: the ops listener of the PDS.
    pub ops_url: String,
}

/// An `http` or `https` origin without a path, a query or a fragment.
fn origin(name: &'static str, value: &str) -> Result<String, ConfigError> {
    let invalid = |reason| ConfigError::Invalid {
        name,
        value: value.to_owned(),
        reason,
    };
    let url = Url::parse(value).map_err(|_| invalid("expected a URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("expected http or https"));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("expected an origin without a path"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("expected an origin without credentials"));
    }
    Ok(value.trim_end_matches('/').to_owned())
}

impl Config {
    /// Reads the configuration from the variables.
    pub fn from_vars(vars: &BTreeMap<String, String>) -> Result<Self, ConfigError> {
        let text = |name: &'static str| {
            vars.get(name)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
        };
        let required = |name: &'static str| {
            text(name)
                .ok_or(ConfigError::Missing(name))
                .and_then(|value| origin(name, value))
        };
        let address = text("CONSOLE_ADDRESS").unwrap_or("127.0.0.1:2585");
        let address = address.parse().map_err(|_| ConfigError::Invalid {
            name: "CONSOLE_ADDRESS",
            value: address.to_owned(),
            reason: "expected an address and a port, such as 127.0.0.1:2585",
        })?;
        let public_url = required("CONSOLE_PUBLIC_URL")?;
        // The console is a loopback client of the PDS (SPEC §11.2): the PDS
        // fetches no metadata for it, so the console needs no public name.
        let loopback = Url::parse(&public_url).is_ok_and(|url| {
            url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
        });
        if !loopback {
            return Err(ConfigError::Invalid {
                name: "CONSOLE_PUBLIC_URL",
                value: public_url,
                reason: "expected http://127.0.0.1 or http://[::1] with a port",
            });
        }
        let pds_public_url = required("CONSOLE_PDS_PUBLIC_URL")?;
        let pds_internal_url = match text("CONSOLE_PDS_INTERNAL_URL") {
            Some(value) => origin("CONSOLE_PDS_INTERNAL_URL", value)?,
            None => pds_public_url.clone(),
        };
        Ok(Self {
            address,
            public_url,
            pds_public_url,
            pds_internal_url,
            ops_url: required("CONSOLE_OPS_URL")?,
        })
    }

    /// Where the PDS sends the browser after the authorization.
    pub fn redirect_uri(&self) -> String {
        format!("{}{CALLBACK_PATH}", self.public_url)
    }

    /// The OAuth client id of the console: the loopback form, which names
    /// the redirect URI and the scope. `PDS_OPS_CLIENT_IDS` of the PDS
    /// holds this value.
    pub fn client_id(&self) -> String {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("redirect_uri", &self.redirect_uri())
            .append_pair("scope", SCOPE)
            .finish();
        format!("http://localhost?{query}")
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
        ("CONSOLE_PUBLIC_URL", "http://127.0.0.1:2585"),
        ("CONSOLE_PDS_PUBLIC_URL", "https://pds.example.com/"),
        ("CONSOLE_OPS_URL", "http://pds:2584"),
    ];

    #[test]
    fn the_configuration_is_read_with_its_defaults() {
        let config = Config::from_vars(&vars(GOOD)).unwrap();
        assert_eq!(config.address, "127.0.0.1:2585".parse().unwrap());
        assert_eq!(config.pds_public_url, "https://pds.example.com");
        assert_eq!(config.pds_internal_url, "https://pds.example.com");
        assert_eq!(
            config.client_id(),
            "http://localhost?redirect_uri=http%3A%2F%2F127.0.0.1%3A2585%2Foauth%2Fcallback&scope=atproto"
        );
    }

    #[test]
    fn a_missing_or_wrong_value_is_refused() {
        let mut missing = vars(GOOD);
        missing.remove("CONSOLE_OPS_URL");
        assert_eq!(
            Config::from_vars(&missing),
            Err(ConfigError::Missing("CONSOLE_OPS_URL"))
        );
        for (name, value) in [
            ("CONSOLE_PUBLIC_URL", "https://console.example.com"),
            ("CONSOLE_PUBLIC_URL", "http://localhost:2585"),
            ("CONSOLE_PDS_PUBLIC_URL", "https://pds.example.com/path"),
            ("CONSOLE_OPS_URL", "ftp://pds:2584"),
            ("CONSOLE_OPS_URL", "http://user:pw@pds:2584"),
            ("CONSOLE_ADDRESS", "nowhere"),
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
