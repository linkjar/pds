//! OAuth clients (`docs/SPEC.md` §11.2): what a `client_id` is, and the
//! checks on a client's metadata, as `client-manager.ts` of
//! `@atproto/oauth-provider` at the pin makes them.

use serde_json::{Map, Value, json};
use url::Url;

/// Why a client is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    /// The metadata is not acceptable: `invalid_client_metadata`.
    #[error("{0}")]
    Metadata(String),
    /// A redirect URI is not acceptable: `invalid_redirect_uri`.
    #[error("{0}")]
    RedirectUri(String),
    /// The `client_id` is not one: `invalid_client_id`.
    #[error("{0}")]
    ClientId(String),
}

fn metadata(message: impl Into<String>) -> ClientError {
    ClientError::Metadata(message.into())
}

fn redirect(message: impl Into<String>) -> ClientError {
    ClientError::RedirectUri(message.into())
}

/// The kind of a `client_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientId {
    /// `http://localhost` with optional `redirect_uri` and `scope`
    /// parameters: a native client in development. The server writes its
    /// metadata.
    Loopback {
        /// The redirect URIs of the query, when it has any.
        redirect_uris: Vec<String>,
        /// The scope of the query.
        scope: Option<String>,
    },
    /// An HTTPS URL of a metadata document.
    Discoverable(Url),
}

/// A host name that only resolves inside a network: one label, or a
/// reserved top-level domain.
pub fn is_local_hostname(host: &str) -> bool {
    let labels: Vec<&str> = host.trim_end_matches('.').split('.').collect();
    labels.len() < 2
        || matches!(
            labels.last().copied().unwrap_or_default(),
            "test" | "local" | "localhost" | "invalid" | "example"
        )
}

/// A redirect URI of a native client under a scheme of its own: the
/// scheme has a dot, and the rest is one slash and a path.
fn is_private_use(url: &Url) -> bool {
    url.scheme().contains('.')
}

impl ClientId {
    /// Reads a `client_id`.
    pub fn parse(client_id: &str) -> Result<Self, ClientError> {
        let bad = |message: &str| ClientError::ClientId(message.to_owned());
        let url = Url::parse(client_id).map_err(|_| bad("Invalid client ID"))?;
        if url.fragment().is_some() {
            return Err(bad("Client ID must not contain a fragment"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(bad("Client ID must not contain credentials"));
        }
        if url.scheme() == "http" && url.host_str() == Some("localhost") {
            if url.port().is_some() {
                return Err(bad("Loopback client ID must not contain a port"));
            }
            if !matches!(url.path(), "" | "/") {
                return Err(bad("Loopback client ID must not contain a path component"));
            }
            let mut redirect_uris = Vec::new();
            let mut scope = None;
            for (name, value) in url.query_pairs() {
                match name.as_ref() {
                    "redirect_uri" => redirect_uris.push(value.into_owned()),
                    "scope" if scope.is_none() => scope = Some(value.into_owned()),
                    "scope" => return Err(bad("Duplicate \"scope\" query parameter")),
                    other => {
                        return Err(bad(&format!(
                            "Invalid query parameter \"{other}\" in client ID"
                        )));
                    }
                }
            }
            return Ok(Self::Loopback {
                redirect_uris,
                scope,
            });
        }
        if url.scheme() != "https" {
            return Err(bad("Client ID must use the \"https:\" protocol"));
        }
        let host = url.host_str().unwrap_or_default();
        if is_local_hostname(host) || host.parse::<std::net::IpAddr>().is_ok() {
            return Err(bad("Client ID hostname must be a public domain name"));
        }
        if url.path() == "/" {
            return Err(bad(
                "Client ID must contain a path component (e.g. \"/client-metadata.json\")",
            ));
        }
        if url.path().ends_with('/') {
            return Err(bad("Client ID path must not end with a trailing slash"));
        }
        Ok(Self::Discoverable(url))
    }

    /// The metadata the server writes for a loopback client.
    pub fn loopback_metadata(
        client_id: &str,
        redirect_uris: &[String],
        scope: Option<&str>,
    ) -> Value {
        let redirect_uris: Vec<String> = if redirect_uris.is_empty() {
            vec!["http://127.0.0.1/".to_owned(), "http://[::1]/".to_owned()]
        } else {
            redirect_uris.to_vec()
        };
        json!({
            "client_id": client_id,
            "scope": scope.unwrap_or("atproto"),
            "redirect_uris": redirect_uris,
            "response_types": ["code"],
            "grant_types": ["authorization_code", "refresh_token"],
            "token_endpoint_auth_method": "none",
            "application_type": "native",
            "dpop_bound_access_tokens": true,
        })
    }
}

/// A client's metadata after the checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    /// The `client_id`.
    pub id: String,
    /// The metadata as the client published it, with the defaults filled.
    pub metadata: Map<String, Value>,
}

impl Client {
    fn strings(&self, key: &str) -> Vec<&str> {
        self.metadata
            .get(key)
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    fn has(&self, key: &str) -> bool {
        self.metadata.get(key).is_some_and(|value| !value.is_null())
    }

    fn client_uri(&self) -> Result<Option<Url>, ClientError> {
        self.text("client_uri")
            .map(|uri| Url::parse(uri).map_err(|_| metadata("client_uri is not a URL")))
            .transpose()
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).and_then(Value::as_str)
    }

