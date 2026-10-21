use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

struct Answers {
    answers: Vec<Vec<SocketAddr>>,
    calls: AtomicUsize,
}

impl Resolve for Answers {
    fn resolve(&self, _: Name) -> Resolving {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = self.answers.get(call).cloned().unwrap_or_default();
        Box::pin(async move { Ok(Box::new(answer.into_iter()) as Addrs) })
    }
}

fn answers(values: &[&[&str]]) -> Arc<Answers> {
    Arc::new(Answers {
        answers: values
            .iter()
            .map(|answer| {
                answer
                    .iter()
                    .map(|ip| SocketAddr::new(ip.parse().unwrap(), 0))
                    .collect()
            })
            .collect(),
        calls: AtomicUsize::new(0),
    })
}

#[tokio::test]
async fn the_connector_uses_the_checked_answer_and_rechecks_new_resolutions() {
    // An attacker supplies a public answer, then rebinds to loopback.
    let source = answers(&[&["8.8.8.8"], &["127.0.0.1"]]);
    let resolver = CheckedResolver {
        source: source.clone(),
        policy: Policy::Public,
    };
    let checked: Vec<_> = resolver
        .resolve("untrusted.test".parse().unwrap())
        .await
        .unwrap()
        .collect();
    assert_eq!(checked, vec!["8.8.8.8:0".parse().unwrap()]);
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert!(
        resolver
            .resolve("untrusted.test".parse().unwrap())
            .await
            .is_err()
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn empty_or_mixed_public_private_answers_are_refused() {
    for response in [
        &[][..],
        &["8.8.8.8", "127.0.0.1"][..],
        &["::ffff:10.0.0.1"][..],
    ] {
        let resolver = CheckedResolver {
            source: answers(&[response]),
            policy: Policy::Public,
        };
        assert!(
            resolver
                .resolve("untrusted.test".parse().unwrap())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn private_dns_answers_never_reach_a_socket() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = answers(&[&["127.0.0.1"]]);
    let http = client(Policy::Public, source.clone()).unwrap();
    // Bypass only the URL guard here, to exercise the connector itself.
    let result = http
        .get(format!(
            "http://untrusted.test:{}/",
            listener.local_addr().unwrap().port()
        ))
        .send()
        .await;
    assert!(result.is_err());
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

fn response_server(response: &str) -> (String, std::thread::JoinHandle<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    let response = response.to_owned();
    let task = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            assert_eq!(stream.read(&mut byte).unwrap(), 1);
            request.push(byte[0]);
        }
        let _ = stream.write_all(response.as_bytes());
        String::from_utf8(request).unwrap()
    });
    (url, task)
}

#[tokio::test]
async fn one_lookup_connects_to_the_returned_address_and_preserves_the_host() {
    let (url, server) =
        response_server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
    let url = url.replace("127.0.0.1", "untrusted.test");
    let source = answers(&[&["127.0.0.1"], &["127.0.0.2"]]);
    let http = client(Policy::Development, source.clone()).unwrap();
    let response = http.get(&url).send().await.unwrap();
    assert_eq!(read_bounded(response, 2).await.unwrap(), b"{}");
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert!(server.join().unwrap().contains("host: untrusted.test:"));
}

#[tokio::test]
async fn body_limits_apply_with_and_without_content_length() {
    for response in [
        "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n12345",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\n123\r\n2\r\n45\r\n0\r\n\r\n",
    ] {
        let (url, server) = response_server(response);
        let response = request(Method::GET, &url, Policy::Development)
            .unwrap()
            .send()
            .await
            .unwrap();
        assert!(matches!(
            read_bounded(response, 4).await,
            Err(FetchError::TooLarge(4))
        ));
        server.join().unwrap();
    }
}

#[tokio::test]
async fn redirects_are_not_followed_even_in_development() {
    let destination = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (url, server) = response_server(&format!(
        "HTTP/1.1 302 Found\r\nLocation: http://{}/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        destination.local_addr().unwrap()
    ));
    let response = request(Method::GET, &url, Policy::Development)
        .unwrap()
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FOUND);
    server.join().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), destination.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn request_deadlines_cover_dns_and_the_response_body() {
    struct Pending;
    impl Resolve for Pending {
        fn resolve(&self, _: Name) -> Resolving {
            Box::pin(std::future::pending())
        }
    }
    let http = client(Policy::Public, Arc::new(Pending)).unwrap();
    let error = http
        .get("https://untrusted.test/")
        .timeout(Duration::from_millis(20))
        .send()
        .await
        .unwrap_err();
    assert!(error.is_timeout());

    // The headers arrive immediately, but the promised body never does.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (release, held) = std::sync::mpsc::channel::<()>();
    let task = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            assert_eq!(stream.read(&mut byte).unwrap(), 1);
            request.push(byte[0]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
            .unwrap();
        // Keep the body pending until the client observes its timeout.
        held.recv_timeout(Duration::from_secs(2)).unwrap();
    });
    let response = request(Method::GET, &url, Policy::Development)
        .unwrap()
        .timeout(Duration::from_millis(100))
        .send()
        .await
        .unwrap();
    assert!(
        matches!(read_bounded(response, 1).await, Err(FetchError::Request(error)) if error.is_timeout())
    );
    release.send(()).unwrap();
    task.join().unwrap();
}

#[test]
fn special_use_ranges_are_not_public() {
    for address in [
        "0.1.2.3",
        "10.1.2.3",
        "100.64.0.1",
        "127.0.0.2",
        "169.254.169.254",
        "172.16.0.1",
        "192.0.0.1",
        "192.0.2.1",
        "192.88.99.1",
        "192.168.0.1",
        "198.18.0.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "240.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "::ffff:127.0.0.1",
        "fc00::1",
        "fe80::1",
        "ff02::1",
        "64:ff9b::a00:1",
        "2001::1",
        "2001:db8::1",
        "2002:7f00:1::1",
        "3fff::1",
    ] {
        assert!(!is_public(address.parse().unwrap()), "{address}");
    }
    for address in [
        "1.1.1.1",
        "8.8.8.8",
        "100.128.0.1",
        "2606:4700::1111",
        "::ffff:8.8.8.8",
    ] {
        assert!(is_public(address.parse().unwrap()), "{address}");
    }
}

#[test]
fn public_urls_cannot_bypass_dns_checks_with_literals_or_alternate_syntax() {
    for url in [
        "http://public.test/",
        "https://public.test:8443/",
        "https://user:pass@public.test/",
        "https://public.test/#fragment",
        "https://127.1/",
        "https://2130706433/",
        "https://0x7f000001/",
        "https://[::ffff:127.0.0.1]/",
        "https://localhost./",
        "https://metadata.local/",
        "file:///etc/passwd",
    ] {
        assert!(destination(url, Policy::Public).is_err(), "{url}");
    }
    assert!(destination("https://public.test/path", Policy::Public).is_ok());
    assert!(destination("http://127.0.0.1:1234/path", Policy::Development).is_ok());
}
