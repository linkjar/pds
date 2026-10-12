//! Content addressing: the CID forms a repository accepts (SPEC §6.1).
//!
//! Blocks are addressed by CIDv1 over SHA-256. Tree nodes, commits and
//! records use the `dag-cbor` codec; blobs use `raw`. Any other version,
//! codec or hash makes the object invalid.

use cid::Cid;
use cid::multihash::Multihash;
use sha2::{Digest, Sha256};

use crate::cbor::MAX_CID_LEN;

/// The multicodec code of `dag-cbor`.
pub const DAG_CBOR: u64 = 0x71;
/// The multicodec code of `raw`.
pub const RAW: u64 = 0x55;
/// The multihash code of SHA-256.
pub const SHA2_256: u64 = 0x12;

/// What a repository CID addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CidKind {
    /// A tree node, a commit or a record: `dag-cbor`.
    DagCbor,
    /// A blob: `raw`.
    Raw,
}

/// A CID that a repository does not accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CidError {
    /// Not CIDv1.
    #[error("CID must be version 1")]
    Version,
    /// A codec other than `dag-cbor` and `raw`.
    #[error("CID codec must be dag-cbor or raw")]
    Codec,
    /// A hash other than SHA-256 with a 32-byte digest.
    #[error("CID hash must be SHA-256")]
    Hash,
    /// The binary form is longer than 100 bytes.
    #[error("CID is longer than 100 bytes")]
    TooLong,
}

/// The SHA-256 digest of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn cid_v1(codec: u64, digest: &[u8; 32]) -> Cid {
    // A 32-byte digest always fits the 64-byte multihash; the fallback is
    // never taken.
    let hash = Multihash::<64>::wrap(SHA2_256, digest).unwrap_or_default();
    Cid::new_v1(codec, hash)
}

/// The CID of a DAG-CBOR block: a tree node, a commit or a record.
pub fn cid_for_cbor(bytes: &[u8]) -> Cid {
    cid_v1(DAG_CBOR, &sha256(bytes))
}

/// The CID of a blob.
pub fn cid_for_raw(bytes: &[u8]) -> Cid {
    cid_v1(RAW, &sha256(bytes))
}

/// The CID of a blob whose SHA-256 digest is already known.
pub fn cid_for_raw_digest(digest: &[u8; 32]) -> Cid {
    cid_v1(RAW, digest)
}

/// Classifies a CID by the rules of SPEC §6.1 and rejects every other form.
pub fn repo_cid_kind(cid: &Cid) -> Result<CidKind, CidError> {
    if cid.version() != cid::Version::V1 {
        return Err(CidError::Version);
    }
    let hash = cid.hash();
    if hash.code() != SHA2_256 || hash.size() != 32 {
        return Err(CidError::Hash);
    }
    match cid.codec() {
        DAG_CBOR => Ok(CidKind::DagCbor),
        RAW => Ok(CidKind::Raw),
        _ => Err(CidError::Codec),
    }
}

/// Whether `cid` is the SHA-256 address of `bytes`. A CID with another hash
/// is never a match.
pub fn cid_matches(cid: &Cid, bytes: &[u8]) -> bool {
    let hash = cid.hash();
    hash.code() == SHA2_256 && hash.digest() == sha256(bytes)
}

/// Rejects a CID whose binary form is longer than the limit of SPEC §6.1.
pub fn check_cid_len(cid: &Cid) -> Result<(), CidError> {
    if cid.encoded_len() > MAX_CID_LEN {
        Err(CidError::TooLong)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_match_known_vectors() {
        // The empty DAG-CBOR map, `a0`.
        assert_eq!(
            cid_for_cbor(&[0xa0]).to_string(),
            "bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua"
        );
        // An empty blob.
        assert_eq!(
            cid_for_raw(&[]).to_string(),
            "bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku"
        );
        assert!(cid_matches(&cid_for_raw(b"abc"), b"abc"));
        assert!(!cid_matches(&cid_for_raw(b"abc"), b"abd"));
    }

    #[test]
    fn only_the_two_blessed_forms_pass() {
        assert_eq!(repo_cid_kind(&cid_for_cbor(b"x")), Ok(CidKind::DagCbor));
        assert_eq!(repo_cid_kind(&cid_for_raw(b"x")), Ok(CidKind::Raw));
        // CIDv0.
        let v0 = Cid::try_from("QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG").unwrap();
        assert_eq!(repo_cid_kind(&v0), Err(CidError::Version));
        // dag-pb.
        let hash = Multihash::<64>::wrap(SHA2_256, &sha256(b"x")).unwrap();
        assert_eq!(
            repo_cid_kind(&Cid::new_v1(0x70, hash)),
            Err(CidError::Codec)
        );
        // SHA-512.
        let long = Multihash::<64>::wrap(0x13, &[0u8; 64]).unwrap();
        assert_eq!(
            repo_cid_kind(&Cid::new_v1(DAG_CBOR, long)),
            Err(CidError::Hash)
        );
        // A truncated SHA-256.
        let short = Multihash::<64>::wrap(SHA2_256, &[0u8; 20]).unwrap();
        assert_eq!(
            repo_cid_kind(&Cid::new_v1(DAG_CBOR, short)),
            Err(CidError::Hash)
        );
    }
}
