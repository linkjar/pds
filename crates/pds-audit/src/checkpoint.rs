//! Signed checkpoints (`docs/SPEC.md` §20.3).
//!
//! A checkpoint covers the entries from `first` to `last`. It holds the
//! Merkle root of their bytes, the hash of the last entry, the time and
//! the key, and an Ed25519 signature over all of those.

use pds_types::Data;
use pds_types::data::Bytes;
use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};

use crate::merkle;

/// What the signed bytes begin with, so that a signature of a checkpoint
/// is a signature of nothing else.
pub const DOMAIN: &str = "linkjar-pds-audit-checkpoint-v1";

/// A checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// The first entry it covers.
    pub first: u64,
    /// The last entry it covers.
    pub last: u64,
    /// The Merkle root of the entries from `first` to `last`.
    pub root: [u8; 32],
    /// The hash of the last entry: the head of the chain at this time.
    pub head: [u8; 32],
    /// When it was made (ISO 8601).
    pub time: String,
    /// The key that signed it.
    pub key_id: String,
    /// The Ed25519 signature over [`Checkpoint::signed_bytes`].
    pub signature: Vec<u8>,
}

impl Checkpoint {
    /// The bytes that the signature covers: a DAG-CBOR array of the
    /// domain and every field but the signature.
    pub fn signed_bytes(&self) -> Vec<u8> {
        Data::Array(vec![
            Data::String(DOMAIN.to_owned()),
            Data::Integer(i64::try_from(self.first).unwrap_or(i64::MAX)),
            Data::Integer(i64::try_from(self.last).unwrap_or(i64::MAX)),
            Data::Bytes(Bytes(self.root.to_vec())),
            Data::Bytes(Bytes(self.head.to_vec())),
            Data::String(self.time.clone()),
            Data::String(self.key_id.clone()),
        ])
        .to_dag_cbor()
    }
}

/// The id of a public key: `ed25519:` and the first 16 hexadecimal digits
/// of its SHA-256.
pub fn key_id(public_key: &[u8]) -> String {
    use std::fmt::Write;
    let digest = Sha256::digest(public_key);
    let mut id = String::from("ed25519:");
    for byte in &digest[..8] {
        let _ = write!(id, "{byte:02x}");
    }
    id
}

/// The checkpoint key (SPEC §20.3; Appendix D, the custody of the audit
/// checkpoint key).
pub struct Signer {
    pair: Ed25519KeyPair,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("key_id", &self.key_id())
            .finish_non_exhaustive()
    }
}

impl Signer {
    /// The signer of a 32-byte Ed25519 seed.
    pub fn from_seed(seed: &[u8]) -> Option<Self> {
        Ed25519KeyPair::from_seed_unchecked(seed)
            .ok()
            .map(|pair| Self { pair })
    }

    /// The public key.
    pub fn public_key(&self) -> Vec<u8> {
        self.pair.public_key().as_ref().to_vec()
    }

    /// The id of the key.
    pub fn key_id(&self) -> String {
        key_id(self.pair.public_key().as_ref())
    }

    /// Signs the checkpoint over the entries given: their bytes, in
    /// order, with the number of the first. `None` for no entries.
    pub fn checkpoint(&self, first: u64, entries: &[Vec<u8>], time: &str) -> Option<Checkpoint> {
        let last_bytes = entries.last()?;
        let leaves: Vec<[u8; 32]> = entries.iter().map(|bytes| merkle::leaf(bytes)).collect();
        let mut checkpoint = Checkpoint {
            first,
            last: first + u64::try_from(entries.len()).ok()? - 1,
            root: merkle::root(&leaves),
            head: Sha256::digest(last_bytes).into(),
            time: time.to_owned(),
            key_id: self.key_id(),
            signature: Vec::new(),
        };
        checkpoint.signature = self.pair.sign(&checkpoint.signed_bytes()).as_ref().to_vec();
        Some(checkpoint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkpoint_covers_a_range_and_names_its_key() {
        let signer = Signer::from_seed(&[7; 32]).unwrap();
        let entries = vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()];
        let checkpoint = signer
            .checkpoint(5, &entries, "2026-10-13T00:00:00.000Z")
            .unwrap();
        assert_eq!((checkpoint.first, checkpoint.last), (5, 7));
        assert_eq!(checkpoint.signature.len(), 64);
        assert!(checkpoint.key_id.starts_with("ed25519:"));
        assert_eq!(checkpoint.key_id.len(), 8 + 16);
        assert_eq!(checkpoint.head, <[u8; 32]>::from(Sha256::digest(b"three")));
        assert!(signer.checkpoint(1, &[], "t").is_none());
        // The same key and the same entries give the same signature.
        let again = signer
            .checkpoint(5, &entries, "2026-10-13T00:00:00.000Z")
            .unwrap();
        assert_eq!(again, checkpoint);
        assert!(Signer::from_seed(&[1; 31]).is_none());
        assert!(!format!("{signer:?}").contains("pair"));
    }
}
