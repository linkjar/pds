//! `HEAD` through a real listener.
//!
//! The entry point treats `HEAD` as `GET` on every route. These tests hold
//! what follows from that on the wire: the status, the content type and the
//! content length of the `GET` answer, and no body. They use the routes that need no data
//! directory. `HEAD` on `getRepo` and `getBlob` needs a data directory, and
//! `crates/linkjar-pds/tests` holds the fixture for one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use pds_identity::{DidResolver, ResolverConfig};
use pds_storage::DataLayout;
use pds_xrpc::auth::AuthConfig;
use pds_xrpc::{AppState, ServerConfig};

fn state() -> AppState {
    let http = reqwest::Client::new();
    let config = ServerConfig {
        version: Some("0.0.0-test".to_owned()),
        handle_domains: vec![".test".to_owned()],
        invite_required: false,
        blob_upload_limit: 1024,
        privacy_policy_url: None,
        terms_of_service_url: None,
        contact_email: None,
        appview_url: None,
        proxy_timeout: Duration::from_secs(1),
        auth: AuthConfig {
            jwt_secret: b"secret".to_vec(),
            admin_password: "password".to_owned(),
            service_did: "did:web:pds.test".to_owned(),
        },
    };
    let resolver = DidResolver::new(
        ResolverConfig {
            plc_url: "http://127.0.0.1:9".to_owned(),
            timeout: Duration::from_secs(1),
            stale_ttl: Duration::from_secs(1),
            max_ttl: Duration::from_secs(1),
            dev_mode: true,
        },
        http.clone(),
        None,
        || 0,
    );
    // No route under test opens the data directory.
    let layout = DataLayout::new(Path::new("/nonexistent/pds-xrpc-head-test"));
    AppState::new(config, layout, None, resolver, http, || 0)
}

/// One answer as it crossed the wire: the status, the header lines in lower
/// case, and every byte after the blank line.
struct Answer {
    status: u16,
    headers: Vec<String>,
    body: Vec<u8>,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key == name).then_some(value.trim())
        })
    }
}

fn request(address: SocketAddr, method: &str, path: &str) -> Answer {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nhost: pds.test\r\nconnection: close\r\n\r\n"
    )
    .unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let head = String::from_utf8(bytes[..split].to_vec()).unwrap();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap()
        .parse()
        .unwrap();
    Answer {
        status,
        headers: lines.map(str::to_ascii_lowercase).collect(),
        body: bytes[split + 4..].to_vec(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn head_answers_as_get_without_a_body() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = pds_xrpc::router(state());
    tokio::spawn(async move { axum::serve(listener, router).await });

    // The third value is how much longer the body of the `HEAD` answer would
    // be: the "not found" page names the method.
    for (path, status, longer) in [
        ("/", 200, 0),
        ("/robots.txt", 200, 0),
        ("/nope", 404, "HEAD".len() - "GET".len()),
        ("/xrpc/com.atproto.server.describeServer", 200, 0),
        ("/xrpc/com.atproto.sync.getRepo", 400, 0),
        ("/xrpc/com.atproto.server.createSession", 401, 0),
    ] {
        let (get, head) = tokio::task::spawn_blocking(move || {
            (
                request(address, "GET", path),
                request(address, "HEAD", path),
            )
        })
        .await
        .unwrap();
        assert_eq!((get.status, head.status), (status, status), "{path}");
        assert!(!get.body.is_empty(), "{path}");
        assert!(head.body.is_empty(), "{path}");
        assert_eq!(
            head.header("content-type"),
            get.header("content-type"),
            "{path}"
        );
        assert_eq!(
            head.header("content-length"),
            Some((get.body.len() + longer).to_string().as_str()),
            "{path}"
        );
    }
}
