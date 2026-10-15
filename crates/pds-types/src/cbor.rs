//! Strict DAG-CBOR with the decode limits of SPEC §6.1.
//!
//! [`Reader`] is a pull parser over a byte slice and [`Writer`] its inverse.
//! Both handle the subset of CBOR that DAG-CBOR allows: integers, byte
//! strings, text strings, arrays, maps with text keys, tag 42 over a CID,
//! booleans and null. Everything else is an error: floats, other tags,
//! `undefined`, indefinite lengths, integers that are not in their shortest
//! form, map keys out of canonical order or repeated, and text that is not
//! UTF-8. An input that decodes therefore encodes back to the same bytes.
//!
//! The limits are applied while reading, before a value is built, so a
//! hostile input costs no more than its own length.

use std::collections::BTreeMap;

use cid::Cid;

use crate::content::is_canonical_cid_bytes;
use crate::data::{Bytes, CidLink, Data};

/// The largest integer magnitude in the data model: 2^53 - 1 (SPEC §6.1).
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
/// The largest DAG-CBOR record in bytes (SPEC §6.1).
pub const MAX_CBOR_RECORD_LEN: usize = 1024 * 1024;
/// The largest JSON record in bytes (SPEC §6.1).
pub const MAX_JSON_RECORD_LEN: usize = 2 * 1024 * 1024;
/// The firehose frame limit in bytes (SPEC §6.1, §6.3).
pub const MAX_FRAME_LEN: usize = 5_000_000;
/// The deepest nesting of arrays and objects (SPEC §6.1).
pub const MAX_DEPTH: usize = 32;
/// The most elements in one array or object (SPEC §6.1).
pub const MAX_CONTAINER_LEN: usize = 131_072;
/// The longest object key in bytes (SPEC §6.1).
pub const MAX_KEY_LEN: usize = 8 * 1024;
/// The longest binary CID in bytes (SPEC §6.1).
pub const MAX_CID_LEN: usize = 100;

/// Decode limits. [`Limits::RECORD`] holds the values of SPEC §6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Longest input in bytes.
    pub max_len: usize,
    /// Deepest nesting of arrays and objects. The top-level value is depth 1
    /// when it is a container.
    pub max_depth: usize,
    /// Most elements in one array or entries in one object.
    pub max_container_len: usize,
    /// Longest object key in bytes.
    pub max_key_len: usize,
    /// Longest binary CID in bytes.
    pub max_cid_len: usize,
    /// Largest integer magnitude.
    pub max_integer: i64,
}

impl Limits {
    /// The limits for one record in DAG-CBOR.
    pub const RECORD: Self = Self {
        max_len: MAX_CBOR_RECORD_LEN,
        max_depth: MAX_DEPTH,
        max_container_len: MAX_CONTAINER_LEN,
        max_key_len: MAX_KEY_LEN,
        max_cid_len: MAX_CID_LEN,
        max_integer: MAX_SAFE_INTEGER,
    };

    /// The limits for one record in JSON: the record limits with the JSON
    /// size.
    pub const JSON_RECORD: Self = Self::RECORD.with_max_len(MAX_JSON_RECORD_LEN);

    /// The limits for one firehose frame.
    pub const FRAME: Self = Self::RECORD.with_max_len(MAX_FRAME_LEN);

    /// The same limits with another input size.
    #[must_use]
    pub const fn with_max_len(mut self, max_len: usize) -> Self {
        self.max_len = max_len;
        self
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::RECORD
    }
}

/// A decode limit of SPEC §6.1 that an input exceeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LimitError {
    /// The input is longer than the limit.
    #[error("input is longer than the limit")]
    TooLong,
    /// Arrays and objects nest deeper than the limit.
    #[error("nesting is deeper than the limit")]
    TooDeep,
    /// An array or object has more elements than the limit.
    #[error("a container has more elements than the limit")]
    ContainerTooLong,
    /// An object key is longer than the limit.
    #[error("an object key is longer than the limit")]
    KeyTooLong,
    /// An integer is outside the range of the data model.
    #[error("an integer is outside the allowed range")]
    IntegerOutOfRange,
    /// A binary CID is longer than the limit.
    #[error("a CID is longer than the limit")]
    CidTooLong,
}

