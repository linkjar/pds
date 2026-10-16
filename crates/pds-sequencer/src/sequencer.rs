//! The event log and its subscriptions (`docs/SPEC.md` §7).

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use pds_storage::{DataLayout, SeqInsert, SeqRow, SequencerDb, StorageError};
use pds_types::iso_millis;
use tokio::sync::watch;

use crate::crawlers::Crawlers;
use crate::events::{FrameError, frame, info_frame};

/// The most events one read of the log returns.
const PAGE: u32 = 500;

/// One read of the log stops after this many bytes of events, so that a
/// subscriber far behind a log of large commits holds a few megabytes and
/// not the whole range (SPEC §7.5).
const PAGE_BYTES: usize = 4 * 1024 * 1024;

/// The most bytes of encoded frames the sequencer keeps for other
/// subscribers.
const FRAME_CACHE_BYTES: usize = 16 * 1024 * 1024;

/// How long a live subscription waits for a wakeup before it reads the log
/// again. A lost wakeup costs at most this (SPEC §7.4).
const POLL: Duration = Duration::from_secs(1);

/// A frame, shared by every subscriber that receives it.
pub type Frame = Arc<[u8]>;

/// A sequencer call failed.
#[derive(Debug, thiserror::Error)]
pub enum SequencerError {
    /// The log could not be read or written.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// A stored event cannot be sent.
    #[error(transparent)]
    Frame(#[from] FrameError),
    /// A blocking task ended without a result.
    #[error("the sequencer task failed: {0}")]
    Task(String),
}

/// Why a subscription ends with an error frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The cursor is above the newest event.
    FutureCursor,
    /// The subscriber fell too far behind the log.
    ConsumerTooSlow,
}

impl Refusal {
    /// The error name of the frame.
    pub const fn error(self) -> &'static str {
        match self {
            Self::FutureCursor => "FutureCursor",
            Self::ConsumerTooSlow => "ConsumerTooSlow",
        }
    }

    /// The message of the frame, as the Reference words it.
    pub const fn message(self) -> &'static str {
        match self {
            Self::FutureCursor => "Cursor in the future.",
            Self::ConsumerTooSlow => "Stream consumer too slow",
        }
    }
}

/// The event log: one writer, and a wakeup for subscribers.
pub struct Sequencer {
    layout: DataLayout,
    writer: Mutex<SequencerDb>,
    /// The newest sequence number this process knows of.
    head: watch::Sender<i64>,
    crawlers: Crawlers,
    now_ms: Box<dyn Fn() -> i64 + Send + Sync>,
    frames: Mutex<FrameCache>,
}

/// The frames of the newest events, encoded once for every subscriber that
/// reads them (SPEC §7.5).
#[derive(Default)]
struct FrameCache {
    /// The frames in the order they came, for eviction.
    frames: VecDeque<(i64, Frame)>,
    /// The same frames by sequence number.
    by_seq: HashMap<i64, Frame>,
    bytes: usize,
}

impl FrameCache {
    /// Keeps `frame` for `seq` and returns the frame every subscriber
    /// shares: the one another subscriber kept meanwhile, or this one.
    fn keep(&mut self, seq: i64, frame: Frame) -> Frame {
        if let Some(shared) = self.by_seq.get(&seq) {
            return Arc::clone(shared);
        }
        self.bytes += frame.len();
        self.frames.push_back((seq, Arc::clone(&frame)));
        self.by_seq.insert(seq, Arc::clone(&frame));
        while self.bytes > FRAME_CACHE_BYTES {
            match self.frames.pop_front() {
                Some((old_seq, old)) => {
                    self.bytes -= old.len();
                    self.by_seq.remove(&old_seq);
                }
                None => break,
            }
        }
        frame
    }
}

impl std::fmt::Debug for Sequencer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sequencer")
            .field("path", &self.layout.sequencer_db)
            .field("head", &*self.head.borrow())
            .finish_non_exhaustive()
    }
}

impl Sequencer {
    /// Opens the log at `path`. `now_ms` is the clock (SPEC §3.2).
    pub fn open(
        path: &Path,
        crawlers: Crawlers,
        now_ms: impl Fn() -> i64 + Send + Sync + 'static,
    ) -> Result<Self, StorageError> {
        let mut layout = DataLayout::new(path.parent().unwrap_or_else(|| Path::new(".")));
        path.clone_into(&mut layout.sequencer_db);
        Self::open_in(&layout, crawlers, now_ms)
    }

