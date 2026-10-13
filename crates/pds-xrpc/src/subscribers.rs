//! The live subscribers of the firehose (`docs/SPEC.md` §7.3, §22.1): what
//! the ops listener shows of each, and the means to disconnect one.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tokio::sync::Notify;

/// One connection to `subscribeRepos`.
#[derive(Debug)]
pub struct Subscriber {
    id: u64,
    remote: Option<String>,
    user_agent: Option<String>,
    connected_ms: i64,
    cursor: AtomicI64,
    frames: AtomicU64,
    bytes: AtomicU64,
    last_frame_ms: AtomicI64,
    stop: Notify,
}

impl Subscriber {
    /// Notes a frame that was sent.
    pub fn sent(&self, len: usize, cursor: i64, now_ms: i64) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        self.bytes
            .fetch_add(u64::try_from(len).unwrap_or(0), Ordering::Relaxed);
        self.cursor.store(cursor, Ordering::Relaxed);
        self.last_frame_ms.store(now_ms, Ordering::Relaxed);
    }

    /// Resolves when an operator disconnects the subscriber.
    pub async fn stopped(&self) {
        self.stop.notified().await;
    }

    /// What the ops listener shows. `head` is the last sequence number of
    /// the log.
    fn to_json(&self, head: i64) -> Value {
        let cursor = self.cursor.load(Ordering::Relaxed);
        let last = self.last_frame_ms.load(Ordering::Relaxed);
        json!({
            "id": self.id,
            "remote": self.remote,
            "userAgent": self.user_agent,
            "connectedAt": pds_sequencer::iso_millis(self.connected_ms),
            "cursor": cursor,
            "lag": (head - cursor).max(0),
            "frames": self.frames.load(Ordering::Relaxed),
            "bytes": self.bytes.load(Ordering::Relaxed),
            "lastFrameAt": (last > 0).then(|| pds_sequencer::iso_millis(last)),
        })
    }
}

/// The subscribers that are connected now.
#[derive(Debug, Default)]
pub struct Subscribers {
    next: AtomicU64,
    live: Mutex<BTreeMap<u64, Arc<Subscriber>>>,
}

/// A place in the registry. The subscriber leaves when this is dropped.
#[derive(Debug)]
pub struct Registered<'a> {
    registry: &'a Subscribers,
    subscriber: Arc<Subscriber>,
}

impl std::ops::Deref for Registered<'_> {
    type Target = Subscriber;

    fn deref(&self) -> &Subscriber {
        &self.subscriber
    }
}

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        self.registry
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.subscriber.id);
    }
}

impl Subscribers {
    /// Adds a subscriber that starts after `cursor`.
    pub fn register(
        &self,
        remote: Option<String>,
        user_agent: Option<String>,
        cursor: i64,
        now_ms: i64,
    ) -> Registered<'_> {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let subscriber = Arc::new(Subscriber {
            id,
            remote,
            user_agent,
            connected_ms: now_ms,
            cursor: AtomicI64::new(cursor),
            frames: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            last_frame_ms: AtomicI64::new(0),
            stop: Notify::new(),
        });
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, Arc::clone(&subscriber));
        Registered {
            registry: self,
            subscriber,
        }
    }

    /// How many are connected.
    pub fn len(&self) -> usize {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether none is connected.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Each subscriber, in order of connection.
    pub fn list(&self, head: i64) -> Vec<Value> {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(|subscriber| subscriber.to_json(head))
            .collect()
    }

    /// Tells the subscriber `id` to stop. `false` when it is not connected.
    pub fn disconnect(&self, id: u64) -> bool {
        let found = self
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            .cloned();
        found.is_some_and(|subscriber| {
            subscriber.stop.notify_one();
            true
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_subscriber_is_listed_until_it_leaves_and_can_be_stopped() {
        let registry = Subscribers::default();
        let first = registry.register(Some("10.0.0.1".to_owned()), None, 5, 1_000);
        let second = registry.register(None, Some("relay".to_owned()), 0, 2_000);
        first.sent(100, 7, 3_000);
        let listed = registry.list(10);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["cursor"], 7);
        assert_eq!(listed[0]["lag"], 3);
        assert_eq!(listed[0]["bytes"], 100);
        assert_eq!(listed[1]["userAgent"], "relay");
        assert!(listed[1]["lastFrameAt"].is_null());

        assert!(registry.disconnect(listed[1]["id"].as_u64().unwrap()));
        second.stopped().await;
        drop(second);
        assert_eq!(registry.len(), 1);
        assert!(!registry.disconnect(2));
        drop(first);
        assert!(registry.is_empty());
    }
}