/// Why an input is not acceptable DAG-CBOR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CborError {
    /// The input ends inside a value.
    #[error("unexpected end of input")]
    UnexpectedEnd,
    /// Bytes follow the value.
    #[error("trailing bytes after the value")]
    TrailingBytes,
    /// The input exceeds a decode limit.
    #[error(transparent)]
    Limit(#[from] LimitError),
    /// A float. The data model has none.
    #[error("floats are not allowed in the data model")]
    Float,
    /// An integer or a length that is not in its shortest form.
    #[error("an integer is not in its shortest form")]
    NonMinimal,
    /// A string, array or map of indefinite length.
    #[error("indefinite lengths are not allowed")]
    Indefinite,
    /// A tag other than 42.
    #[error("only tag 42 is allowed")]
    Tag,
    /// `undefined`, another simple value, or a reserved encoding.
    #[error("a value that DAG-CBOR does not allow")]
    Unsupported,
    /// A text string that is not UTF-8.
    #[error("a text string is not UTF-8")]
    Utf8,
    /// An object key that is not a text string.
    #[error("an object key is not a text string")]
    KeyType,
    /// Object keys that are out of canonical order or repeated.
    #[error("object keys are not in canonical order")]
    KeyOrder,
    /// A link whose CID is malformed or lacks the leading zero byte.
    #[error("a link does not hold a valid CID")]
    Cid,
    /// A value of another type than the caller asked for.
    #[error("unexpected type: wanted {wanted}")]
    Type {
        /// What the caller asked for.
        wanted: &'static str,
    },
}

/// The head of the next value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Head {
    /// An integer.
    Integer(i64),
    /// A byte string of this length.
    Bytes(usize),
    /// A text string of this length in bytes.
    Text(usize),
    /// An array of this many elements.
    Array(usize),
    /// A map of this many entries.
    Map(usize),
    /// Tag 42; a byte string with the CID follows.
    Link,
    /// A boolean.
    Bool(bool),
    /// Null.
    Null,
}

/// A pull parser over DAG-CBOR bytes.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    limits: Limits,
}

impl<'a> Reader<'a> {
    /// A reader over `buf`. Fails when the input is longer than the limit.
    pub const fn new(buf: &'a [u8], limits: Limits) -> Result<Self, CborError> {
        if buf.len() > limits.max_len {
            return Err(CborError::Limit(LimitError::TooLong));
        }
        Ok(Self {
            buf,
            pos: 0,
            limits,
        })
    }

    /// The offset of the next unread byte.
    pub const fn position(&self) -> usize {
        self.pos
    }