    /// The scopes the client may ask for.
    pub fn scopes(&self) -> Vec<&str> {
        self.text("scope").unwrap_or_default().split(' ').collect()
    }

    /// The redirect URIs.
    pub fn redirect_uris(&self) -> Vec<&str> {
        self.strings("redirect_uris")
    }

    /// The grant types.
    pub fn grant_types(&self) -> Vec<&str> {
        self.strings("grant_types")
    }

    /// `none` or `private_key_jwt`.
    pub fn auth_method(&self) -> &str {
        self.text("token_endpoint_auth_method").unwrap_or("none")
    }

    /// The name the client gives itself. An untrusted client is shown by
    /// its `client_id` and not by this.
    pub fn name(&self) -> Option<&str> {
        self.text("client_name")
    }
}

fn duplicate<'a>(items: &[&'a str]) -> Option<&'a str> {
    items
        .iter()
        .enumerate()
        .find(|(at, item)| items[..*at].contains(item))
        .map(|(_, item)| *item)
}

fn check_redirect_uris(
    uris: &[&str],
    native: bool,
    discoverable: Option<&Url>,
) -> Result<(), ClientError> {
    for uri in uris {
        let url = Url::parse(uri).map_err(|_| redirect(format!("Invalid redirect URI: {uri}")))?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(redirect(format!(
                "Redirect URI {url} must not contain credentials"
            )));
        }
        let host = url.host_str().unwrap_or_default();
        match url.scheme() {
            _ if host == "localhost" => {
                return Err(redirect(format!(
                    "Loopback redirect URI {url} is not allowed (use explicit IPs instead)"
                )));
            }
            scheme if host == "127.0.0.1" || host == "[::1]" => {
                if !native {
                    return Err(redirect(
                        "Loopback redirect URIs are only allowed for native apps",
                    ));
                }
                if scheme != "http" {
                    return Err(redirect(format!(
                        "Loopback redirect URI {url} must use HTTP"
                    )));
                }
            }
            "http" => {
                return Err(redirect(
                    "Only loopback redirect URIs are allowed to use the \"http\" scheme",
                ));
            }
            "https" => {
                if is_local_hostname(host) {
                    return Err(redirect(format!(
                        "Redirect URI \"{url}\"'s domain name must not be a local hostname"
                    )));
                }
            }
            scheme if is_private_use(&url) => {
                if !native {
                    return Err(redirect(
                        "Private-Use URI Scheme redirect URI are only allowed for native apps",
                    ));
                }
                if let Some(client) = discoverable {
                    let mut labels: Vec<&str> =
                        client.host_str().unwrap_or_default().split('.').collect();
                    labels.reverse();
                    if scheme != labels.join(".") {
                        return Err(redirect(
                            "Private-Use URI Scheme redirect URI, for discoverable client metadata, must be the fully qualified domain name (FQDN) of the client_id, in reverse order",
                        ));
                    }
                }
            }
            scheme => {
                return Err(redirect(format!(
                    "Invalid redirect URI scheme \"{scheme}:\""
                )));
            }
        }
    }
    Ok(())
}

