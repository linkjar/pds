//! CAR version 1: the archive format of repository exports, imports and
//! firehose commits.
//!
//! A CAR is a header and a sequence of blocks. The header is a varint
//! length and the DAG-CBOR map `{"roots": [link, ...], "version": 1}`. Each
//! block is a varint length, a binary CID and the block bytes.
//!
//! The reader takes untrusted bytes. It checks every block against its CID
//! before the block is used, accepts SHA-256 only (SPEC §6.1), keeps the
//! first copy of a block that appears twice, and tolerates any block order
//! (SPEC §6.5). Its work is linear in the input.

use std::io;

use cid::Cid;
use pds_types::cbor::{Limits, Reader, Writer};
use pds_types::content::{SHA2_256, cid_matches};

use crate::block::BlockMap;
use crate::error::RepoError;

/// The longest header that is read: room for some two hundred roots.
const MAX_HEADER_LEN: usize = 8 * 1024;

/// A CAR read into memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Car {
    /// The roots named in the header.
    pub roots: Vec<Cid>,
    /// The blocks, in file order, each once.
    pub blocks: BlockMap,
}

impl Car {
    /// The single root of a repository CAR.
    pub fn root(&self) -> Result<Cid, RepoError> {
        match self.roots.as_slice() {
            [root] => Ok(*root),
            _ => Err(RepoError::Car("expected exactly one root")),
        }
    }
}

/// Reads an unsigned LEB128 varint. `None` at the end of the input.
fn read_varint(buf: &[u8], pos: &mut usize) -> Result<Option<usize>, RepoError> {
    if *pos >= buf.len() {
        return Ok(None);
    }
    let mut value: u64 = 0;
    for shift in (0..64).step_by(7) {
        let byte = *buf.get(*pos).ok_or(RepoError::Car("truncated varint"))?;
        *pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return usize::try_from(value)
                .map(Some)
                .map_err(|_| RepoError::Car("length out of range"));
        }
    }
    Err(RepoError::Car("varint is too long"))
}

fn take<'a>(buf: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], RepoError> {
    let end = pos
        .checked_add(len)
        .ok_or(RepoError::Car("length out of range"))?;
    let out = buf.get(*pos..end).ok_or(RepoError::Car("truncated"))?;
    *pos = end;
    Ok(out)
}

fn read_header(bytes: &[u8]) -> Result<Vec<Cid>, RepoError> {
    let bad = |_| RepoError::Car("malformed header");
    let mut r = Reader::new(bytes, Limits::RECORD).map_err(bad)?;
    if r.map().map_err(bad)? != 2 || r.key().map_err(bad)? != "roots" {
        return Err(RepoError::Car("malformed header"));
    }
    let count = r.array().map_err(bad)?;
    let mut roots = Vec::with_capacity(count.min(bytes.len()));
    for _ in 0..count {
        roots.push(r.link().map_err(bad)?);
    }
    if r.key().map_err(bad)? != "version" {
        return Err(RepoError::Car("malformed header"));
    }
    if r.integer().map_err(bad)? != 1 {
        return Err(RepoError::Car("unsupported CAR version"));
    }
    r.finish().map_err(bad)?;
    Ok(roots)
}

/// Reads a CAR. The caller bounds the size of `bytes`; nothing here
/// allocates more than the input holds.
pub fn read_car(bytes: &[u8]) -> Result<Car, RepoError> {
    let mut pos = 0;
    let header_len = read_varint(bytes, &mut pos)?.ok_or(RepoError::Car("empty"))?;
    if header_len > MAX_HEADER_LEN {
        return Err(RepoError::Car("header is too long"));
    }
    let roots = read_header(take(bytes, &mut pos, header_len)?)?;
    let mut blocks = BlockMap::new();
    while let Some(len) = read_varint(bytes, &mut pos)? {
        let section = take(bytes, &mut pos, len)?;
        let mut rest = section;
        let cid = Cid::read_bytes(&mut rest).map_err(|_| RepoError::Car("malformed CID"))?;
        // Any codec may be archived; the address must be CIDv1 over SHA-256.
        let hash = cid.hash();
        if cid.version() != cid::Version::V1 || hash.code() != SHA2_256 || hash.size() != 32 {
            return Err(RepoError::Car("block CID is not CIDv1 with SHA-256"));
        }
        if !cid_matches(&cid, rest) {
            return Err(RepoError::CidMismatch(Box::new(cid)));
        }
        if !blocks.contains(&cid) {
            blocks.insert(cid, rest.to_vec());
        }
    }
    Ok(Car { roots, blocks })
}

fn write_varint<W: io::Write>(out: &mut W, mut value: usize) -> io::Result<()> {
    let mut buf = [0u8; 10];
    let mut n = 0;
    loop {
        // Seven bits at a time, least significant first.
        let low = u8::try_from(value & 0x7f).unwrap_or(0x7f);
        value >>= 7;
        let more = value != 0;
        if let Some(slot) = buf.get_mut(n) {
            *slot = if more { low | 0x80 } else { low };
        }
        n += 1;
        if !more {
            break;
        }
    }
    out.write_all(buf.get(..n).unwrap_or_default())
}

/// Writes a CAR block by block.
#[derive(Debug)]
pub struct CarWriter<W> {
    out: W,
    scratch: Vec<u8>,
}