    /// The bytes not read yet.
    pub fn remaining(&self) -> &'a [u8] {
        self.buf.get(self.pos..).unwrap_or_default()
    }

    /// Whether every byte was read.
    pub const fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// Fails when bytes follow the last value.
    pub const fn finish(&self) -> Result<(), CborError> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(CborError::TrailingBytes)
        }
    }

    fn byte(&mut self) -> Result<u8, CborError> {
        let b = *self.buf.get(self.pos).ok_or(CborError::UnexpectedEnd)?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], CborError> {
        let end = self.pos.checked_add(len).ok_or(CborError::UnexpectedEnd)?;
        let out = self
            .buf
            .get(self.pos..end)
            .ok_or(CborError::UnexpectedEnd)?;
        self.pos = end;
        Ok(out)
    }

    /// Reads the argument of a head byte and checks that it is minimal.
    fn argument(&mut self, info: u8) -> Result<u64, CborError> {
        let (value, floor) = match info {
            0..=23 => return Ok(u64::from(info)),
            24 => (u64::from(self.byte()?), 24),
            25 => {
                let b = self.take(2)?;
                let mut a = [0u8; 2];
                a.copy_from_slice(b);
                (u64::from(u16::from_be_bytes(a)), 1 << 8)
            }
            26 => {
                let b = self.take(4)?;
                let mut a = [0u8; 4];
                a.copy_from_slice(b);
                (u64::from(u32::from_be_bytes(a)), 1 << 16)
            }
            27 => {
                let b = self.take(8)?;
                let mut a = [0u8; 8];
                a.copy_from_slice(b);
                (u64::from_be_bytes(a), 1 << 32)
            }
            31 => return Err(CborError::Indefinite),
            _ => return Err(CborError::Unsupported),
        };
        if value < floor {
            return Err(CborError::NonMinimal);
        }
        Ok(value)
    }

    fn length(&mut self, info: u8) -> Result<usize, CborError> {
        let n = self.argument(info)?;
        // A length beyond the input can never be satisfied.
        usize::try_from(n).map_err(|_| CborError::UnexpectedEnd)
    }

    fn container(&mut self, info: u8) -> Result<usize, CborError> {
        let n = self.length(info)?;
        if n > self.limits.max_container_len {
            return Err(CborError::Limit(LimitError::ContainerTooLong));
        }
        Ok(n)
    }

    /// Reads the head of the next value. For a string the caller then reads
    /// the bytes; for a container, its elements; for a link, [`Reader::cid`].
    pub fn head(&mut self) -> Result<Head, CborError> {
        let b = self.byte()?;
        let info = b & 0x1f;
        match b >> 5 {
            0 => {
                let n = self.argument(info)?;
                let v = i64::try_from(n)
                    .map_err(|_| CborError::Limit(LimitError::IntegerOutOfRange))?;
                if v > self.limits.max_integer {
                    return Err(CborError::Limit(LimitError::IntegerOutOfRange));
                }
                Ok(Head::Integer(v))
            }
            1 => {
                let n = self.argument(info)?;
                let v = i64::try_from(n)
                    .map_err(|_| CborError::Limit(LimitError::IntegerOutOfRange))?;
                // The value is -1 - n.
                if v >= self.limits.max_integer {
                    return Err(CborError::Limit(LimitError::IntegerOutOfRange));
                }
                Ok(Head::Integer(-1 - v))
            }
            2 => Ok(Head::Bytes(self.length(info)?)),
            3 => Ok(Head::Text(self.length(info)?)),
            4 => Ok(Head::Array(self.container(info)?)),
            5 => Ok(Head::Map(self.container(info)?)),
            6 => {
                if self.argument(info)? == 42 {
                    Ok(Head::Link)
                } else {
                    Err(CborError::Tag)
                }
            }
            _ => match info {
                20 => Ok(Head::Bool(false)),
                21 => Ok(Head::Bool(true)),
                22 => Ok(Head::Null),
                25..=27 => Err(CborError::Float),
                _ => Err(CborError::Unsupported),
            },
        }
    }

    /// Whether the next value is null, without reading it.
    pub fn peek_null(&self) -> bool {
        self.buf.get(self.pos) == Some(&0xf6)
    }

    /// Reads a null.
    pub fn null(&mut self) -> Result<(), CborError> {
        match self.head()? {
            Head::Null => Ok(()),
            _ => Err(CborError::Type { wanted: "null" }),
        }
    }

    /// Reads an integer.
    pub fn integer(&mut self) -> Result<i64, CborError> {
        match self.head()? {
            Head::Integer(v) => Ok(v),
            _ => Err(CborError::Type { wanted: "integer" }),
        }
    }

    /// Reads a byte string.
    pub fn bytes(&mut self) -> Result<&'a [u8], CborError> {
        match self.head()? {
            Head::Bytes(n) => self.take(n),
            _ => Err(CborError::Type { wanted: "bytes" }),
        }
    }

    /// Reads a text string.
    pub fn text(&mut self) -> Result<&'a str, CborError> {
        match self.head()? {
            Head::Text(n) => self.text_body(n),
            _ => Err(CborError::Type { wanted: "text" }),
        }
    }

    fn text_body(&mut self, len: usize) -> Result<&'a str, CborError> {
        std::str::from_utf8(self.take(len)?).map_err(|_| CborError::Utf8)
    }

    /// Reads an array head and returns its length.
    pub fn array(&mut self) -> Result<usize, CborError> {
        match self.head()? {
            Head::Array(n) => Ok(n),
            _ => Err(CborError::Type { wanted: "array" }),
        }
    }

    /// Reads a map head and returns its number of entries.
    pub fn map(&mut self) -> Result<usize, CborError> {
        match self.head()? {
            Head::Map(n) => Ok(n),
            _ => Err(CborError::Type { wanted: "map" }),
        }
    }

    /// Reads a map key and checks the length limit. The caller checks the
    /// order with [`key_order`].
    pub fn key(&mut self) -> Result<&'a str, CborError> {
        match self.head()? {
            Head::Text(n) if n > self.limits.max_key_len => {
                Err(CborError::Limit(LimitError::KeyTooLong))
            }
            Head::Text(n) => self.text_body(n),
            _ => Err(CborError::KeyType),
        }
    }

    /// Reads a link: tag 42 over the CID bytes.
    pub fn link(&mut self) -> Result<Cid, CborError> {
        match self.head()? {
            Head::Link => self.cid(),
            _ => Err(CborError::Type { wanted: "link" }),
        }
    }

    /// Reads the byte string that follows tag 42: a zero byte and the binary
    /// CID, which must not be longer than the limit.
    pub fn cid(&mut self) -> Result<Cid, CborError> {
        let raw = self.bytes().map_err(|_| CborError::Cid)?;
        let body = raw.strip_prefix(&[0u8]).ok_or(CborError::Cid)?;
        if body.len() > self.limits.max_cid_len {
            return Err(LimitError::CidTooLong.into());
        }
        let cid = Cid::try_from(body).map_err(|_| CborError::Cid)?;
        // `Cid::try_from` stops at the end of the CID and reads varints
        // that overflow or run long. Only the bytes the CID encodes to are
        // a link; anything else would not encode back to the input.
        if !is_canonical_cid_bytes(&cid, body) {
            return Err(CborError::Cid);
        }
        Ok(cid)
    }

    /// Reads one value of any type and discards it, with every check
    /// applied. `depth` is the nesting depth of the value's parent.
    pub fn skip(&mut self, depth: usize) -> Result<(), CborError> {
        match self.head()? {
            Head::Integer(_) | Head::Bool(_) | Head::Null => Ok(()),
            Head::Bytes(n) => self.take(n).map(|_| ()),
            Head::Text(n) => self.text_body(n).map(|_| ()),
            Head::Link => self.cid().map(|_| ()),
            Head::Array(n) => {
                let depth = self.descend(depth)?;
                for _ in 0..n {
                    self.skip(depth)?;
                }
                Ok(())
            }
            Head::Map(n) => {
                let depth = self.descend(depth)?;
                let mut prev: Option<&str> = None;
                for _ in 0..n {
                    let key = self.key()?;
                    key_order(prev, key)?;
                    prev = Some(key);
                    self.skip(depth)?;
                }
                Ok(())
            }
        }
    }

    const fn descend(&self, depth: usize) -> Result<usize, CborError> {
        if depth >= self.limits.max_depth {
            Err(CborError::Limit(LimitError::TooDeep))
        } else {
            Ok(depth + 1)
        }
    }

    /// Reads one value into the data model. `depth` is the nesting depth of
    /// the value's parent: zero for a top-level value.
    pub fn data(&mut self, depth: usize) -> Result<Data, CborError> {
        Ok(match self.head()? {
            Head::Integer(v) => Data::Integer(v),
            Head::Bool(b) => Data::Bool(b),
            Head::Null => Data::Null,
            Head::Bytes(n) => Data::Bytes(Bytes(self.take(n)?.to_vec())),
            Head::Text(n) => Data::String(self.text_body(n)?.to_owned()),
            Head::Link => Data::Link(Box::new(CidLink(self.cid()?))),
            Head::Array(n) => {
                let depth = self.descend(depth)?;
                // Every element takes at least one byte, so the declared
                // length cannot reserve more than the input holds.
                let mut out = Vec::with_capacity(n.min(self.remaining().len()));
                for _ in 0..n {
                    out.push(self.data(depth)?);
                }
                Data::Array(out)
            }
            Head::Map(n) => {
                let depth = self.descend(depth)?;
                let mut out = BTreeMap::new();
                let mut prev: Option<&str> = None;
                for _ in 0..n {
                    let key = self.key()?;
                    key_order(prev, key)?;
                    prev = Some(key);
                    let value = self.data(depth)?;
                    out.insert(key.to_owned(), value);
                }
                Data::Object(out)
            }
        })
    }
}

