//! The signed commit at the root of a repository (SPEC §6.1).
//!
//! A commit is the DAG-CBOR map `{did, version: 3, data, rev, prev, sig}`.
//! `data` is the root of the tree, `rev` a TID that grows with every
//! commit, and `prev` is present and null. `sig` signs the encoding of the
//! same map without `sig`.
//!
//! This crate does not sign or verify: it produces and reads the bytes, and
//! the caller applies a key. Key custody is `pds-identity`'s, and a key may
//! live in a KMS.

use cid::Cid;
use pds_types::cbor::{CborError, Limits, Reader, Writer};
use pds_types::content::{CidKind, cid_for_cbor, repo_cid_kind};
use pds_types::{Did, Tid};

use crate::error::RepoError;

/// The commit version this engine writes and reads.
pub const COMMIT_VERSION: i64 = 3;

/// A commit before it is signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsignedCommit {
    /// The account.
    pub did: Did,
    /// The revision.
    pub rev: Tid,
    /// The root of the tree.
    pub data: Cid,
    /// The previous commit. Version 3 writes null; a commit from elsewhere
    /// may name one, and the signature covers it.
    pub prev: Option<Cid>,
}

/// A signed commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The account.
    pub did: Did,
    /// The revision.
    pub rev: Tid,
    /// The root of the tree.
    pub data: Cid,
    /// The previous commit; null in every commit this engine writes.
    pub prev: Option<Cid>,
    /// The signature over [`Commit::unsigned_bytes`].
    pub sig: Vec<u8>,
}

fn write_fields(
    w: &mut Writer,
    did: &Did,
    rev: &Tid,
    sig: Option<&[u8]>,
    data: &Cid,
    prev: Option<&Cid>,
) {
    // Canonical key order: by length, then bytewise.
    w.map(if sig.is_some() { 6 } else { 5 });
    w.text("did");
    w.text(did.as_str());
    w.text("rev");
    w.text(rev.as_str());
    if let Some(sig) = sig {
        w.text("sig");
        w.bytes(sig);
    }
    w.text("data");
    w.link(data);
    w.text("prev");
    match prev {
        Some(cid) => w.link(cid),
        None => w.null(),
    }
    w.text("version");
    w.integer(COMMIT_VERSION);
}

impl UnsignedCommit {
    /// A commit for a new revision, with a null `prev`.
    pub const fn new(did: Did, rev: Tid, data: Cid) -> Self {
        Self {
            did,
            rev,
            data,
            prev: None,
        }
    }

    /// The bytes that the signature covers.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(128);
        write_fields(
            &mut w,
            &self.did,
            &self.rev,
            None,
            &self.data,
            self.prev.as_ref(),
        );
        w.into_vec()
    }

    /// Attaches a signature that the caller made over [`Self::to_bytes`].
    pub fn with_signature(self, sig: Vec<u8>) -> Commit {
        Commit {
            did: self.did,
            rev: self.rev,
            data: self.data,
            prev: self.prev,
            sig,
        }
    }

    /// Signs with the caller's key: `sign` receives the bytes to sign and
    /// returns the signature.
    pub fn sign<E>(self, sign: impl FnOnce(&[u8]) -> Result<Vec<u8>, E>) -> Result<Commit, E> {
        let sig = sign(&self.to_bytes())?;
        Ok(self.with_signature(sig))
    }
}

impl Commit {
    /// The commit block.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(200);
        write_fields(
            &mut w,
            &self.did,
            &self.rev,
            Some(&self.sig),
            &self.data,
            self.prev.as_ref(),
        );
        w.into_vec()
    }

    /// The CID of the commit block.
    pub fn cid(&self) -> Cid {
        cid_for_cbor(&self.to_bytes())
    }

    /// The bytes that the signature covers: the commit without `sig`.
    pub fn unsigned_bytes(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(128);
        write_fields(
            &mut w,
            &self.did,
            &self.rev,
            None,
            &self.data,
            self.prev.as_ref(),
        );
        w.into_vec()
    }

    /// Checks the signature with the caller's key: `verify` receives the
    /// signed bytes and the signature.
    pub fn verify(&self, verify: impl FnOnce(&[u8], &[u8]) -> bool) -> Result<(), RepoError> {
        if verify(&self.unsigned_bytes(), &self.sig) {
            Ok(())
        } else {
            Err(RepoError::Signature)
        }
    }

    /// Decodes a commit block.
    ///
    /// The block must be a version 3 commit in canonical encoding with
    /// exactly the six fields. Earlier versions and unknown fields are
    /// refused: their signatures cover bytes this reader would not
    /// reproduce.
    pub fn decode(bytes: &[u8]) -> Result<Self, RepoError> {
        Self::decode_inner(bytes).map_err(|e| match e {
            DecodeError::Cbor => RepoError::Commit("not a commit in canonical DAG-CBOR"),
            DecodeError::Rule(why) => RepoError::Commit(why),
        })
    }

    fn decode_inner(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes, Limits::RECORD)?;
        let field = |r: &mut Reader<'_>, want: &'static str| -> Result<(), DecodeError> {
            if r.key()? == want {
                Ok(())
            } else {
                Err(DecodeError::Rule("unexpected field"))
            }
        };
        if r.map()? != 6 {
            return Err(DecodeError::Rule("a commit has six fields"));
        }
        field(&mut r, "did")?;
        let did = Did::parse(r.text()?).map_err(|_| DecodeError::Rule("did is not a DID"))?;
        field(&mut r, "rev")?;
        let rev = Tid::parse(r.text()?).map_err(|_| DecodeError::Rule("rev is not a TID"))?;
        field(&mut r, "sig")?;
        let sig = r.bytes()?.to_vec();
        field(&mut r, "data")?;
        let data = r.link()?;
        if repo_cid_kind(&data) != Ok(CidKind::DagCbor) {
            return Err(DecodeError::Rule("data is not a dag-cbor SHA-256 CID"));
        }
        field(&mut r, "prev")?;
        let prev = if r.peek_null() {
            r.null()?;
            None
        } else {
            Some(r.link()?)
        };
        field(&mut r, "version")?;
        if r.integer()? != COMMIT_VERSION {
            return Err(DecodeError::Rule("unsupported commit version"));
        }
        r.finish()?;
        Ok(Self {
            did,
            rev,
            data,
            prev,
            sig,
        })
    }
}