impl<W: io::Write> CarWriter<W> {
    /// Writes the header. A repository CAR has one root, the commit; a CAR
    /// without a root has an empty list.
    pub fn new(mut out: W, root: Option<&Cid>) -> io::Result<Self> {
        let mut header = Writer::with_capacity(64);
        header.map(2);
        header.text("roots");
        header.array(usize::from(root.is_some()));
        if let Some(cid) = root {
            header.link(cid);
        }
        header.text("version");
        header.integer(1);
        write_varint(&mut out, header.as_slice().len())?;
        out.write_all(header.as_slice())?;
        Ok(Self {
            out,
            scratch: Vec::with_capacity(64),
        })
    }

    /// Writes one block.
    pub fn block(&mut self, cid: &Cid, bytes: &[u8]) -> io::Result<()> {
        self.scratch.clear();
        cid.write_bytes(&mut self.scratch)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        write_varint(&mut self.out, self.scratch.len() + bytes.len())?;
        self.out.write_all(&self.scratch)?;
        self.out.write_all(bytes)
    }

    /// Returns the output.
    pub fn finish(self) -> W {
        self.out
    }
}

/// Writes a block map as a CAR in memory, blocks in the map's order.
pub fn write_car(root: Option<&Cid>, blocks: &BlockMap) -> Vec<u8> {
    let mut out = Vec::with_capacity(blocks.byte_len() + blocks.len() * 40 + 64);
    // Writing into a vector does not fail.
    if let Ok(mut writer) = CarWriter::new(&mut out, root) {
        for (cid, bytes) in blocks.iter() {
            let _ = writer.block(cid, bytes);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use pds_types::content::{cid_for_cbor, cid_for_raw};

    use super::*;

    fn sample() -> (Cid, BlockMap) {
        let mut blocks = BlockMap::new();
        let root = blocks.insert_cbor(vec![0xa0]);
        blocks.insert_cbor(vec![0x01]);
        blocks.insert(cid_for_raw(b"blob"), b"blob".to_vec());
        (root, blocks)
    }

    #[test]
    fn round_trip() {
        let (root, blocks) = sample();
        let bytes = write_car(Some(&root), &blocks);
        let car = read_car(&bytes).unwrap();
        assert_eq!(car.root().unwrap(), root);
        assert_eq!(car.blocks, blocks);
        // A CAR without roots.
        let none = read_car(&write_car(None, &blocks)).unwrap();
        assert_eq!(none.roots, Vec::<Cid>::new());
        assert!(none.root().is_err());
        // The header layout is the Reference's: a2 "roots" [link] "version" 1.
        assert_eq!(bytes[0], 0x3a);
        assert_eq!(&bytes[1..8], &[0xa2, 0x65, b'r', b'o', b'o', b't', b's']);
    }

    #[test]
    fn duplicates_and_any_order() {
        let (root, blocks) = sample();
        let mut out = Vec::new();
        let mut w = CarWriter::new(&mut out, Some(&root)).unwrap();
        let listed: Vec<_> = blocks.iter().collect();
        for (cid, bytes) in listed.iter().rev() {
            w.block(cid, bytes).unwrap();
        }
        for (cid, bytes) in &listed {
            w.block(cid, bytes).unwrap();
        }
        let car = read_car(&out).unwrap();
        assert_eq!(car.blocks.len(), 3);
        assert_eq!(car.blocks.cids().next(), listed.last().map(|(c, _)| *c));
    }

    #[test]
    fn hostile_input() {
        let (root, blocks) = sample();
        let good = write_car(Some(&root), &blocks);
        // Every truncation fails or yields fewer blocks; none panics.
        for cut in 0..good.len() {
            if let Ok(car) = read_car(&good[..cut]) {
                assert!(car.blocks.len() < 3, "cut at {cut}");
            }
        }
        // A block whose bytes do not match its CID.
        let mut wrong = BlockMap::new();
        wrong.insert(cid_for_cbor(&[0x01]), vec![0x02]);
        assert!(matches!(
            read_car(&write_car(None, &wrong)),
            Err(RepoError::CidMismatch(_))
        ));
        // A SHA-512 CID.
        let hash = cid::multihash::Multihash::<64>::wrap(0x13, &[0u8; 64]).unwrap();
        let mut other = BlockMap::new();
        other.insert(Cid::new_v1(0x71, hash), vec![0x01]);
        assert!(read_car(&write_car(None, &other)).is_err());
        // Version 2, a header that is too long, and a length beyond the input.
        let mut v2 = Writer::new();
        v2.map(2);
        v2.text("roots");
        v2.array(0);
        v2.text("version");
        v2.integer(2);
        let mut bytes = vec![u8::try_from(v2.as_slice().len()).unwrap()];
        bytes.extend_from_slice(v2.as_slice());
        assert!(matches!(
            read_car(&bytes),
            Err(RepoError::Car("unsupported CAR version"))
        ));
        assert!(read_car(&[0xff, 0xff, 0x03]).is_err());
        assert!(read_car(&[0x80; 11]).is_err());
        assert!(read_car(&[]).is_err());
        let mut long = good;
        long.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert!(read_car(&long).is_err());
    }
}
