#![no_main]
//! What a client controls in a read request: the query string, the
//! `Authorization` header and the `listRepos` cursor.
use http::{HeaderMap, HeaderValue, header};
use libfuzzer_sys::fuzz_target;
use pds_xrpc::api::sync::parse_repo_cursor;
use pds_xrpc::auth::{AuthConfig, SCOPE_TAKENDOWN, optional_credentials};
use pds_xrpc::params::{Format, Params, Spec};

/// Every kind and format of parameter that a served method declares.
const SPECS: &[Spec] = &[
    Spec::required("did", Format::Did),
    Spec::optional("repo", Format::AtIdentifier),
    Spec::optional("handle", Format::Handle),
    Spec::optional("collection", Format::Nsid),
    Spec::optional("rkey", Format::RecordKey),
    Spec::optional("since", Format::Tid),
    Spec::optional("cid", Format::Cid),
    Spec::list("cids", Format::Cid),
    Spec::integer("limit", 1, 1000, 500),
    Spec::optional("cursor", Format::Any),
    Spec::boolean("reverse", false),
];

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        // A refusal is an answer; its message quotes the input and must
        // still be a message.
        if let Err(error) = Params::parse("com.example.method", Some(text), SPECS) {
            assert_eq!(error.status.as_u16(), 400);
            assert!(error.message.starts_with("Invalid com.example.method params: "));
        }
        if let Ok((time, did)) = parse_repo_cursor(text) {
            // The time is the one format the query compares as text.
            assert_eq!(time.len(), "1970-01-01T00:00:00.000Z".len());
            assert!(!did.is_empty());
        }
    }
    if let Ok(value) = HeaderValue::from_bytes(data) {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, value);
        let config = AuthConfig {
            jwt_secret: b"fuzz".to_vec(),
            admin_password: "fuzz".to_owned(),
            service_did: "did:web:pds.test".to_owned(),
        };
        // No input is a credential here: the secret is not in the corpus.
        let _ = optional_credentials(&headers, &config, &[SCOPE_TAKENDOWN], 1_700_000_000);
    }
});