enum DecodeError {
    Cbor,
    Rule(&'static str),
}

impl From<CborError> for DecodeError {
    fn from(_: CborError) -> Self {
        Self::Cbor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unsigned() -> UnsignedCommit {
        UnsignedCommit::new(
            Did::parse("did:plc:ewvi7nxzyoun6zhxrhs64oiz").unwrap(),
            Tid::parse("3jqfcqzm3fo2j").unwrap(),
            cid_for_cbor(&[0xa2, 0x61, b'e', 0x80, 0x61, b'l', 0xf6]),
        )
    }

    #[test]
    fn round_trip_and_signed_bytes() {
        let commit = unsigned().with_signature(vec![7u8; 64]);
        let bytes = commit.to_bytes();
        assert_eq!(Commit::decode(&bytes).unwrap(), commit);
        assert_eq!(commit.unsigned_bytes(), unsigned().to_bytes());
        assert_eq!(commit.cid(), cid_for_cbor(&bytes));
        // The data model decodes the block to the six fields, prev null.
        let data = pds_types::Data::from_dag_cbor(&bytes).unwrap();
        assert_eq!(data.get("version"), Some(&pds_types::Data::Integer(3)));
        assert_eq!(data.get("prev"), Some(&pds_types::Data::Null));
        assert_eq!(
            data.get("did"),
            Some(&pds_types::Data::String(
                "did:plc:ewvi7nxzyoun6zhxrhs64oiz".into()
            ))
        );
        // The caller's key decides.
        assert!(
            commit
                .verify(|msg, sig| msg == unsigned().to_bytes() && sig == [7u8; 64])
                .is_ok()
        );
        assert!(matches!(
            commit.verify(|_, _| false),
            Err(RepoError::Signature)
        ));
        let signed: Result<Commit, ()> = unsigned().sign(|_| Ok(vec![1, 2, 3]));
        assert_eq!(signed.unwrap().sig, vec![1, 2, 3]);
    }

    #[test]
    fn a_previous_commit_is_kept_and_signed() {
        let mut u = unsigned();
        u.prev = Some(cid_for_cbor(b"previous"));
        let commit = u.clone().with_signature(vec![0u8; 64]);
        let back = Commit::decode(&commit.to_bytes()).unwrap();
        assert_eq!(back.prev, u.prev);
        assert_eq!(back.unsigned_bytes(), u.to_bytes());
        assert_ne!(u.to_bytes(), unsigned().to_bytes());
    }

    #[test]
    fn decode_refuses_other_shapes() {
        let build = |version: i64, extra: bool, raw_data: bool| {
            let u = unsigned();
            let mut w = Writer::new();
            w.map(if extra { 7 } else { 6 });
            w.text("did");
            w.text(u.did.as_str());
            w.text("rev");
            w.text(u.rev.as_str());
            w.text("sig");
            w.bytes(&[0u8; 64]);
            w.text("data");
            if raw_data {
                w.link(&pds_types::content::cid_for_raw(b"x"));
            } else {
                w.link(&u.data);
            }
            w.text("prev");
            w.null();
            if extra {
                w.text("extra");
                w.null();
            }
            w.text("version");
            w.integer(version);
            w.into_vec()
        };
        assert!(Commit::decode(&build(3, false, false)).is_ok());
        assert!(matches!(
            Commit::decode(&build(2, false, false)),
            Err(RepoError::Commit("unsupported commit version"))
        ));
        assert!(Commit::decode(&build(3, true, false)).is_err());
        assert!(Commit::decode(&build(3, false, true)).is_err());
        // An unsigned commit is not a commit, nor is a record.
        assert!(Commit::decode(&unsigned().to_bytes()).is_err());
        assert!(Commit::decode(&[0xa0]).is_err());
        let mut trailing = build(3, false, false);
        trailing.push(0);
        assert!(Commit::decode(&trailing).is_err());
    }
}