/// Checks the metadata of the client `client_id` and fills its defaults.
pub fn validate(client_id: &str, document: &Value) -> Result<Client, ClientError> {
    let kind = ClientId::parse(client_id)?;
    let mut fields = document
        .as_object()
        .cloned()
        .ok_or_else(|| metadata("Client metadata must be a JSON object"))?;
    for (key, default) in [
        ("grant_types", json!(["authorization_code"])),
        ("response_types", json!(["code"])),
        ("application_type", json!("web")),
        ("token_endpoint_auth_method", json!("client_secret_basic")),
    ] {
        fields.entry(key.to_owned()).or_insert(default);
    }
    let client = Client {
        id: client_id.to_owned(),
        metadata: fields,
    };
    check_fields(&client)?;
    check_auth(&client)?;
    check_responses(&client)?;
    check_placement(&client, &kind)?;
    Ok(client)
}

fn check_fields(client: &Client) -> Result<(), ClientError> {
    let has = |key: &str| client.has(key);

    if has("jwks") && has("jwks_uri") {
        return Err(metadata("jwks_uri and jwks are mutually exclusive"));
    }
    for key in [
        "default_max_age",
        "userinfo_signed_response_alg",
        "id_token_signed_response_alg",
        "userinfo_encrypted_response_alg",
    ] {
        if has(key) {
            return Err(metadata(format!("Unsupported \"{key}\" parameter")));
        }
    }
    let client_uri = client.client_uri()?;
    if client_uri
        .as_ref()
        .is_some_and(|uri| is_local_hostname(uri.host_str().unwrap_or_default()))
    {
        return Err(metadata("client_uri hostname is invalid"));
    }
    if !has("scope") {
        return Err(metadata("Missing scope property"));
    }
    let scopes = client.scopes();
    if !scopes.contains(&"atproto") {
        return Err(metadata("Missing \"atproto\" scope"));
    }
    if let Some(scope) = duplicate(&scopes) {
        return Err(metadata(format!("Duplicate scope \"{scope}\"")));
    }
    let grants = client.grant_types();
    if let Some(grant) = duplicate(&grants) {
        return Err(metadata(format!("Duplicate grant type \"{grant}\"")));
    }
    for grant in &grants {
        match *grant {
            "authorization_code" | "refresh_token" => {}
            "implicit" => return Err(metadata(format!("Grant type \"{grant}\" is not allowed"))),
            _ => return Err(metadata(format!("Grant type \"{grant}\" is not supported"))),
        }
    }
    if client.text("client_id").is_some_and(|id| id != client.id) {
        return Err(metadata("client_id does not match"));
    }
    if client
        .text("subject_type")
        .is_some_and(|kind| kind != "public")
    {
        return Err(metadata("Only \"public\" subject_type is supported"));
    }
    Ok(())
}

fn check_auth(client: &Client) -> Result<(), ClientError> {
    let has = |key: &str| client.has(key);
    match client.auth_method() {
        "none" => {
            if has("token_endpoint_auth_signing_alg") {
                return Err(metadata(
                    "token_endpoint_auth_method \"none\" must not have token_endpoint_auth_signing_alg",
                ));
            }
        }
        "private_key_jwt" => {
            if !has("jwks") && !has("jwks_uri") {
                return Err(metadata(
                    "private_key_jwt auth method requires jwks or jwks_uri",
                ));
            }
            if client.metadata.get("jwks").is_some_and(|jwks| {
                jwks.get("keys")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
            }) {
                return Err(metadata(
                    "private_key_jwt auth method requires at least one key in jwks",
                ));
            }
            if !has("token_endpoint_auth_signing_alg") {
                return Err(metadata(
                    "Missing token_endpoint_auth_signing_alg client metadata",
                ));
            }
        }
        other => {
            return Err(metadata(format!(
                "Unsupported client authentication method \"{other}\". Make sure \"token_endpoint_auth_method\" is set to one of: \"none\", \"private_key_jwt\""
            )));
        }
    }
    Ok(())
}

