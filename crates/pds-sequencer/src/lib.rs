//! The event log, firehose subscriptions and crawler notification
//! (`docs/SPEC.md` §7).
//!
//! The log is the Reference's `repo_seq` table. A commit handler appends to
//! it through [`Sequencer::sequence`]; a subscriber reads it through
//! [`Subscription`]. Frames come from the stored rows only, so a subscriber
//! sees exactly what a restart would serve.

pub mod crawlers;
pub mod events;
pub mod sequencer;

pub use crawlers::Crawlers;
pub use events::{
    CommitOp, FrameError, MAX_FRAME_LEN, account_event, commit_event, error_frame, event_rev,
    identity_event, info_frame, sync_event,
};
pub use sequencer::{Frame, Refusal, Sequencer, SequencerError, Subscription};
