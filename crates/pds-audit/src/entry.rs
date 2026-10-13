//! The entry of the audit log (`docs/SPEC.md` §20.2).
//!
//! An entry is a DAG-CBOR array of thirteen fields in a fixed order. A
//! field that does not apply is `null`. The hash of an entry is the
//! SHA-256 of its bytes, and the last field of an entry is the hash of
//! the entry before it, so no entry changes without every later hash
//! changing.

use pds_types::Data;
use pds_types::data::Bytes;
use sha2::{Digest, Sha256};

/// The hash before the first entry.
pub const GENESIS: [u8; 32] = [0; 32];
/// How many fields an entry has.
const FIELDS: usize = 13;

/// What happened, before it has a place in the chain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    /// Who acted: a DID, or the identity of an operator such as `admin`.
    pub actor: String,
    /// What was done: the method or the name of the action.
    pub action: String,
    /// What it was done to: an account, a record, a blob, an invite code.
    pub subject: Option<String>,
    /// The kind of credential: `session`, `oauth`, `admin`, `service`,
    /// `page` or `server`.
    pub credential: Option<String>,
    /// The OAuth client.
    pub client_id: Option<String>,
    /// The device of a page.
    pub device_id: Option<String>,
    /// The request.
    pub request_id: Option<String>,
    /// `ok`, or the name of the refusal.
    pub outcome: String,
    /// The reason an operator gave.
    pub reason: Option<String>,
    /// The sidecar that acted for the operator.
    pub sidecar: Option<String>,
}

/// An entry of the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its number: the first is 1, and each is one more than the one
    /// before.
    pub seq: u64,
    /// When it was written (ISO 8601).
    pub time: String,
    /// What happened.
    pub what: Draft,
    /// The hash of the entry before it, or [`GENESIS`].
    pub prev: [u8; 32],
}

/// Why bytes are not an entry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not an audit entry: {0}")]
pub struct Malformed(pub &'static str);

fn optional(value: Option<&String>) -> Data {
    value.map_or(Data::Null, |text| Data::String(text.clone()))
}

impl Entry {
    /// The entry that follows `prev`.
    pub fn after(prev: Option<&Self>, time: &str, what: Draft) -> Self {
        Self {
            seq: prev.map_or(1, |prev| prev.seq + 1),
            time: time.to_owned(),
            what,
            prev: prev.map_or(GENESIS, Self::hash),
        }
    }

    /// The bytes of the entry: the DAG-CBOR array of its fields.
    pub fn to_bytes(&self) -> Vec<u8> {
        let what = &self.what;
        Data::Array(vec![
            Data::Integer(i64::try_from(self.seq).unwrap_or(i64::MAX)),
            Data::String(self.time.clone()),
            Data::String(what.actor.clone()),
            Data::String(what.action.clone()),
            optional(what.subject.as_ref()),
            optional(what.credential.as_ref()),
            optional(what.client_id.as_ref()),
            optional(what.device_id.as_ref()),
            optional(what.request_id.as_ref()),
            Data::String(what.outcome.clone()),
            optional(what.reason.as_ref()),
            optional(what.sidecar.as_ref()),
            Data::Bytes(Bytes(self.prev.to_vec())),
        ])
        .to_dag_cbor()
    }

    /// The SHA-256 of the bytes of the entry.
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.to_bytes()).into()
    }

    /// Reads an entry from its bytes. The bytes must be the canonical
    /// form: an entry that encodes to other bytes is refused.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Malformed> {
        let Ok(Data::Array(fields)) = Data::from_dag_cbor(bytes) else {
            return Err(Malformed("it is not a DAG-CBOR array"));
        };
        if fields.len() != FIELDS {
            return Err(Malformed("it does not have thirteen fields"));
        }
        let text = |at: usize| match &fields[at] {
            Data::String(text) => Ok(text.clone()),
            _ => Err(Malformed("a required text is missing")),
        };
        let optional = |at: usize| match &fields[at] {
            Data::Null => Ok(None),
            Data::String(text) => Ok(Some(text.clone())),
            _ => Err(Malformed("a field is neither a text nor null")),
        };
        let seq = match &fields[0] {
            Data::Integer(seq) => u64::try_from(*seq).ok().filter(|seq| *seq > 0),
            _ => None,
        }
        .ok_or(Malformed("the sequence number is not a positive integer"))?;
        let prev = match &fields[12] {
            Data::Bytes(Bytes(hash)) => <[u8; 32]>::try_from(hash.as_slice()).ok(),
            _ => None,
        }
        .ok_or(Malformed("the previous hash is not 32 bytes"))?;
        let entry = Self {
            seq,
            time: text(1)?,
            what: Draft {
                actor: text(2)?,
                action: text(3)?,
                subject: optional(4)?,
                credential: optional(5)?,
                client_id: optional(6)?,
                device_id: optional(7)?,
                request_id: optional(8)?,
                outcome: text(9)?,
                reason: optional(10)?,
                sidecar: optional(11)?,
            },
            prev,
        };
        if entry.to_bytes() != bytes {
            return Err(Malformed("the bytes are not the canonical form"));
        }
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(action: &str) -> Draft {
        Draft {
            actor: "did:plc:alice".to_owned(),
            action: action.to_owned(),
            subject: Some("did:plc:alice".to_owned()),
            credential: Some("session".to_owned()),
            outcome: "ok".to_owned(),
            ..Draft::default()
        }
    }

    #[test]
    fn an_entry_round_trips_and_names_the_entry_before_it() {
        let first = Entry::after(None, "2026-10-13T00:00:00.000Z", draft("a"));
        assert_eq!((first.seq, first.prev), (1, GENESIS));
        let second = Entry::after(Some(&first), "2026-10-13T00:00:01.000Z", draft("b"));
        assert_eq!((second.seq, second.prev), (2, first.hash()));
        let bytes = second.to_bytes();
        // A DAG-CBOR array of thirteen items.
        assert_eq!(bytes[0], 0x8d);
        assert_eq!(Entry::from_bytes(&bytes), Ok(second.clone()));

        // A change of any field changes the hash.
        let mut changed = second.clone();
        changed.what.outcome = "refused".to_owned();
        assert_ne!(changed.hash(), second.hash());
        let mut moved = second.clone();
        moved.what.subject = None;
        assert_ne!(moved.hash(), second.hash());
    }

    #[test]
    fn bytes_that_are_not_an_entry_are_refused() {
        let entry = Entry::after(None, "t", draft("a"));
        let bytes = entry.to_bytes();
        assert!(Entry::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        assert!(Entry::from_bytes(&[0x80]).is_err());
        assert!(Entry::from_bytes(b"not cbor").is_err());
        let wrong = Data::Array(vec![Data::Null; FIELDS]).to_dag_cbor();
        assert!(Entry::from_bytes(&wrong).is_err());
    }
}
