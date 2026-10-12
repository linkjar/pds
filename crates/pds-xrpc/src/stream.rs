//! Response bodies produced on the blocking pool.
//!
//! SQLite and file reads are synchronous (SPEC §3.2). A large answer is
//! written on a blocking thread into a bounded channel, and the response
//! body reads from the channel, so the server never holds a whole export in
//! memory.

use std::io;

use axum::body::{Body, Bytes};
use futures_util::stream;
use tokio::sync::mpsc;

/// The size at which buffered output is handed to the response.
const CHUNK_LEN: usize = 64 * 1024;
/// How many chunks may wait for the client.
const QUEUE_LEN: usize = 4;

type Item = Result<Bytes, io::Error>;

/// The write end of a streamed body.
#[derive(Debug)]
pub struct BodyWriter {
    buffer: Vec<u8>,
    sender: mpsc::Sender<Item>,
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
        // The receiver is gone when the client went away.
        self.sender.blocking_send(Ok(chunk)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the client closed the connection",
            )
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
/// When `produce` fails after the status line has gone out, the body ends
/// with an error, which closes the connection: the client sees a truncated
/// answer and not a complete one.
pub fn blocking_body<F>(produce: F) -> Body
where
    F: FnOnce(&mut BodyWriter) -> io::Result<()> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel::<Item>(QUEUE_LEN);
    tokio::task::spawn_blocking(move || {
        let mut writer = BodyWriter {
            buffer: Vec::with_capacity(CHUNK_LEN),
            sender,
        };
        let result = produce(&mut writer).and_then(|()| writer.send());
        if let Err(error) = result {
            if error.kind() != io::ErrorKind::BrokenPipe {
                tracing::error!(%error, "a streamed response failed");
            }
            let _ = writer.sender.blocking_send(Err(error));
        }
    });
    Body::from_stream(stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    }))
}
