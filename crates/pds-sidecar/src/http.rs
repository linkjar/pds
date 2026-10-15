//! The requests and the answers of the pages of a sidecar: the query, the
//! cookies, the forms, a redirect and a JSON body.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;

/// A parameter of the query. An empty value is no value.
pub fn param(query: Option<&str>, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(key, value)| key == name && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// The value of the cookie `name`.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

/// A 303 to `to` that no cache keeps. An address that is not a header
/// value sends the browser to `/`.
pub fn redirect(to: &str) -> Response {
    let location = HeaderValue::from_str(to).unwrap_or_else(|_| HeaderValue::from_static("/"));
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, location),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
    )
        .into_response()
}

/// A JSON answer that no cache keeps.
pub fn json_response(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body.to_string(),
    )
        .into_response()
}

/// The fields of a form of at most `limit` bytes, each value trimmed.
/// `None` when the body is larger.
pub async fn form(body: Body, limit: usize) -> Option<Vec<(String, String)>> {
    let bytes = axum::body::to_bytes(body, limit).await.ok()?;
    Some(
        url::form_urlencoded::parse(&bytes)
            .map(|(key, value)| (key.into_owned(), value.trim().to_owned()))
            .collect(),
    )
}

/// A field of a form, or `""`.
pub fn field<'a>(fields: &'a [(String, String)], name: &str) -> &'a str {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map_or("", |(_, value)| value.as_str())
}

/// Whether a form comes from a page of this sidecar: the browser names
/// `public_url` as the origin of the request, or names no origin.
pub fn same_origin(headers: &HeaderMap, public_url: &str) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|origin| origin == public_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parameter_is_read_from_the_query_and_a_cookie_from_the_header() {
        assert_eq!(param(Some("a=1&b=&c=x%20y"), "c"), Some("x y".to_owned()));
        assert_eq!(param(Some("a=1&b="), "b"), None);
        assert_eq!(param(None, "a"), None);
        let mut headers = HeaderMap::new();
        headers.append(header::COOKIE, HeaderValue::from_static("one=1; two=2"));
        headers.append(header::COOKIE, HeaderValue::from_static("three=3"));
        assert_eq!(cookie(&headers, "two"), Some("2".to_owned()));
        assert_eq!(cookie(&headers, "three"), Some("3".to_owned()));
        assert_eq!(cookie(&headers, "four"), None);
    }

    #[tokio::test]
    async fn a_form_is_read_within_its_limit() {
        let fields = form(Body::from("csrf=abc&reason=+spaced+&x=1"), 64)
            .await
            .unwrap();
        assert_eq!(field(&fields, "reason"), "spaced");
        assert_eq!(field(&fields, "csrf"), "abc");
        assert_eq!(field(&fields, "missing"), "");
        assert!(form(Body::from("a=1&b=2"), 4).await.is_none());
    }

    #[test]
    fn a_form_comes_from_this_sidecar_or_from_no_named_origin() {
        let mut headers = HeaderMap::new();
        assert!(same_origin(&headers, "http://127.0.0.1:2585"));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:2585"),
        );
        assert!(same_origin(&headers, "http://127.0.0.1:2585"));
        assert!(!same_origin(&headers, "https://mcp.example.com"));
        let response = redirect("\n");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::LOCATION], "/");
    }
}
