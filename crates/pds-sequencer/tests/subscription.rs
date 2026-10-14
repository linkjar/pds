//! The rows of the cursor table (`docs/SPEC.md` §7.3) on a log in a
//! temporary directory, and the paging of a long backfill (§7.5).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::time::Duration;

use pds_sequencer::{Crawlers, Refusal, Sequencer, Subscription, identity_event};
use pds_storage::schema;
use pds_types::cbor::Reader;
use pds_types::{Data, Limits};

const DAY_MS: i64 = 86_400_000;
static NEXT: AtomicU32 = AtomicU32::new(0);

struct Log {
    dir: PathBuf,
    sequencer: Arc<Sequencer>,
    clock: Arc<AtomicI64>,
}

impl Drop for Log {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Log {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pds-sequencer-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sequencer.sqlite");
        drop(schema::create(&path, schema::SEQUENCER).unwrap());
        let clock = Arc::new(AtomicI64::new(1_767_225_600_000));
        let now = Arc::clone(&clock);
        let crawlers = Crawlers::new("pds.test".to_owned(), Vec::new(), reqwest::Client::new());
        let sequencer =
            Sequencer::open(&path, crawlers, move || now.load(Ordering::SeqCst)).unwrap();
        Self {
            dir,
            sequencer: Arc::new(sequencer),
            clock,
        }
    }

    /// Appends `count` events, each with a handle of `size` characters.
    fn append(&self, count: usize, size: usize) {
        for _ in 0..count {
            let handle = "h".repeat(size);
            let event = identity_event("did:plc:abc", Some(&handle), &self.sequencer.now());
            self.sequencer.sequence(&[event]).unwrap();
        }
    }

    async fn open(&self, cursor: Option<i64>, max_buffer: u32) -> Result<Subscription, Refusal> {
        Subscription::open(Arc::clone(&self.sequencer), cursor, DAY_MS, max_buffer)
            .await
            .unwrap()
    }
}

fn decode(frame: &[u8]) -> (String, Data) {
    let mut reader = Reader::new(frame, Limits::FRAME).unwrap();
    let head = reader.data(0).unwrap();
    let body = reader.data(0).unwrap();
    let Some(Data::String(kind)) = head.get("t") else {
        panic!("header {head:?}");
    };
    (kind.clone(), body)
}

fn seq(frame: &[u8]) -> i64 {
    match decode(frame).1.get("seq") {
        Some(Data::Integer(seq)) => *seq,
        other => panic!("seq {other:?}"),
    }
}

/// Reads until the subscription is at `head`.
async fn drain(subscription: &mut Subscription, head: i64) -> Vec<Vec<i64>> {
    let mut pages = Vec::new();
    while subscription.cursor() < head {
        let frames = subscription.next().await.unwrap().unwrap();
        pages.push(frames.iter().map(|frame| seq(frame)).collect());
    }
    pages
}

#[tokio::test]
async fn a_cursor_replays_what_follows_it_and_no_cursor_replays_nothing() {
    let log = Log::new();
    log.append(5, 8);

    // cursor=0: the whole log. A cursor inside it: exclusive, to verify at
    // the pin (`parity/verify-at-pin.md`, the backfill cursor).
    let mut from_zero = log.open(Some(0), 500).await.unwrap();
    assert_eq!(drain(&mut from_zero, 5).await.concat(), [1, 2, 3, 4, 5]);
    let mut from_three = log.open(Some(3), 500).await.unwrap();
    assert_eq!(drain(&mut from_three, 5).await.concat(), [4, 5]);

    // No cursor: live events only. A cursor at the head: the same.
    let mut live = log.open(None, 500).await.unwrap();
    let mut at_head = log.open(Some(5), 500).await.unwrap();
    assert_eq!((live.cursor(), at_head.cursor()), (5, 5));
    let waiting = tokio::time::timeout(Duration::from_millis(200), live.next()).await;
    assert!(waiting.is_err(), "nothing arrives until something happens");
    log.append(1, 8);
    for subscription in [&mut live, &mut at_head, &mut from_zero] {
        let frames = subscription.next().await.unwrap().unwrap();
        assert_eq!(frames.iter().map(|f| seq(f)).collect::<Vec<_>>(), [6]);
    }

    // A cursor above the head.
    assert_eq!(
        log.open(Some(7), 500).await.err(),
        Some(Refusal::FutureCursor)
    );
    assert_eq!(Refusal::FutureCursor.message(), "Cursor in the future.");
}

#[tokio::test]
async fn a_cursor_older_than_the_window_gets_info_and_the_window() {
    let log = Log::new();
    log.append(2, 8);
    log.clock.fetch_add(DAY_MS + 1, Ordering::SeqCst);
    log.append(2, 8);

    // The next event after the cursor is older than the window.
    let mut outdated = log.open(Some(0), 500).await.unwrap();
    let first = outdated.next().await.unwrap().unwrap();
    assert_eq!(first.len(), 1);
    let (kind, body) = decode(&first[0]);
    assert_eq!(kind, "#info");
    assert_eq!(
        body.get("name"),
        Some(&Data::String("OutdatedCursor".to_owned()))
    );
    assert_eq!(drain(&mut outdated, 4).await.concat(), [3, 4]);

    // A cursor whose next event is inside the window is like any other.
    let mut inside = log.open(Some(2), 500).await.unwrap();
    assert_eq!(drain(&mut inside, 4).await.concat(), [3, 4]);

    // Nothing in the window: the info frame, then live events only.
    log.clock.fetch_add(DAY_MS + 1, Ordering::SeqCst);
    let mut stale = log.open(Some(0), 500).await.unwrap();
    assert_eq!(decode(&stale.next().await.unwrap().unwrap()[0]).0, "#info");
    assert_eq!(stale.cursor(), 4);
}

#[tokio::test]
async fn a_subscriber_that_falls_behind_the_buffer_is_too_slow() {
    let log = Log::new();
    log.append(20, 8);

    // The backfill is not held to the buffer.
    let mut subscription = log.open(Some(0), 5).await.unwrap();
    assert_eq!(drain(&mut subscription, 20).await.concat().len(), 20);

    // At the head, a wait is not slowness: seven events arrive at once.
    let waiting = tokio::time::timeout(Duration::from_millis(100), subscription.next()).await;
    assert!(waiting.is_err());
    log.append(7, 8);
    let frames = subscription.next().await.unwrap().unwrap();
    assert_eq!(frames.len(), 7);

    // While it holds those frames, more than the buffer arrive.
    log.append(6, 8);
    assert_eq!(
        subscription.next().await.unwrap().err(),
        Some(Refusal::ConsumerTooSlow)
    );

    // Five would have been within the buffer.
    let mut other = log.open(Some(27), 5).await.unwrap();
    assert_eq!(drain(&mut other, 33).await.concat().len(), 6);
    log.append(5, 8);
    assert_eq!(other.next().await.unwrap().unwrap().len(), 5);
}

#[tokio::test]
async fn a_long_backfill_is_read_in_pages_bounded_by_bytes() {
    let log = Log::new();
    // Twelve events of about one megabyte.
    log.append(12, 1_000_000);
    let mut subscription = log.open(Some(0), 500).await.unwrap();
    let pages = drain(&mut subscription, 12).await;
    assert!(pages.len() >= 3, "{pages:?}");
    assert!(pages.iter().all(|page| page.len() <= 5), "{pages:?}");
    assert_eq!(pages.concat(), (1..=12).collect::<Vec<i64>>());
}
