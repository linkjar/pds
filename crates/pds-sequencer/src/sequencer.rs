//! The event log and its subscriptions (`docs/SPEC.md` §7).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use pds_storage::{SeqInsert, SeqRow, SequencerDb, StorageError};
use tokio::sync::watch;

use crate::crawlers::Crawlers;
use crate::events::{FrameError, frame, info_frame};

/// How many events one read of the log returns.
const PAGE: u32 = 500;

/// How many encoded frames the sequencer keeps for other subscribers.
const FRAME_CACHE: usize = 1024;

/// How long a live subscription waits for a wakeup before it reads the log
/// again. A lost wakeup costs at most this (SPEC §7.4).
const POLL: Duration = Duration::from_secs(1);

/// Milliseconds since the Unix epoch as the ISO 8601 text the Reference
/// writes: UTC with three decimals.
pub fn iso_millis(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

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
    path: PathBuf,
    writer: Mutex<SequencerDb>,
    /// The newest sequence number this process knows of.
    head: watch::Sender<i64>,
    crawlers: Crawlers,
    now_ms: Box<dyn Fn() -> i64 + Send + Sync>,
    frames: Mutex<VecDeque<(i64, Frame)>>,
}

impl std::fmt::Debug for Sequencer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sequencer")
            .field("path", &self.path)
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
        let writer = SequencerDb::open_writable(path)?;
        let head = writer.current()?.unwrap_or(0);
        Ok(Self {
            path: path.to_owned(),
            writer: Mutex::new(writer),
            head: watch::Sender::new(head),
            crawlers,
            now_ms: Box::new(now_ms),
            frames: Mutex::new(VecDeque::with_capacity(FRAME_CACHE)),
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
        SequencerDb::open(&self.path)
    }

    fn cached(&self, seq: i64) -> Option<Frame> {
        let frames = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
        frames
            .iter()
            .rev()
            .find(|(cached, _)| *cached == seq)
            .map(|(_, frame)| Arc::clone(frame))
    }

    /// The frame of a stored event. It is encoded once and shared
    /// (SPEC §7.5).
    fn frame_of(&self, row: &SeqRow) -> Result<Frame, FrameError> {
        if let Some(frame) = self.cached(row.seq) {
            return Ok(frame);
        }
        let encoded: Frame = frame(row)?.into();
        let mut frames = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
        if frames.len() == FRAME_CACHE {
            frames.pop_front();
        }
        frames.push_back((row.seq, Arc::clone(&encoded)));
        drop(frames);
        Ok(encoded)
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
            // One more than the buffer, to see that the subscriber is past it.
            let limit = PAGE.max(self.max_buffer.saturating_add(1));
            let rows = blocking(move || Ok(sequencer.reader()?.range(cursor, limit)?)).await?;
            if self.live && !self.waited && rows.len() > self.max_buffer as usize {
                return Ok(Err(Refusal::ConsumerTooSlow));
            }
            if rows.len() < limit as usize {
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
            let cursor = self.cursor;
            let _ = tokio::time::timeout(POLL, self.wake.wait_for(|head| *head > cursor)).await;
            self.waited = true;
        }
    }
}