/// Checks that `key` follows `prev` in the canonical DAG-CBOR order: shorter
/// keys first, then bytewise. An equal key is a duplicate and fails too.
pub fn key_order(prev: Option<&str>, key: &str) -> Result<(), CborError> {
    match prev {
        Some(p) if !canonical_less(p, key) => Err(CborError::KeyOrder),
        _ => Ok(()),
    }
}

fn canonical_less(a: &str, b: &str) -> bool {
    (a.len(), a.as_bytes()) < (b.len(), b.as_bytes())
}

/// Decodes one DAG-CBOR value into the data model under `limits`. Nothing
/// may follow the value.
pub fn decode(bytes: &[u8], limits: Limits) -> Result<Data, CborError> {
    let mut r = Reader::new(bytes, limits)?;
    let v = r.data(0)?;
    r.finish()?;
    Ok(v)
}

/// Checks that `bytes` hold one acceptable DAG-CBOR value under `limits`
/// without building it. Use it before a typed decode of untrusted bytes.
pub fn check(bytes: &[u8], limits: Limits) -> Result<(), CborError> {
    let mut r = Reader::new(bytes, limits)?;
    r.skip(0)?;
    r.finish()
}

/// Writes canonical DAG-CBOR into a byte vector.
#[derive(Debug, Clone, Default)]
pub struct Writer {
    out: Vec<u8>,
}

