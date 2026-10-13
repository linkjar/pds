//! `com.atproto.sync.subscribeRepos`: the firehose (`docs/SPEC.md` §7).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::FromRequestParts as _;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::request::Parts;
use axum::response::Response;
use pds_sequencer::{Frame, Sequencer, Subscription, error_frame};
use pds_types::cbor::MAX_SAFE_INTEGER;

use crate::params::{Params, Spec};
use crate::state::AppState;

/// The method name.
pub const NSID: &str = "com.atproto.sync.subscribeRepos";

/// The close code after an error frame: policy violation.
const CLOSE_POLICY: u16 = 1008;
/// The close code of a stream that ended without an error.
const CLOSE_NORMAL: u16 = 1000;

/// The `cursor` of the request: `Ok(None)` when it gives none, and the
/// message of the error frame when it is not an integer.
fn cursor(query: Option<&str>) -> Result<Option<i64>, String> {
    let given = query.unwrap_or_default().split('&').any(|part| {
        let (name, value) = part.split_once('=').unwrap_or((part, ""));
        name == "cursor" && !value.is_empty()
    });
    if !given {
        return Ok(None);
    }
    const CURSOR: &[Spec] = &[Spec::integer(
        "cursor",
        -MAX_SAFE_INTEGER,
        MAX_SAFE_INTEGER,
        0,
    )];
    Params::parse(NSID, query, CURSOR)
        .map(|params| Some(params.integer("cursor")))
        .map_err(|error| error.message)
}

/// Answers a WebSocket upgrade of the firehose. `None` when the request is
/// not an upgrade.
pub async fn upgrade(state: &AppState, parts: &mut Parts) -> Option<Response> {
    let writes = state.writes()?;
    let upgrade = WebSocketUpgrade::from_request_parts(parts, &())
        .await
        .ok()?;
    let cursor = cursor(parts.uri.query());
    let sequencer = Arc::clone(writes.sequencer());
    let backfill_limit_ms = writes.settings().backfill_limit_ms;
    let max_buffer = writes.settings().max_subscription_buffer;
    Some(upgrade.on_upgrade(move |socket| async move {
        serve(socket, sequencer, cursor, backfill_limit_ms, max_buffer).await;
    }))
}

async fn close(mut socket: WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

/// Sends the error frame and closes, as the Reference's stream server does.
async fn refuse(mut socket: WebSocket, error: &'static str, message: &str) {
    if socket
        .send(Message::Binary(error_frame(error, message).into()))
        .await
        .is_ok()
    {
        close(socket, CLOSE_POLICY, error).await;
    }
}

fn binary(frame: Frame) -> Message {
    Message::Binary(Bytes::from_owner(frame))
}

async fn serve(
    mut socket: WebSocket,
    sequencer: Arc<Sequencer>,
    cursor: Result<Option<i64>, String>,
    backfill_limit_ms: i64,
    max_buffer: u32,
) {
    let cursor = match cursor {
        Ok(cursor) => cursor,
        Err(message) => return refuse(socket, "InvalidRequest", &message).await,
    };
    tracing::info!(?cursor, "request to com.atproto.sync.subscribeRepos");
    let mut subscription =
        match Subscription::open(sequencer, cursor, backfill_limit_ms, max_buffer).await {
            Ok(Ok(subscription)) => subscription,
            Ok(Err(refusal)) => return refuse(socket, refusal.error(), refusal.message()).await,
            Err(error) => {
                tracing::error!(%error, "the firehose could not start");
                return;
            }
        };
    loop {
        tokio::select! {
            // A frame the client sends is ignored. The end of its side ends
            // the stream.
            received = socket.recv() => match received {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
            frames = subscription.next() => match frames {
                Ok(Ok(frames)) => {
                    for frame in frames {
                        if socket.send(binary(frame)).await.is_err() {
                            return;
                        }
                    }
                }
                Ok(Err(refusal)) => {
                    return refuse(socket, refusal.error(), refusal.message()).await;
                }
                Err(error) => {
                    // The Reference ends the connection without a close
                    // frame when its handler fails.
                    tracing::error!(%error, cursor = subscription.cursor(), "the firehose failed");
                    return;
                }
            },
        }
    }
}

/// Closes a stream that ended without an error. The firehose never ends by
/// itself; a shutdown ends it.
pub async fn close_normally(socket: WebSocket) {
    close(socket, CLOSE_NORMAL, "").await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_is_an_optional_integer() {
        assert_eq!(cursor(None), Ok(None));
        assert_eq!(cursor(Some("cursor=")), Ok(None));
        assert_eq!(cursor(Some("cursor=0")), Ok(Some(0)));
        assert_eq!(cursor(Some("a=b&cursor=41")), Ok(Some(41)));
        assert!(cursor(Some("cursor=abc")).unwrap_err().contains("integer"));
    }
}
