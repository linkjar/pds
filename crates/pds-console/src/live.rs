//! The live view of the overview: the stream `internal.ops.subscribeState`
//! of the ops listener, read with the operator's session.
//!
//! The browser never reaches the listener: a page keeps one subscription
//! for its regions, and Topcoat sends each new rendering over the
//! browser's WebSocket to the console.

use std::sync::Arc;

use futures_util::StreamExt as _;
use pds_sidecar::client::ClientError;
use serde_json::Value;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

use crate::{Console, client::Call, session::Session};

/// The path of the stream on the ops listener.
pub const STREAM_PATH: &str = "/xrpc/internal.ops.subscribeState";

/// One frame of the stream: the overview and the subscribers.
#[derive(Debug, Clone, Default)]
pub struct Frame {
    /// The state of the server, as `internal.ops.getOverview` gives it.
    pub overview: Value,
    /// The firehose subscribers, as `internal.ops.listSubscribers` lists
    /// them.
    pub subscribers: Vec<Value>,
}

impl Frame {
    /// A frame from a text frame of the stream.
    pub fn parse(text: &str) -> Option<Self> {
        let mut value: Value = serde_json::from_str(text).ok()?;
        let subscribers = value["subscribers"].as_array().cloned()?;
        Some(Self {
            overview: value["overview"].take(),
            subscribers,
        })
    }

    /// The ids of the subscribers, in the order of the frame.
    pub fn ids(&self) -> Vec<u64> {
        self.subscribers
            .iter()
            .filter_map(|subscriber| subscriber["id"].as_u64())
            .collect()
    }

    /// A number of the overview at a dotted path, such as `audit.entries`.
    pub fn number(&self, path: &str) -> Option<i64> {
        path.split('.')
            .fold(&self.overview, |at, key| &at[key])
            .as_i64()
    }
}

/// The first frame, from two reads: the stream sends its first frame two
/// seconds after it opens.
pub async fn first(console: &Console, session: &Session) -> Result<Frame, ClientError> {
    let overview = console
        .ops(session, Call::get("/xrpc/internal.ops.getOverview"))
        .await?;
    let listed = console
        .ops(session, Call::get("/xrpc/internal.ops.listSubscribers"))
        .await?;
    Ok(Frame {
        overview,
        subscribers: listed["subscribers"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    })
}

/// What a feed last gave.
#[derive(Debug, Clone)]
pub enum Update {
    /// The stream is opening.
    Waiting,
    /// A frame of the stream.
    Frame(Arc<Frame>),
    /// The stream ended, with why.
    Ended(String),
}

/// How a region shows its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Live {
    /// The numbers of the page load; the browser is connecting.
    Snapshot,
    /// The numbers follow the stream.
    Streaming,
    /// The stream is not there; the numbers are the last ones.
    Ended(String),
}

impl Live {
    /// The state as a data attribute.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Streaming => "streaming",
            Self::Ended(_) => "ended",
        }
    }
}

/// One subscription to the stream for a page, shared by the regions of
/// the page. The task reading the stream ends when the last clone goes.
#[derive(Debug, Clone)]
pub struct Feed {
    receiver: watch::Receiver<Update>,
    _guard: Arc<Guard>,
}

/// Aborts the task of the feed when the feed is dropped.
#[derive(Debug)]
struct Guard(tokio::task::JoinHandle<()>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Feed {
    /// Opens the stream with the session of the operator.
    pub fn start(console: Console, session: Arc<Session>) -> Self {
        let (sender, receiver) = watch::channel(Update::Waiting);
        let task = tokio::spawn(read(console, session, sender));
        Self {
            receiver,
            _guard: Arc::new(Guard(task)),
        }
    }

    /// A receiver of the updates. It sees every update after this call.
    pub fn watch(&self) -> watch::Receiver<Update> {
        self.receiver.clone()
    }
}

/// Reads the stream until it ends or no region listens any more.
async fn read(console: Console, session: Arc<Session>, sender: watch::Sender<Update>) {
    let mut stream = match console.stream(&session).await {
        Ok(stream) => stream,
        Err(error) => {
            tracing::warn!(%error, "the live stream of the ops listener did not open");
            let _ = sender.send(Update::Ended(error.to_string()));
            return;
        }
    };
    while let Some(message) = stream.next().await {
        match message {
            Ok(Message::Text(text)) => {
                if let Some(frame) = Frame::parse(&text)
                    && sender.send(Update::Frame(Arc::new(frame))).is_err()
                {
                    return;
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    let _ = sender.send(Update::Ended("The stream of the server ended.".to_owned()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_the_overview_and_the_subscribers() {
        let frame = Frame::parse(
            r#"{"overview": {"accounts": 3, "audit": {"entries": 9}}, "subscribers": [{"id": 7}, {"id": 2}]}"#,
        )
        .unwrap();
        assert_eq!(frame.number("accounts"), Some(3));
        assert_eq!(frame.number("audit.entries"), Some(9));
        assert_eq!(frame.number("audit.missing"), None);
        assert_eq!(frame.ids(), [7, 2]);
        assert!(Frame::parse(r#"{"overview": {}}"#).is_none());
        assert!(Frame::parse("not json").is_none());
    }
}
