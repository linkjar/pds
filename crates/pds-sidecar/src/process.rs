//! What a process of LinkJar does at its start and at its end: it logs,
//! it stops on a signal, and it answers the health check of its image.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

/// Logs as JSON lines on standard error, at the level of `LOG_LEVEL` or
/// `info`. Standard output belongs to the commands of a binary.
pub fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("LOG_LEVEL")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

/// Resolves on `SIGTERM` or an interrupt (SPEC §14.3).
pub async fn shutdown() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}

/// Asks the sidecar that listens on `address` for its `/health` route, as
/// the health check of its image does.
///
/// `Ok` when the route answers 200 within three seconds. A sidecar that
/// listens on every address is asked on loopback. The error names what
/// went wrong, for the check's log.
pub async fn health(address: SocketAddr) -> Result<(), String> {
    let mut target = address;
    if address.ip().is_unspecified() {
        target.set_ip(match address.ip() {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        });
    }
    let url = format!("http://{target}/health");
    // The request goes to this host, never to a proxy from the environment.
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())?;
    let status = client
        .get(&url)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map_err(|error| format!("{url}: {error}"))?
        .status();
    if status == reqwest::StatusCode::OK {
        Ok(())
    } else {
        Err(format!("{url} answered {status}"))
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    /// One answer to one request, with `status`.
    async fn answer_once(status: &'static str) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") && socket.read(&mut byte).await.unwrap() == 1 {
                request.push(byte[0]);
            }
            assert!(request.starts_with(b"GET /health HTTP/1.1\r\n"));
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        address
    }

    #[tokio::test]
    async fn the_health_check_passes_on_200_and_on_nothing_else() {
        // The listener is on every address; the check asks loopback.
        assert_eq!(health(answer_once("200 OK").await).await, Ok(()));
        let answered = health(answer_once("503 Service Unavailable").await).await;
        assert!(
            answered
                .unwrap_err()
                .ends_with("answered 503 Service Unavailable")
        );
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = closed.local_addr().unwrap();
        drop(closed);
        assert!(health(address).await.is_err());
    }
}
