//! Response bodies produced on the blocking pool.
//!
//! SQLite and file reads are synchronous (SPEC §3.2). A large answer is
//! written on a blocking thread into a bounded channel, and the response
//! body reads from the channel, so the server never holds a whole export in
//! memory.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::{Body, Bytes};
use futures_util::stream;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, error::SendTimeoutError};

/// The size at which buffered output is handed to the response.
const CHUNK_LEN: usize = 64 * 1024;
/// How many chunks may wait for the client.
const QUEUE_LEN: usize = 4;
/// How long one chunk may wait for a place in the queue. A client that
/// takes nothing for this long loses the answer, so that it does not hold a
/// blocking thread and an open store without limit.
const STALL: Duration = Duration::from_secs(60);

/// The write end of a streamed body.
#[derive(Debug)]
pub struct BodyWriter {
    buffer: Vec<u8>,
    sender: mpsc::Sender<Bytes>,
    runtime: Handle,
    stall: Duration,
}

impl BodyWriter {
    fn send(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(std::mem::replace(
            &mut self.buffer,
            Vec::with_capacity(CHUNK_LEN),
        ));
        self.runtime
            .block_on(self.sender.send_timeout(chunk, self.stall))
            .map_err(|error| match error {
                // The receiver is gone when the client went away.
                SendTimeoutError::Closed(_) => io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "the client closed the connection",
                ),
                SendTimeoutError::Timeout(_) => {
                    io::Error::new(io::ErrorKind::TimedOut, "the client stopped reading")
                }
            })
    }
}

impl io::Write for BodyWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= CHUNK_LEN {
            self.send()?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send()
    }
}

/// A body that `produce` writes on the blocking pool.
///
/// The body ends without an error only when `produce` has returned `Ok` and
/// its last byte is in the queue. When `produce` fails or panics, or the
/// client takes nothing for the time of `STALL`, the body ends with an error, which
/// closes the connection: the client sees a truncated answer and not a
/// complete one.
///
/// Call it inside the Tokio runtime that serves the response.
pub fn blocking_body<F>(produce: F) -> Body
where
    F: FnOnce(&mut BodyWriter) -> io::Result<()> + Send + 'static,
{
    body_with_stall(STALL, produce)
}

fn body_with_stall<F>(stall: Duration, produce: F) -> Body
where
    F: FnOnce(&mut BodyWriter) -> io::Result<()> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel::<Bytes>(QUEUE_LEN);
    // The producer sets this after its last chunk is in the queue. A panic,
    // an error and a stall all drop the sender with the flag unset.
    let finished = Arc::new(AtomicBool::new(false));
    let produced = Arc::clone(&finished);
    let runtime = Handle::current();
    tokio::task::spawn_blocking(move || {
        let mut writer = BodyWriter {
            buffer: Vec::with_capacity(CHUNK_LEN),
            sender,
            runtime,
            stall,
        };
        match produce(&mut writer).and_then(|()| writer.send()) {
            Ok(()) => produced.store(true, Ordering::Release),
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {}
            Err(error) => tracing::error!(%error, "a streamed response failed"),
        }
    });
    Body::from_stream(stream::unfold(Some(receiver), move |receiver| {
        let finished = Arc::clone(&finished);
        async move {
            let mut receiver = receiver?;
            match receiver.recv().await {
                Some(chunk) => Some((Ok(chunk), Some(receiver))),
                None if finished.load(Ordering::Acquire) => None,
                None => Some((
                    Err(io::Error::other(
                        "the producer of the body stopped before the end",
                    )),
                    None,
                )),
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::mpsc::channel;

    use axum::body::to_bytes;

    use super::*;

    /// More than the queue and one chunk in the producer's hand can hold.
    const TOO_MUCH: usize = (QUEUE_LEN + 3) * CHUNK_LEN;

    /// Writes `TOO_MUCH` bytes, one chunk at a time.
    fn too_much(out: &mut BodyWriter) -> io::Result<()> {
        let chunk = vec![7_u8; CHUNK_LEN];
        for _ in 0..TOO_MUCH / CHUNK_LEN {
            out.write_all(&chunk)?;
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_finished_body_is_whole() {
        let body = blocking_body(too_much);
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        assert_eq!(bytes.len(), TOO_MUCH);
        assert!(bytes.iter().all(|byte| *byte == 7));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_producer_ends_the_body_with_an_error() {
        let body = blocking_body(|out| {
            out.write_all(b"a short start")?;
            out.flush()?;
            Err(io::Error::other("the store failed"))
        });
        assert!(to_bytes(body, usize::MAX).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_producer_panic_ends_the_body_with_an_error() {
        let body = blocking_body(|out| {
            out.write_all(b"a short start")?;
            out.flush()?;
            panic!("the producer panics");
        });
        assert!(to_bytes(body, usize::MAX).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stalled_client_releases_the_producer_and_loses_the_answer() {
        let (report, outcome) = channel();
        let body = body_with_stall(Duration::from_millis(50), move |out| {
            let result = too_much(out);
            let _ = report.send(result.as_ref().map_err(io::Error::kind).copied());
            result
        });
        // Nothing reads the body until the producer has given up.
        let outcome = tokio::task::spawn_blocking(move || outcome.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome, Err(io::ErrorKind::TimedOut));
        assert!(to_bytes(body, usize::MAX).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dropped_body_stops_the_producer() {
        let (report, outcome) = channel();
        let (started, start) = channel::<()>();
        let body = blocking_body(move |out| {
            let _ = start.recv();
            let result = too_much(out);
            let _ = report.send(result.as_ref().map_err(io::Error::kind).copied());
            result
        });
        drop(body);
        started.send(()).unwrap();
        let outcome = tokio::task::spawn_blocking(move || outcome.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome, Err(io::ErrorKind::BrokenPipe));
    }
}
