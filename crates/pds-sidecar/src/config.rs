//! The checks on the configuration of a sidecar.

use url::Url;

/// An `http` or `https` origin without a path, a query, a fragment or
/// credentials, as its text without the trailing slash. Otherwise the
/// reason, for the error of the configuration.
pub fn origin(value: &str) -> Result<String, &'static str> {
    let url = Url::parse(value).map_err(|_| "expected a URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("expected http or https");
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err("expected an origin without a path");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("expected an origin without credentials");
    }
    Ok(value.trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_origin_is_a_scheme_and_a_host_and_no_more() {
        assert_eq!(
            origin("https://pds.example.com/"),
            Ok("https://pds.example.com".to_owned())
        );
        assert_eq!(
            origin("http://127.0.0.1:2584"),
            Ok("http://127.0.0.1:2584".to_owned())
        );
        assert_eq!(origin("pds.example.com"), Err("expected a URL"));
        assert_eq!(
            origin("ftp://pds.example.com"),
            Err("expected http or https")
        );
        assert_eq!(
            origin("https://pds.example.com/xrpc"),
            Err("expected an origin without a path")
        );
        assert_eq!(
            origin("https://pds.example.com/?a=1"),
            Err("expected an origin without a path")
        );
        assert_eq!(
            origin("https://pds.example.com/#top"),
            Err("expected an origin without a path")
        );
        assert_eq!(
            origin("https://user:pw@pds.example.com"),
            Err("expected an origin without credentials")
        );
    }
}