fn check_responses(client: &Client) -> Result<(), ClientError> {
    let has = |key: &str| client.has(key);
    let grants = client.grant_types();
    if has("authorization_encrypted_response_enc") {
        return Err(metadata(
            "Encrypted authorization response is not supported",
        ));
    }
    if client
        .metadata
        .get("tls_client_certificate_bound_access_tokens")
        == Some(&Value::Bool(true))
    {
        return Err(metadata("Mutual-TLS bound access tokens are not supported"));
    }
    if client.metadata.get("dpop_bound_access_tokens") != Some(&Value::Bool(true)) {
        return Err(metadata("\"dpop_bound_access_tokens\" must be true"));
    }
    if !client.strings("response_types").contains(&"code") {
        return Err(metadata("response_types must include \"code\""));
    }
    if !grants.contains(&"authorization_code") {
        return Err(metadata(
            "The \"code\" response type requires that \"grant_types\" contains \"authorization_code\"",
        ));
    }
    if !client.strings("authorization_details_types").is_empty() {
        return Err(metadata("authorization_details_types are not supported"));
    }
    Ok(())
}

/// The redirect URIs, and the rules of the kind of client.
fn check_placement(client: &Client, kind: &ClientId) -> Result<(), ClientError> {
    let client_uri = client.client_uri()?;
    let redirect_uris = client.redirect_uris();
    if redirect_uris.is_empty() {
        return Err(metadata("At least one redirect_uri is required"));
    }
    let native = client.text("application_type") == Some("native");
    if native && client.auth_method() != "none" {
        return Err(metadata(
            "Native clients must authenticate using \"none\" method",
        ));
    }
    let discoverable = match kind {
        ClientId::Discoverable(url) => Some(url),
        ClientId::Loopback { .. } => None,
    };
    check_redirect_uris(&redirect_uris, native, discoverable)?;

    match kind {
        ClientId::Loopback { .. } => {
            if client_uri.is_some() {
                return Err(metadata("client_uri is not allowed for loopback clients"));
            }
            if !native {
                return Err(metadata(
                    "Loopback clients must have application_type \"native\"",
                ));
            }
            if client.auth_method() != "none" {
                return Err(metadata(format!(
                    "Loopback clients are not allowed to use \"token_endpoint_auth_method\" {}",
                    client.auth_method()
                )));
            }
        }
        ClientId::Discoverable(id) => {
            if client.text("client_id").is_none() {
                return Err(metadata("client_id is required for discoverable clients"));
            }
            if let Some(uri) = &client_uri {
                if uri.origin() != id.origin() {
                    return Err(metadata(
                        "client_uri must have the same origin as the client_id",
                    ));
                }
                let parent = if uri.path().ends_with('/') {
                    uri.path().to_owned()
                } else {
                    format!("{}/", uri.path())
                };
                if id.path() != uri.path() && !id.path().starts_with(&parent) {
                    return Err(metadata("client_uri must be a parent URL of the client_id"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "https://app.example.org/client-metadata.json";

    fn document() -> Value {
        json!({
            "client_id": ID,
            "client_name": "Example",
            "client_uri": "https://app.example.org",
            "redirect_uris": ["https://app.example.org/callback"],
            "scope": "atproto transition:generic",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "application_type": "web",
            "dpop_bound_access_tokens": true,
        })
    }

    #[test]
    fn a_web_client_is_accepted_and_each_defect_is_named() {
        let client = validate(ID, &document()).unwrap();
        assert_eq!(client.scopes(), ["atproto", "transition:generic"]);
        assert_eq!(client.redirect_uris(), ["https://app.example.org/callback"]);
        assert_eq!(
            (client.auth_method(), client.name()),
            ("none", Some("Example"))
        );

        let with = |key: &str, value: Value| {
            let mut doc = document();
            doc[key] = value;
            validate(ID, &doc).unwrap_err()
        };
        for (key, value, message) in [
            (
                "scope",
                json!("transition:generic"),
                "Missing \"atproto\" scope",
            ),
            (
                "scope",
                json!("atproto atproto"),
                "Duplicate scope \"atproto\"",
            ),
            (
                "grant_types",
                json!(["authorization_code", "implicit"]),
                "Grant type \"implicit\" is not allowed",
            ),
            (
                "grant_types",
                json!(["refresh_token"]),
                "The \"code\" response type requires that \"grant_types\" contains \"authorization_code\"",
            ),
            (
                "client_id",
                json!("https://other.example.org/c.json"),
                "client_id does not match",
            ),
            (
                "dpop_bound_access_tokens",
                json!(false),
                "\"dpop_bound_access_tokens\" must be true",
            ),
            (
                "token_endpoint_auth_method",
                json!("private_key_jwt"),
                "private_key_jwt auth method requires jwks or jwks_uri",
            ),
            (
                "client_uri",
                json!("https://elsewhere.example.org"),
                "client_uri must have the same origin as the client_id",
            ),
            (
                "redirect_uris",
                json!([]),
                "At least one redirect_uri is required",
            ),
        ] {
            assert_eq!(
                with(key, value),
                ClientError::Metadata(message.to_owned()),
                "{key}"
            );
        }
        for (uri, message) in [
            (
                "http://app.example.org/cb",
                "Only loopback redirect URIs are allowed to use the \"http\" scheme",
            ),
            (
                "http://127.0.0.1/cb",
                "Loopback redirect URIs are only allowed for native apps",
            ),
            (
                "https://app.test/cb",
                "Redirect URI \"https://app.test/cb\"'s domain name must not be a local hostname",
            ),
            (
                "org.example.app:/cb",
                "Private-Use URI Scheme redirect URI are only allowed for native apps",
            ),
        ] {
            assert_eq!(
                with("redirect_uris", json!([uri])),
                ClientError::RedirectUri(message.to_owned()),
                "{uri}"
            );
        }
        let mut missing = document();
        missing
            .as_object_mut()
            .unwrap()
            .remove("token_endpoint_auth_method");
        assert!(
            matches!(validate(ID, &missing), Err(ClientError::Metadata(message)) if message.starts_with("Unsupported client authentication method"))
        );
    }

    #[test]
    fn a_client_id_is_a_metadata_url_or_a_loopback() {
        assert!(matches!(ClientId::parse(ID), Ok(ClientId::Discoverable(_))));
        for bad in [
            "app.example.org",
            "http://app.example.org/c.json",
            "https://app.example.org/",
            "https://app.example.org/c.json#x",
            "https://localhost/c.json",
            "https://app.test/c.json",
            "https://192.0.2.1/c.json",
            "http://localhost:8080",
            "http://localhost/path",
            "http://localhost?other=1",
        ] {
            assert!(
                matches!(ClientId::parse(bad), Err(ClientError::ClientId(_))),
                "{bad}"
            );
        }
        let loopback =
            "http://localhost?redirect_uri=http%3A%2F%2F127.0.0.1%3A4000%2Fcb&scope=atproto";
        let ClientId::Loopback {
            redirect_uris,
            scope,
        } = ClientId::parse(loopback).unwrap()
        else {
            panic!("not a loopback client");
        };
        assert_eq!(redirect_uris, ["http://127.0.0.1:4000/cb"]);
        let generated = ClientId::loopback_metadata(loopback, &redirect_uris, scope.as_deref());
        let client = validate(loopback, &generated).unwrap();
        assert_eq!(client.auth_method(), "none");
        let default = ClientId::loopback_metadata("http://localhost", &[], None);
        assert_eq!(
            default["redirect_uris"],
            json!(["http://127.0.0.1/", "http://[::1]/"])
        );
        assert!(validate("http://localhost", &default).is_ok());

        // A native client under the client's own domain, reversed.
        let mut native = document();
        native["application_type"] = json!("native");
        native["redirect_uris"] = json!(["org.example.app:/callback"]);
        assert!(validate(ID, &native).is_ok());
        native["redirect_uris"] = json!(["com.other.app:/callback"]);
        assert!(matches!(
            validate(ID, &native),
            Err(ClientError::RedirectUri(_))
        ));
    }
}
