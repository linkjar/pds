//! The events of the log and their frames (`docs/SPEC.md` §7.1, §7.2).
//!
//! A stored event is the DAG-CBOR body without `seq` and `time`, as the
//! Reference stores it (`sequencer/events.ts` at the pin). A frame is the
//! header and that body with `seq` and `time` added.

use cid::Cid;
use pds_repo::{BlockMap, CommitData, write_car};
use pds_storage::{SeqInsert, SeqRow};
use pds_types::cbor::Writer;
use pds_types::{CborError, Data, Limits};

/// The largest frame the server sends (SPEC §7.2).
pub const MAX_FRAME_LEN: usize = 5_000_000;

/// One operation of a commit, as the firehose reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOp {
    /// `create`, `update` or `delete`.
    pub action: &'static str,
    /// `<collection>/<rkey>`.
    pub path: String,
    /// The record after the operation. `None` for a delete.
    pub cid: Option<Cid>,
    /// The record before the operation, when there was one.
    pub prev: Option<Cid>,
}

/// The `#commit` event of a commit. `ops` are in the order of the writes.
pub fn commit_event(did: &str, commit: &CommitData, ops: &[CommitOp], now: &str) -> SeqInsert {
    let mut blocks = BlockMap::new();
    blocks.extend(&commit.new_blocks);
    blocks.extend(&commit.relevant_blocks);
    let car = write_car(Some(&commit.cid), &blocks);

    // Keys in canonical order: by length, then by bytes.
    let mut w = Writer::with_capacity(car.len() + 512);
    w.map(9 + usize::from(commit.prev_data.is_some()));
    w.text("ops");
    w.array(ops.len());
    for op in ops {
        w.map(3 + usize::from(op.prev.is_some()));
        w.text("cid");
        match &op.cid {
            Some(cid) => w.link(cid),
            None => w.null(),
        }
        w.text("path");
        w.text(&op.path);
        if let Some(prev) = &op.prev {
            w.text("prev");
            w.link(prev);
        }
        w.text("action");
        w.text(op.action);
    }
    w.text("rev");
    w.text(commit.rev.as_str());
    w.text("repo");
    w.text(did);
    w.text("blobs");
    w.array(0);
    w.text("since");
    match &commit.since {
        Some(since) => w.text(since.as_str()),
        None => w.null(),
    }
    w.text("blocks");
    w.bytes(&car);
    w.text("commit");
    w.link(&commit.cid);
    w.text("rebase");
    w.bool(false);
    w.text("tooBig");
    w.bool(false);
    if let Some(prev_data) = &commit.prev_data {
        w.text("prevData");
        w.link(prev_data);
    }
    SeqInsert {
        did: did.to_owned(),
        event_type: "append",
        event: w.into_vec(),
        sequenced_at: now.to_owned(),
    }
}

/// The `#sync` event that states the head of a repository: the commit
/// block alone.
pub fn sync_event(did: &str, rev: &str, commit: &Cid, commit_block: &[u8], now: &str) -> SeqInsert {
    let mut blocks = BlockMap::new();
    blocks.insert(*commit, commit_block.to_vec());
    let car = write_car(Some(commit), &blocks);
    let mut w = Writer::with_capacity(car.len() + 128);
    w.map(3);
    w.text("did");
    w.text(did);
    w.text("rev");
    w.text(rev);
    w.text("blocks");
    w.bytes(&car);
    SeqInsert {
        did: did.to_owned(),
        event_type: "sync",
        event: w.into_vec(),
        sequenced_at: now.to_owned(),
    }
}

/// The `#identity` event of an account, with its handle when it is known.
pub fn identity_event(did: &str, handle: Option<&str>, now: &str) -> SeqInsert {
    let mut w = Writer::with_capacity(128);
    w.map(1 + usize::from(handle.is_some()));
    w.text("did");
    w.text(did);
    if let Some(handle) = handle {
        w.text("handle");
        w.text(handle);
    }
    SeqInsert {
        did: did.to_owned(),
        event_type: "identity",
        event: w.into_vec(),
        sequenced_at: now.to_owned(),
    }
}

/// The `#account` event of an account. `status` is `None` for an active
/// account, and otherwise `takendown`, `suspended`, `deleted` or
/// `deactivated`.
pub fn account_event(did: &str, status: Option<&str>, now: &str) -> SeqInsert {
    let mut w = Writer::with_capacity(128);
    w.map(2 + usize::from(status.is_some()));
    w.text("did");
    w.text(did);
    w.text("active");
    w.bool(status.is_none());
    if let Some(status) = status {
        w.text("status");
        w.text(status);
    }
    SeqInsert {
        did: did.to_owned(),
        event_type: "account",
        event: w.into_vec(),
        sequenced_at: now.to_owned(),
    }
}