impl Writer {
    /// An empty writer.
    pub const fn new() -> Self {
        Self { out: Vec::new() }
    }

    /// A writer with room for `capacity` bytes.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            out: Vec::with_capacity(capacity),
        }
    }

    /// The bytes written so far.
    pub fn as_slice(&self) -> &[u8] {
        &self.out
    }

    /// The bytes written.
    pub fn into_vec(self) -> Vec<u8> {
        self.out
    }

    fn head(&mut self, major: u8, n: u64) {
        let m = major << 5;
        if n < 24 {
            // The match arm bounds `n` below 24.
            self.out.push(m | u8::try_from(n).unwrap_or(0));
        } else if let Ok(v) = u8::try_from(n) {
            self.out.push(m | 0x18);
            self.out.push(v);
        } else if let Ok(v) = u16::try_from(n) {
            self.out.push(m | 0x19);
            self.out.extend_from_slice(&v.to_be_bytes());
        } else if let Ok(v) = u32::try_from(n) {
            self.out.push(m | 0x1a);
            self.out.extend_from_slice(&v.to_be_bytes());
        } else {
            self.out.push(m | 0x1b);
            self.out.extend_from_slice(&n.to_be_bytes());
        }
    }

    /// Writes an integer.
    pub fn integer(&mut self, v: i64) {
        if v >= 0 {
            self.head(0, v.unsigned_abs());
        } else {
            // -1 - v, which fits because v is negative.
            self.head(1, (v + 1).unsigned_abs());
        }
    }

    /// Writes a byte string.
    pub fn bytes(&mut self, v: &[u8]) {
        self.head(2, v.len() as u64);
        self.out.extend_from_slice(v);
    }

    /// Writes a text string.
    pub fn text(&mut self, v: &str) {
        self.head(3, v.len() as u64);
        self.out.extend_from_slice(v.as_bytes());
    }

    /// Writes an array head; the caller writes `len` elements after it.
    pub fn array(&mut self, len: usize) {
        self.head(4, len as u64);
    }

    /// Writes a map head; the caller writes `len` entries after it, keys in
    /// canonical order.
    pub fn map(&mut self, len: usize) {
        self.head(5, len as u64);
    }

    /// Writes a link: tag 42 over a zero byte and the binary CID.
    pub fn link(&mut self, cid: &Cid) {
        self.out.extend_from_slice(&[0xd8, 42]);
        self.head(2, cid.encoded_len() as u64 + 1);
        self.out.push(0);
        // Writing into a vector does not fail.
        let _ = cid.write_bytes(&mut self.out);
    }

    /// Writes a boolean.
    pub fn bool(&mut self, v: bool) {
        self.out.push(if v { 0xf5 } else { 0xf4 });
    }

    /// Writes null.
    pub fn null(&mut self) {
        self.out.push(0xf6);
    }

    /// Writes a data model value, object keys in canonical order.
    pub fn data(&mut self, v: &Data) {
        match v {
            Data::Null => self.null(),
            Data::Bool(b) => self.bool(*b),
            Data::Integer(i) => self.integer(*i),
            Data::String(s) => self.text(s),
            Data::Bytes(b) => self.bytes(&b.0),
            Data::Link(l) => self.link(&l.0),
            Data::Array(a) => {
                self.array(a.len());
                for item in a {
                    self.data(item);
                }
            }
            Data::Object(o) => {
                self.map(o.len());
                let mut entries: Vec<(&String, &Data)> = o.iter().collect();
                entries.sort_unstable_by(|(a, _), (b, _)| {
                    (a.len(), a.as_bytes()).cmp(&(b.len(), b.as_bytes()))
                });
                for (k, item) in entries {
                    self.text(k);
                    self.data(item);
                }
            }
        }
    }
}

