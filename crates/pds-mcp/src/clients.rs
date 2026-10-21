//! The AI clients of the MCP server: a registration, or a Client ID
//! Metadata Document that the server fetches (`docs/sidecars.md` §2.3).

use std::net::IpAddr;
use std::time::Duration;

use pds_identity::safe_fetch::{self, Policy};
use serde_json::Value;
use url::Url;

use crate::store::McpClient;

/// The largest metadata document.
const DOCUMENT_LIMIT: usize = 64 * 1024;

/// The most redirect URIs of a client.
const MOST_REDIRECTS: usize = 16;

/// Whether a client may take the answer of an authorization at `uri`:
/// `https`, `http` on a loopback address, or a scheme of an application.
pub fn redirect_is_allowed(uri: &str) -> bool {
    let Ok(url) = Url::parse(uri) else {
        return false;
    };
    if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.scheme() {
        "https" => url.host_str().is_some(),
        "http" => matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost")),
        "javascript" | "data" | "file" | "vbscript" | "blob" | "about" => false,
        _ => true,
    }
}

/// Whether `uri` is a redirect URI of the client. A loopback URI matches
/// with any port (RFC 8252 §7.3).
pub fn has_redirect(client: &McpClient, uri: &str) -> bool {
    let Ok(given) = Url::parse(uri) else {
        return false;
    };
    client.redirect_uris.iter().any(|registered| {
        if registered == uri {
            return true;
        }
        let Ok(own) = Url::parse(registered) else {
            return false;
        };
        let loopback = own.scheme() == "http"
            && matches!(own.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
        loopback
            && given.scheme() == "http"
            && given.host_str() == own.host_str()
            && given.path() == own.path()
            && given.query() == own.query()
            && given.fragment().is_none()
    })
}

/// The redirect URIs and the name of a registration or of a metadata
/// document.
pub fn read_metadata(document: &Value) -> Result<(Option<String>, Vec<String>), String> {
    let uris = document
        .get("redirect_uris")
        .and_then(Value::as_array)
        .ok_or("redirect_uris must be an array")?;
    if uris.is_empty() || uris.len() > MOST_REDIRECTS {
        return Err("redirect_uris must have between 1 and 16 entries".to_owned());
    }
    let mut redirect_uris = Vec::new();
    for uri in uris {
        let uri = uri.as_str().ok_or("a redirect URI must be a string")?;
        if uri.len() > 2048 || !redirect_is_allowed(uri) {
            return Err(format!("the redirect URI {uri:?} is not allowed"));
        }
        redirect_uris.push(uri.to_owned());
    }
    let name = document
        .get("client_name")
        .and_then(Value::as_str)
        .map(|name| name.chars().take(100).collect());
    Ok((name, redirect_uris))
}

/// Whether a client id is the address of a metadata document: `https`, a
/// host name with a dot, and a path.
pub fn is_document_url(client_id: &str) -> Option<Url> {
    let url = Url::parse(client_id).ok()?;
    let host = url.host_str()?;
    let named = host.contains('.') && host.parse::<IpAddr>().is_err() && !host.starts_with('[');
    (url.scheme() == "https"
        && named
        && url.path().len() > 1
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}

/// Fetches the metadata document of a client: a public address, no
/// redirect, status 200, JSON, at most 64 KiB, and a `client_id` that is
/// the address.
pub async fn fetch(client_id: &str, url: &Url) -> Result<McpClient, String> {
    let response = safe_fetch::request(reqwest::Method::GET, url.as_str(), Policy::Public)
        .map_err(|_| "the host of the client is not on the public internet")?
        .header("accept", "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| "the client metadata could not be fetched")?;
    if response.status() != reqwest::StatusCode::OK {
        return Err("the client metadata could not be fetched".to_owned());
    }
    let bytes = safe_fetch::read_bounded(response, DOCUMENT_LIMIT)
        .await
        .map_err(|_| "the client metadata request failed or the document is too large")?;
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|_| "the client metadata is not JSON")?;
    if document.get("client_id").and_then(Value::as_str) != Some(client_id) {
        return Err("the client metadata names another client_id".to_owned());
    }
    let (name, redirect_uris) = read_metadata(&document)?;
    Ok(McpClient {
        id: client_id.to_owned(),
        name,
        redirect_uris,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn metadata_fetch_refuses_private_destinations() {
        for address in [
            "https://127.0.0.1/client.json",
            "https://[::1]/client.json",
            "https://169.254.169.254/client.json",
        ] {
            let url = Url::parse(address).unwrap();
            let error = fetch(address, &url).await.err().unwrap();
            assert_eq!(
                error,
                "the host of the client is not on the public internet"
            );
        }
    }

    #[test]
    fn a_redirect_uri_is_checked_and_matched() {
        for good in [
            "https://app.example/cb",
            "http://127.0.0.1:3333/cb",
            "http://localhost/cb",
            "cursor://anysphere.cursor-retrieval/oauth/callback",
        ] {
            assert!(redirect_is_allowed(good), "{good}");
        }
        for bad in [
            "http://app.example/cb",
            "javascript:alert(1)",
            "data:text/html,x",
            "https://user:pw@app.example/cb",
            "https://app.example/cb#f",
            "not a url",
        ] {
            assert!(!redirect_is_allowed(bad), "{bad}");
        }
        let client = McpClient {
            id: "c".to_owned(),
            name: None,
            redirect_uris: vec![
                "https://app.example/cb".to_owned(),
                "http://127.0.0.1/cb".to_owned(),
            ],
        };
        assert!(has_redirect(&client, "https://app.example/cb"));
        assert!(has_redirect(&client, "http://127.0.0.1:49152/cb"));
        assert!(!has_redirect(&client, "https://app.example/cb?x=1"));
        assert!(!has_redirect(&client, "http://127.0.0.1:49152/other"));
        assert!(!has_redirect(&client, "https://evil.example/cb"));
    }

    #[test]
    fn metadata_names_redirect_uris_and_a_document_has_a_public_name() {
        let good = json!({"client_name": "Inspector", "redirect_uris": ["http://localhost:6274/oauth/callback"]});
        let (name, uris) = read_metadata(&good).unwrap();
        assert_eq!(name.as_deref(), Some("Inspector"));
        assert_eq!(uris.len(), 1);
        assert!(read_metadata(&json!({})).is_err());
        assert!(read_metadata(&json!({"redirect_uris": []})).is_err());
        assert!(read_metadata(&json!({"redirect_uris": ["javascript:alert(1)"]})).is_err());

        assert!(is_document_url("https://client.example/metadata.json").is_some());
        for bad in [
            "http://client.example/metadata.json",
            "https://client.example",
            "https://10.0.0.1/metadata.json",
            "https://localhost/metadata.json",
            "mcp-client-abc",
        ] {
            assert!(is_document_url(bad).is_none(), "{bad}");
        }
    }
}