    /// Opens the log of a data directory, in the file or in the shared
    /// tier that the layout names.
    pub fn open_in(
        layout: &DataLayout,
        crawlers: Crawlers,
        now_ms: impl Fn() -> i64 + Send + Sync + 'static,
    ) -> Result<Self, StorageError> {
        let writer = layout.sequencer_writable()?;
        let head = writer.current()?.unwrap_or(0);
        Ok(Self {
            layout: layout.clone(),
            writer: Mutex::new(writer),
            head: watch::Sender::new(head),
            crawlers,
            now_ms: Box::new(now_ms),
            frames: Mutex::new(FrameCache::default()),
        })
    }

    /// The current time as the log writes it.
    pub fn now(&self) -> String {
        iso_millis((self.now_ms)())
    }

    /// The newest sequence number, or zero for an empty log.
    pub fn head(&self) -> i64 {
        *self.head.borrow()
    }

    /// Appends events in one transaction, wakes the subscribers and notes
    /// the update for the crawlers. Returns the sequence numbers.
    ///
    /// This call blocks: run it on the blocking pool.
    pub fn sequence(&self, events: &[SeqInsert]) -> Result<Vec<i64>, StorageError> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let seqs = writer.append(events)?;
        if let Some(last) = seqs.last() {
            // Inside the lock, so the head never goes backwards.
            self.head.send_replace(*last);
        }
        drop(writer);
        self.crawlers.notify_of_update((self.now_ms)());
        Ok(seqs)
    }

    /// Appends the `#account` event of a deleted account and removes every
    /// other event of the account, as the Reference does.
    pub fn sequence_deletion(&self, deleted: &SeqInsert) -> Result<i64, StorageError> {
        let seqs = self.sequence(std::slice::from_ref(deleted))?;
        let seq = seqs.first().copied().unwrap_or(0);
        self.writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .delete_for_did_except(&deleted.did, seq)?;
        Ok(seq)
    }

    /// A read connection to the log.
    pub fn reader(&self) -> Result<SequencerDb, StorageError> {
        self.layout.sequencer()
    }

    fn cached(&self, seq: i64) -> Option<Frame> {
        let cache = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
        cache.by_seq.get(&seq).map(Arc::clone)
    }

    /// The frame of a stored event. It is encoded once and shared
    /// (SPEC §7.5).
    fn frame_of(&self, row: &SeqRow) -> Result<Frame, FrameError> {
        if let Some(frame) = self.cached(row.seq) {
            return Ok(frame);
        }
        let encoded: Frame = frame(row)?.into();
        // A frame that would fill the cache by itself is not kept.
        if encoded.len() > FRAME_CACHE_BYTES / 4 {
            return Ok(encoded);
        }
        Ok(self
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keep(row.seq, encoded))
    }
}

/// Where a subscription starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// The sequence number after which events are sent.
    cursor: i64,
    /// Whether the stream begins with `#info` `OutdatedCursor`.
    outdated: bool,
}

/// One subscriber of the log.
///
/// Frames come from the log in `seq` order and nowhere else; the wakeup
/// carries no frame (SPEC §7.4).
#[derive(Debug)]
pub struct Subscription {
    sequencer: Arc<Sequencer>,
    wake: watch::Receiver<i64>,
    cursor: i64,
    max_buffer: u32,
    /// Whether the subscription has reached the head once.
    live: bool,
    /// Whether the last read came after a wait at the head. A subscriber
    /// that waited is idle, not slow.
    waited: bool,
    info: Option<Frame>,
    /// The connection of the last read, kept for the next.
    reader: Option<SequencerDb>,
}

async fn blocking<T, F>(call: F) -> Result<T, SequencerError>
where
    F: FnOnce() -> Result<T, SequencerError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(call)
        .await
        .map_err(|e| SequencerError::Task(e.to_string()))?
}

/// One page after `cursor`, or `None` for a subscriber that is too slow:
/// `buffer` is the most events it may be behind.
fn read_page(
    log: &SequencerDb,
    cursor: i64,
    buffer: Option<u32>,
) -> Result<Option<(Vec<SeqRow>, bool)>, StorageError> {
    if let Some(buffer) = buffer
        && log.pending(cursor, buffer.saturating_add(1))? > buffer
    {
        return Ok(None);
    }
    Ok(Some(log.page(cursor, PAGE, PAGE_BYTES)?))
}