/// Encodes a data model value as canonical DAG-CBOR.
pub fn encode(v: &Data) -> Vec<u8> {
    let mut w = Writer::new();
    w.data(v);
    w.into_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn hex(s: &str) -> Vec<u8> {
        data_encoding::HEXLOWER.decode(s.as_bytes()).unwrap()
    }

    #[test]
    fn rejects_what_dag_cbor_forbids() {
        let cases: &[(&str, CborError)] = &[
            ("f93c00", CborError::Float),             // 1.0 as a half float
            ("fa3f800000", CborError::Float),         // 1.0 as a single
            ("fb3ff0000000000000", CborError::Float), // 1.0 as a double
            ("f7", CborError::Unsupported),           // undefined
            ("f0", CborError::Unsupported),           // simple value 16
            ("1817", CborError::NonMinimal),          // 23 in two bytes
            ("190017", CborError::NonMinimal),
            ("1a00000017", CborError::NonMinimal),
            ("1b0000000000000017", CborError::NonMinimal),
            ("5801aa", CborError::NonMinimal), // a length in two bytes
            ("9f01ff", CborError::Indefinite), // indefinite array
            ("bf6161 01ff", CborError::Indefinite), // indefinite map
            ("5f41aaff", CborError::Indefinite), // indefinite bytes
            ("c100", CborError::Tag),          // tag 1
            ("a2616201616101", CborError::KeyOrder), // b before a
            ("a2616101616102", CborError::KeyOrder), // a twice
            ("a2626161016162 02", CborError::KeyOrder), // longer key first
            ("a10101", CborError::KeyType),    // integer key
            ("62c328", CborError::Utf8),
            ("0000", CborError::TrailingBytes),
            ("8201", CborError::UnexpectedEnd),
            ("d82a4100", CborError::Cid), // a link without a CID
            (
                "1b0020000000000000",
                CborError::Limit(LimitError::IntegerOutOfRange),
            ), // 2^53
            (
                "3b0020000000000000",
                CborError::Limit(LimitError::IntegerOutOfRange),
            ), // -2^53 - 1
            (
                "3b001fffffffffffff",
                CborError::Limit(LimitError::IntegerOutOfRange),
            ), // -2^53
        ];
        for (input, want) in cases {
            let bytes = hex(&input.replace(' ', ""));
            assert_eq!(decode(&bytes, Limits::RECORD), Err(*want), "{input}");
            assert_eq!(check(&bytes, Limits::RECORD), Err(*want), "check {input}");
        }
    }

    #[test]
    fn accepts_the_integer_bounds() {
        assert_eq!(
            decode(&hex("1b001fffffffffffff"), Limits::RECORD),
            Ok(Data::Integer(MAX_SAFE_INTEGER))
        );
        assert_eq!(
            decode(&hex("3b001ffffffffffffe"), Limits::RECORD),
            Ok(Data::Integer(-MAX_SAFE_INTEGER))
        );
    }

    #[test]
    fn link_needs_the_zero_byte_and_an_exact_cid() {
        let cid =
            Cid::try_from("bafyreie5cvv4h45feadgeuwhbcutmh6t2ceseocckahdoe6uat64zmz454").unwrap();
        let mut w = Writer::new();
        w.link(&cid);
        let good = w.into_vec();
        assert_eq!(
            decode(&good, Limits::RECORD),
            Ok(Data::Link(Box::new(CidLink(cid))))
        );
        // Without the zero byte.
        let mut bad = vec![0xd8, 42, 0x58, 36];
        bad.extend_from_slice(&cid.to_bytes());
        assert_eq!(decode(&bad, Limits::RECORD), Err(CborError::Cid));
        // With a byte after the CID.
        let mut bad = vec![0xd8, 42, 0x58, 38, 0];
        bad.extend_from_slice(&cid.to_bytes());
        bad.push(0);
        assert_eq!(decode(&bad, Limits::RECORD), Err(CborError::Cid));
    }

    #[test]
    fn a_link_with_an_overflowing_varint_is_refused() {
        // Found by the `data_cbor` and `repo_node` fuzz targets: the CID
        // crate reads a ten-byte varint whose last byte does not fit 64
        // bits, so two byte strings gave one CID.
        let node = data_encoding::BASE64
            .decode(b"omFlgGFs2CpPAAH///////////8FAgH/")
            .unwrap();
        assert_eq!(decode(&node, Limits::RECORD), Err(CborError::Cid));
        assert_eq!(check(&node, Limits::RECORD), Err(CborError::Cid));
        let link: &[u8] = &[
            216, 42, 88, 43, 0, 1, 226, 226, 226, 226, 226, 226, 255, 255, 255, 35, 7, 29, 226,
            226, 8, 0, 0, 0, 0, 0, 0, 0, 226, 142, 244, 35, 244, 226, 226, 226, 226, 226, 226, 226,
            244, 35, 244, 105, 245, 32, 3,
        ];
        assert_eq!(decode(link, Limits::RECORD), Err(CborError::Cid));
    }

    #[test]
    fn depth_limit_is_thirty_two_containers() {
        let nested = |n: usize| {
            let mut b = vec![0x81; n];
            b.push(0x00);
            b
        };
        assert!(decode(&nested(MAX_DEPTH), Limits::RECORD).is_ok());
        assert_eq!(
            decode(&nested(MAX_DEPTH + 1), Limits::RECORD),
            Err(CborError::Limit(LimitError::TooDeep))
        );
        assert_eq!(
            check(&nested(MAX_DEPTH + 1), Limits::RECORD),
            Err(CborError::Limit(LimitError::TooDeep))
        );
    }

    #[test]
    fn container_key_and_size_limits() {
        // An array head that declares one element more than the limit.
        let mut w = Writer::new();
        w.array(MAX_CONTAINER_LEN + 1);
        assert_eq!(
            decode(w.as_slice(), Limits::RECORD),
            Err(CborError::Limit(LimitError::ContainerTooLong))
        );
        // A full array at the limit decodes.
        let mut w = Writer::new();
        w.array(MAX_CONTAINER_LEN);
        let mut bytes = w.into_vec();
        bytes.resize(bytes.len() + MAX_CONTAINER_LEN, 0x00);
        assert!(decode(&bytes, Limits::RECORD).is_ok());
        // A key of 8 KiB passes, one byte more does not.
        for (len, ok) in [(MAX_KEY_LEN, true), (MAX_KEY_LEN + 1, false)] {
            let mut w = Writer::new();
            w.map(1);
            w.text(&"k".repeat(len));
            w.null();
            let got = decode(w.as_slice(), Limits::RECORD);
            assert_eq!(got.is_ok(), ok, "key of {len} bytes");
            if !ok {
                assert_eq!(got, Err(CborError::Limit(LimitError::KeyTooLong)));
            }
        }
        // One byte over the record size.
        let mut w = Writer::new();
        w.bytes(&vec![0u8; MAX_CBOR_RECORD_LEN]);
        assert_eq!(
            decode(w.as_slice(), Limits::RECORD),
            Err(CborError::Limit(LimitError::TooLong))
        );
        assert!(decode(w.as_slice(), Limits::FRAME).is_ok());
    }

    #[test]
    fn a_declared_length_does_not_allocate() {
        // A map head that declares 2^17 entries and stops.
        let bytes = hex("ba00020000");
        assert_eq!(
            decode(&bytes, Limits::RECORD),
            Err(CborError::UnexpectedEnd)
        );
        // A byte string that declares more than the input holds.
        assert_eq!(
            decode(&hex("5a7fffffff00"), Limits::RECORD),
            Err(CborError::UnexpectedEnd)
        );
    }

    fn arb_data() -> impl Strategy<Value = Data> {
        let cid =
            Cid::try_from("bafyreie5cvv4h45feadgeuwhbcutmh6t2ceseocckahdoe6uat64zmz454").unwrap();
        let leaf = prop_oneof![
            Just(Data::Null),
            any::<bool>().prop_map(Data::Bool),
            (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).prop_map(Data::Integer),
            ".{0,12}".prop_map(Data::String),
            proptest::collection::vec(any::<u8>(), 0..24).prop_map(|b| Data::Bytes(Bytes(b))),
            Just(Data::Link(Box::new(CidLink(cid)))),
        ];
        leaf.prop_recursive(4, 48, 6, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..6).prop_map(Data::Array),
                proptest::collection::btree_map(".{0,6}", inner, 0..6).prop_map(Data::Object),
            ]
        })
    }

    proptest! {
        /// The writer and the reader agree with `serde_ipld_dagcbor`, an
        /// independent implementation, and decoding inverts encoding.
        #[test]
        fn agrees_with_the_serde_codec(v in arb_data()) {
            let ours = encode(&v);
            let theirs = serde_ipld_dagcbor::to_vec(&v).unwrap();
            prop_assert_eq!(&ours, &theirs);
            prop_assert_eq!(&decode(&ours, Limits::RECORD).unwrap(), &v);
            prop_assert!(check(&ours, Limits::RECORD).is_ok());
            let back: Data = serde_ipld_dagcbor::from_slice(&ours).unwrap();
            prop_assert_eq!(back, v);
        }

        /// Whatever decodes encodes back to the same bytes, and `check`
        /// accepts exactly what `decode` accepts.
        #[test]
        fn decoding_is_canonical(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            let decoded = decode(&bytes, Limits::RECORD);
            prop_assert_eq!(decoded.is_ok(), check(&bytes, Limits::RECORD).is_ok());
            if let Ok(v) = decoded {
                prop_assert_eq!(encode(&v), bytes);
            }
        }
    }
}