/// A stored event cannot be sent.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The stored body is not DAG-CBOR.
    #[error("event {seq}: {source}")]
    Body {
        /// The sequence number.
        seq: i64,
        /// What is wrong with the body.
        source: CborError,
    },
    /// The stored body is not a map.
    #[error("event {0}: the body is not a map")]
    NotAMap(i64),
    /// The row has an event type this server does not know.
    #[error("event {seq}: unknown event type {event_type}")]
    UnknownType {
        /// The sequence number.
        seq: i64,
        /// The stored type.
        event_type: String,
    },
    /// The frame would be over the limit.
    #[error("event {0}: the frame is over {MAX_FRAME_LEN} bytes")]
    TooLarge(i64),
}

fn header(w: &mut Writer, kind: &str) {
    w.map(2);
    w.text("t");
    w.text(kind);
    w.text("op");
    w.integer(1);
}

/// The frame of a stored event: the header, then the body with `seq` and
/// `time`.
pub fn frame(row: &SeqRow) -> Result<Vec<u8>, FrameError> {
    let kind = match row.event_type.as_str() {
        "append" => "#commit",
        "sync" => "#sync",
        "identity" => "#identity",
        "account" => "#account",
        other => {
            return Err(FrameError::UnknownType {
                seq: row.seq,
                event_type: other.to_owned(),
            });
        }
    };
    let body =
        Data::from_dag_cbor_with(&row.event, Limits::FRAME).map_err(|source| FrameError::Body {
            seq: row.seq,
            source,
        })?;
    let Data::Object(mut body) = body else {
        return Err(FrameError::NotAMap(row.seq));
    };
    body.insert("seq".to_owned(), Data::Integer(row.seq));
    body.insert("time".to_owned(), Data::String(row.sequenced_at.clone()));
    let mut w = Writer::with_capacity(row.event.len() + 96);
    header(&mut w, kind);
    w.data(&Data::Object(body));
    let bytes = w.into_vec();
    if bytes.len() > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(row.seq));
    }
    Ok(bytes)
}

/// The `#info` frame.
pub fn info_frame(name: &str, message: &str) -> Vec<u8> {
    let mut w = Writer::with_capacity(128);
    header(&mut w, "#info");
    w.map(2);
    w.text("name");
    w.text(name);
    w.text("message");
    w.text(message);
    w.into_vec()
}

/// The error frame. The connection closes after it.
pub fn error_frame(error: &str, message: &str) -> Vec<u8> {
    let mut w = Writer::with_capacity(128);
    w.map(1);
    w.text("op");
    w.integer(-1);
    w.map(2);
    w.text("error");
    w.text(error);
    w.text("message");
    w.text(message);
    w.into_vec()
}

/// The revision an `append` or `sync` body states, or `None` when the body
/// has none.
pub fn event_rev(body: &[u8]) -> Option<String> {
    match Data::from_dag_cbor_with(body, Limits::FRAME)
        .ok()?
        .get("rev")?
    {
        Data::String(rev) => Some(rev.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_the_header_and_the_body_with_seq_and_time() {
        let stored = account_event(
            "did:plc:abc",
            Some("deactivated"),
            "2026-01-01T00:00:00.000Z",
        );
        let row = SeqRow {
            seq: 7,
            did: stored.did,
            event_type: stored.event_type.to_owned(),
            event: stored.event,
            sequenced_at: stored.sequenced_at,
        };
        let bytes = frame(&row).unwrap();
        let mut reader = pds_types::cbor::Reader::new(&bytes, Limits::FRAME).unwrap();
        let head = reader.data(0).unwrap();
        assert_eq!(head.get("t"), Some(&Data::String("#account".to_owned())));
        assert_eq!(head.get("op"), Some(&Data::Integer(1)));
        let body = reader.data(0).unwrap();
        assert_eq!(body.get("seq"), Some(&Data::Integer(7)));
        assert_eq!(body.get("active"), Some(&Data::Bool(false)));
        assert_eq!(
            body.get("status"),
            Some(&Data::String("deactivated".to_owned()))
        );
        assert!(reader.remaining().is_empty());
    }

    #[test]
    fn every_event_body_is_canonical() {
        let now = "2026-01-01T00:00:00.000Z";
        for event in [
            identity_event("did:plc:abc", Some("alice.test"), now),
            identity_event("did:plc:abc", None, now),
            account_event("did:plc:abc", None, now),
        ] {
            Data::from_dag_cbor(&event.event).unwrap();
        }
    }
}