impl Subscription {
    /// Opens a subscription (SPEC §7.3). `cursor` is the client's;
    /// `backfill_limit_ms` is `PDS_REPO_BACKFILL_LIMIT_MS` and `max_buffer`
    /// is `PDS_MAX_SUBSCRIPTION_BUFFER`.
    ///
    /// The outer error is a failure of the server; the inner one is the
    /// error frame to send before the close.
    pub async fn open(
        sequencer: Arc<Sequencer>,
        cursor: Option<i64>,
        backfill_limit_ms: i64,
        max_buffer: u32,
    ) -> Result<Result<Self, Refusal>, SequencerError> {
        // Subscribe before the head is read, so that no event falls between
        // the two.
        let wake = sequencer.head.subscribe();
        let seq = Arc::clone(&sequencer);
        let backfill_time = iso_millis((seq.now_ms)().saturating_sub(backfill_limit_ms));
        let start = blocking(move || {
            let db = seq.reader()?;
            let current = db.current()?.unwrap_or(0);
            let Some(cursor) = cursor else {
                return Ok(Ok(Start {
                    cursor: current,
                    outdated: false,
                }));
            };
            if cursor > current {
                return Ok(Err(Refusal::FutureCursor));
            }
            let next = db.next_time(cursor)?;
            if next.is_some_and(|time| time < backfill_time) {
                // The window starts at the earliest event inside it. With
                // none inside, only live events follow.
                let start = db.earliest_after_time(&backfill_time)?;
                return Ok(Ok(Start {
                    cursor: start.map_or(current, |seq| seq - 1),
                    outdated: true,
                }));
            }
            Ok(Ok(Start {
                cursor,
                outdated: false,
            }))
        })
        .await?;
        Ok(start.map(|start| Self {
            sequencer,
            wake,
            cursor: start.cursor,
            max_buffer,
            live: cursor.is_none(),
            waited: true,
            info: start.outdated.then(|| {
                info_frame(
                    "OutdatedCursor",
                    "Requested cursor exceeded limit. Possibly missing events",
                )
                .into()
            }),
            reader: None,
        }))
    }

    /// The sequence number of the last event this subscription returned.
    pub const fn cursor(&self) -> i64 {
        self.cursor
    }

    /// The next frames, in order: at least one. It waits when the
    /// subscriber is at the head of the log.
    ///
    /// The inner error ends the stream with an error frame. A subscriber
    /// that was at the head is too slow when more events than the buffer
    /// holds were appended while it took its last frames: the Reference
    /// keeps those events in a buffer of that size and ends the stream
    /// when it overflows.
    pub async fn next(&mut self) -> Result<Result<Vec<Frame>, Refusal>, SequencerError> {
        if let Some(info) = self.info.take() {
            return Ok(Ok(vec![info]));
        }
        loop {
            let sequencer = Arc::clone(&self.sequencer);
            let cursor = self.cursor;
            // A subscriber that was at the head and took its last frames
            // without a wait is checked against the buffer.
            let buffer = (self.live && !self.waited).then_some(self.max_buffer);
            // The connection of the last read serves this one. A read that
            // failed drops it, and the next read opens another.
            let kept = self.reader.take();
            let (log, page) = blocking(move || {
                let log = match kept {
                    Some(log) => log,
                    None => sequencer.reader()?,
                };
                let page = read_page(&log, cursor, buffer)?;
                Ok((log, page))
            })
            .await?;
            self.reader = Some(log);
            let Some((rows, at_head)) = page else {
                return Ok(Err(Refusal::ConsumerTooSlow));
            };
            if at_head {
                self.live = true;
            }
            if let Some(last) = rows.last() {
                let mut frames = Vec::with_capacity(rows.len());
                for row in &rows {
                    frames.push(self.sequencer.frame_of(row)?);
                }
                self.cursor = last.seq;
                self.waited = false;
                return Ok(Ok(frames));
            }
            // At the head: wait for an append, or read again after a while.
            // The mark is set before the wait, so that it holds when the
            // caller drops this call while it waits.
            self.waited = true;
            let cursor = self.cursor;
            let _ = tokio::time::timeout(POLL, self.wake.wait_for(|head| *head > cursor)).await;
        }
    }
}
