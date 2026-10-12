//! The atproto data model (SPEC §6.1, §6.3).
//!
//! One value type with two wire forms. JSON uses `{"$link": "<cid>"}` for
//! links and `{"$bytes": "<base64>"}` for bytes; DAG-CBOR uses CID tag 42 and
//! byte strings. The serializer's `is_human_readable` flag picks the form,
//! so `serde_json` and `serde_ipld_dagcbor` both work on the same types.
//! Floats are not part of the model and are rejected on input.
//!
//! Untrusted input goes through [`Data::from_json_slice`] and
//! [`Data::from_dag_cbor`], which apply the decode limits of SPEC §6.1.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::LazyLock;

use data_encoding::{BASE64_NOPAD, Encoding, Specification};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::cbor::{self, CborError, LimitError, Limits, MAX_CID_LEN, MAX_SAFE_INTEGER};

/// Why a JSON document is not a data model value.
#[derive(Debug, thiserror::Error)]
pub enum JsonError {
    /// Not JSON, or JSON that the data model does not allow: a fractional
    /// number, a malformed `$link` or `$bytes`.
    #[error(transparent)]
    Syntax(#[from] serde_json::Error),
    /// The document exceeds a decode limit of SPEC §6.1.
    #[error(transparent)]
    Limit(#[from] LimitError),
}

/// Byte string with the JSON form `{"$bytes": "<standard base64, no padding>"}`.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Bytes(pub Vec<u8>);

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bytes({} bytes)", self.0.len())
    }
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            let mut m = s.serialize_map(Some(1))?;
            m.serialize_entry("$bytes", &BASE64_NOPAD.encode(&self.0))?;
            m.end()
        } else {
            s.serialize_bytes(&self.0)
        }
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if d.is_human_readable() {
            #[derive(Deserialize)]
            struct Repr {
                #[serde(rename = "$bytes")]
                bytes: String,
            }
            let r = Repr::deserialize(d)?;
            decode_base64(&r.bytes)
                .map(Bytes)
                .map_err(de::Error::custom)
        } else {
            struct V;
            impl Visitor<'_> for V {
                type Value = Bytes;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("a byte string")
                }
                fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Bytes, E> {
                    Ok(Bytes(v.to_vec()))
                }
                fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Bytes, E> {
                    Ok(Bytes(v))
                }
            }
            d.deserialize_bytes(V)
        }
    }
}

/// Standard base64 that does not require the unused bits of the last
/// character to be zero.
#[allow(clippy::expect_used)] // a static alphabet that does not build is a build defect
static BASE64_LOOSE: LazyLock<Encoding> = LazyLock::new(|| {
    let mut spec = Specification::new();
    spec.symbols
        .push_str("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/");
    spec.check_trailing_bits = false;
    spec.encoding().expect("static base64 alphabet")
});

/// Decodes standard base64 as the Reference does.
///
/// Padding is optional and the unused bits of the last character need not
/// be zero: `"123"` decodes to two bytes. A lone trailing character and any
/// character outside the alphabet are errors.
pub fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    let trimmed = s.trim_end_matches('=');
    if s.len() - trimmed.len() > 2 {
        return Err("invalid base64: too much padding".to_owned());
    }
    BASE64_LOOSE
        .decode(trimmed.as_bytes())
        .map_err(|e| format!("invalid base64: {e}"))
}

/// The longest `$link` string that is parsed. A 100-byte CID is 161
/// characters in base32; no base the parser accepts needs more than this.
const MAX_LINK_STR_LEN: usize = 256;

/// Parses the string of a `$link`, bounded before and after decoding.
fn parse_link(s: &str) -> Result<cid::Cid, String> {
    if s.len() > MAX_LINK_STR_LEN {
        return Err("CID string is too long".to_owned());
    }
    let c = cid::Cid::try_from(s).map_err(|e| format!("invalid CID: {e}"))?;
    if c.encoded_len() > MAX_CID_LEN {
        return Err("CID is longer than 100 bytes".to_owned());
    }
    Ok(c)
}

/// A link to another block, `{"$link": "<cid>"}` in JSON and tag 42 in CBOR.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CidLink(pub cid::Cid);

impl fmt::Debug for CidLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CidLink({})", self.0)
    }
}

impl Serialize for CidLink {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            let mut m = s.serialize_map(Some(1))?;
            m.serialize_entry("$link", &self.0.to_string())?;
            m.end()
        } else {
            self.0.serialize(s)
        }
    }
}

impl<'de> Deserialize<'de> for CidLink {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if d.is_human_readable() {
            #[derive(Deserialize)]
            struct Repr {
                #[serde(rename = "$link")]
                link: String,
            }
            let r = Repr::deserialize(d)?;
            parse_link(&r.link).map(CidLink).map_err(de::Error::custom)
        } else {
            d.deserialize_any(LinkVisitor)
        }
    }
}

/// Reads a DAG-CBOR link (tag 42 over the CID bytes with a leading identity
/// multibase byte) without going through the CID crate's own visitor.
struct LinkVisitor;

impl<'de> Visitor<'de> for LinkVisitor {
    type Value = CidLink;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a CID link (CBOR tag 42)")
    }
    fn visit_newtype_struct<D: Deserializer<'de>>(self, d: D) -> Result<CidLink, D::Error> {
        d.deserialize_bytes(RawBytes)
            .and_then(|b| cid_from_tagged_bytes(&b).map_err(de::Error::custom))
    }
    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<CidLink, E> {
        cid_from_tagged_bytes(v).map_err(de::Error::custom)
    }
    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<CidLink, E> {
        cid_from_tagged_bytes(&v).map_err(de::Error::custom)
    }
}

struct RawBytes;

impl Visitor<'_> for RawBytes {
    type Value = Vec<u8>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bytes")
    }
    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
        Ok(v.to_vec())
    }
    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
        Ok(v)
    }
}

fn cid_from_tagged_bytes(b: &[u8]) -> Result<CidLink, String> {
    // The wire form carries a leading 0x00 identity-multibase byte; the
    // decoder may or may not have stripped it already.
    let stripped = b.strip_prefix(&[0u8]).unwrap_or(b);
    cid::Cid::try_from(stripped)
        .or_else(|_| cid::Cid::try_from(b))
        .map(CidLink)
        .map_err(|e| format!("invalid CID bytes: {e}"))
}

/// A blob reference as it appears inside records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRef {
    /// Always `"blob"`.
    #[serde(rename = "$type")]
    pub type_: BlobType,
    /// The blob's CID; must use the `raw` codec.
    #[serde(rename = "ref")]
    pub ref_: CidLink,
    /// The MIME type, non-empty.
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    /// The size in bytes, greater than zero.
    pub size: i64,
}

/// The literal `"blob"` tag of a blob reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlobType;

impl Serialize for BlobType {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("blob")
    }
}

impl<'de> Deserialize<'de> for BlobType {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s == "blob" {
            Ok(BlobType)
        } else {
            Err(de::Error::custom("expected $type \"blob\""))
        }
    }
}

/// A value of the atproto data model.
#[derive(Clone, PartialEq, Eq)]
pub enum Data {
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// A signed 64-bit integer; the JSON form stays within 2^53.
    Integer(i64),
    /// A UTF-8 string.
    String(String),
    /// A byte string.
    Bytes(Bytes),
    /// A link, boxed so `Data` stays 32 bytes (a `Cid` carries its digest inline).
    Link(Box<CidLink>),
    /// An array.
    Array(Vec<Data>),
    /// An object with keys in sorted order (the canonical DAG-CBOR order is
    /// applied by the encoder).
    Object(BTreeMap<String, Data>),
}

impl fmt::Debug for Data {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Data::Null => f.write_str("null"),
            Data::Bool(b) => write!(f, "{b}"),
            Data::Integer(i) => write!(f, "{i}"),
            Data::String(s) => write!(f, "{s:?}"),
            Data::Bytes(b) => write!(f, "{b:?}"),
            Data::Link(l) => write!(f, "{l:?}"),
            Data::Array(a) => f.debug_list().entries(a).finish(),
            Data::Object(o) => f.debug_map().entries(o).finish(),
        }
    }
}

impl Data {
    /// The `$type` of an object, when present and a string.
    pub fn type_name(&self) -> Option<&str> {
        match self {
            Data::Object(o) => match o.get("$type") {
                Some(Data::String(s)) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }

    /// Looks up a key of an object.
    pub fn get(&self, key: &str) -> Option<&Data> {
        match self {
            Data::Object(o) => o.get(key),
            _ => None,
        }
    }

    /// Parses a JSON record into the data model under the limits of SPEC
    /// §6.1 for a JSON record.
    pub fn from_json_str(s: &str) -> Result<Data, JsonError> {
        Self::from_json_slice(s.as_bytes(), Limits::JSON_RECORD)
    }

    /// Parses a JSON document into the data model under `limits`. The size
    /// is checked before parsing and the other limits on the parsed value.
    pub fn from_json_slice(bytes: &[u8], limits: Limits) -> Result<Data, JsonError> {
        if bytes.len() > limits.max_len {
            return Err(LimitError::TooLong.into());
        }
        // The parser stops at 128 levels of nesting on its own.
        let v: Data = serde_json::from_slice(bytes)?;
        v.check_limits(limits)?;
        Ok(v)
    }

    /// Checks depth, container size, key length, integer range and CID
    /// length against `limits`. The input size is the caller's to check.
    pub fn check_limits(&self, limits: Limits) -> Result<(), LimitError> {
        self.check_limits_at(limits, 0)
    }

    fn check_limits_at(&self, limits: Limits, depth: usize) -> Result<(), LimitError> {
        match self {
            Data::Null | Data::Bool(_) | Data::String(_) | Data::Bytes(_) => Ok(()),
            Data::Integer(i) => {
                if i.unsigned_abs() > limits.max_integer.unsigned_abs() {
                    Err(LimitError::IntegerOutOfRange)
                } else {
                    Ok(())
                }
            }
            Data::Link(l) => {
                if l.0.encoded_len() > limits.max_cid_len {
                    Err(LimitError::CidTooLong)
                } else {
                    Ok(())
                }
            }
            Data::Array(a) => {
                if depth >= limits.max_depth {
                    return Err(LimitError::TooDeep);
                }
                if a.len() > limits.max_container_len {
                    return Err(LimitError::ContainerTooLong);
                }
                a.iter()
                    .try_for_each(|v| v.check_limits_at(limits, depth + 1))
            }
            Data::Object(o) => {
                if depth >= limits.max_depth {
                    return Err(LimitError::TooDeep);
                }
                if o.len() > limits.max_container_len {
                    return Err(LimitError::ContainerTooLong);
                }
                o.iter().try_for_each(|(k, v)| {
                    if k.len() > limits.max_key_len {
                        return Err(LimitError::KeyTooLong);
                    }
                    v.check_limits_at(limits, depth + 1)
                })
            }
        }
    }

    /// Encodes as canonical DAG-CBOR.
    pub fn to_dag_cbor(&self) -> Vec<u8> {
        cbor::encode(self)
    }

    /// Decodes one DAG-CBOR record under the limits of SPEC §6.1. The input
    /// must be canonical: see [`crate::cbor`].
    pub fn from_dag_cbor(bytes: &[u8]) -> Result<Data, CborError> {
        cbor::decode(bytes, Limits::RECORD)
    }

    /// Decodes one DAG-CBOR value under `limits`.
    pub fn from_dag_cbor_with(bytes: &[u8], limits: Limits) -> Result<Data, CborError> {
        cbor::decode(bytes, limits)
    }
}

impl Serialize for Data {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Data::Null => s.serialize_unit(),
            Data::Bool(b) => s.serialize_bool(*b),
            Data::Integer(i) => s.serialize_i64(*i),
            Data::String(v) => s.serialize_str(v),
            Data::Bytes(b) => b.serialize(s),
            Data::Link(l) => l.serialize(s),
            Data::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            Data::Object(o) => {
                let mut m = s.serialize_map(Some(o.len()))?;
                for (k, v) in o {
                    m.serialize_entry(k, v)?;
                }
                m.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Data {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let human = d.is_human_readable();
        d.deserialize_any(DataVisitor { human })
    }
}

struct DataVisitor {
    human: bool,
}

impl<'de> Visitor<'de> for DataVisitor {
    type Value = Data;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an atproto data model value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Data, E> {
        Ok(Data::Null)
    }
    fn visit_none<E: de::Error>(self) -> Result<Data, E> {
        Ok(Data::Null)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Data, D::Error> {
        Data::deserialize(d)
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Data, E> {
        Ok(Data::Bool(v))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Data, E> {
        Ok(Data::Integer(v))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Data, E> {
        i64::try_from(v)
            .map(Data::Integer)
            .map_err(|_| de::Error::custom("integer out of range"))
    }
    fn visit_i128<E: de::Error>(self, v: i128) -> Result<Data, E> {
        i64::try_from(v)
            .map(Data::Integer)
            .map_err(|_| de::Error::custom("integer out of range"))
    }
    fn visit_u128<E: de::Error>(self, v: u128) -> Result<Data, E> {
        i64::try_from(v)
            .map(Data::Integer)
            .map_err(|_| de::Error::custom("integer out of range"))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Data, E> {
        // JSON has one number type. `123.0` and `1e2` are integers to the
        // Reference, whose parser yields the same number for `123`; a
        // number with a fraction is a float and has no place in the model.
        #[allow(clippy::cast_precision_loss)] // 2^53 - 1 is exact in an f64
        let max = MAX_SAFE_INTEGER as f64;
        if v.fract() == 0.0 && v.abs() <= max {
            #[allow(clippy::cast_possible_truncation)] // whole and within 2^53
            return Ok(Data::Integer(v as i64));
        }
        Err(de::Error::custom(
            "floats are not allowed in the atproto data model",
        ))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Data, E> {
        Ok(Data::String(v.to_owned()))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Data, E> {
        Ok(Data::String(v))
    }
    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Data, E> {
        Ok(Data::Bytes(Bytes(v.to_vec())))
    }
    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Data, E> {
        Ok(Data::Bytes(Bytes(v)))
    }
    fn visit_newtype_struct<D: Deserializer<'de>>(self, d: D) -> Result<Data, D::Error> {
        // DAG-CBOR links arrive as a newtype over the tagged CID bytes.
        LinkVisitor
            .visit_newtype_struct(d)
            .map(|l| Data::Link(Box::new(l)))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Data, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = seq.next_element::<Data>()? {
            out.push(v);
        }
        Ok(Data::Array(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Data, A::Error> {
        let mut out = BTreeMap::new();
        while let Some(k) = map.next_key::<String>()? {
            let v = map.next_value::<Data>()?;
            if out.insert(k, v).is_some() {
                return Err(de::Error::custom("duplicate object key"));
            }
        }
        if self.human {
            // Recognise the JSON forms of links and bytes.
            if out.len() == 1 {
                if let Some(Data::String(s)) = out.get("$link") {
                    let c = parse_link(s).map_err(de::Error::custom)?;
                    return Ok(Data::Link(Box::new(CidLink(c))));
                }
                if let Some(Data::String(s)) = out.get("$bytes") {
                    let b = decode_base64(s).map_err(de::Error::custom)?;
                    return Ok(Data::Bytes(Bytes(b)));
                }
            }
            if out.contains_key("$link") || out.contains_key("$bytes") {
                return Err(de::Error::custom(
                    "$link and $bytes objects may have no other keys",
                ));
            }
        }
        Ok(Data::Object(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> serde_json::Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../interop/data-model")
            .join(name);
        serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).expect("json")
    }

    #[test]
    fn fixtures_roundtrip_json_and_cbor() {
        let cases = fixture("data-model-fixtures.json");
        for case in cases.as_array().expect("array") {
            let json_text = serde_json::to_string(&case["json"]).unwrap();
            let data = Data::from_json_str(&json_text).expect("json parses into the data model");
            let cbor = data.to_dag_cbor();
            assert_eq!(
                cbor,
                serde_ipld_dagcbor::to_vec(&data).expect("encodes"),
                "the two encoders agree"
            );
            let expected = decode_base64(case["cbor_base64"].as_str().unwrap()).unwrap();
            assert_eq!(cbor, expected, "cbor bytes differ for {json_text}");
            let back = Data::from_dag_cbor(&cbor).expect("decodes");
            assert_eq!(back, data);
            let json_again: serde_json::Value =
                serde_json::from_str(&serde_json::to_string(&back).unwrap()).unwrap();
            assert_eq!(json_again, case["json"]);
            let c = crate::content::cid_for_cbor(&cbor);
            assert_eq!(c.to_string(), case["cid"].as_str().unwrap());
        }
    }

    #[test]
    fn invalid_fixtures_are_rejected_or_flagged() {
        // The data-model-invalid cases mix wire-level problems (floats,
        // bad $type) with lexicon-level ones (blob fields). The wire-level
        // ones must fail here; the rest are pds-lexicon's job.
        let cases = fixture("data-model-invalid.json");
        let json_level = ["float", "top-level not an object"];
        for case in cases.as_array().unwrap() {
            let note = case["note"].as_str().unwrap_or("");
            let text = serde_json::to_string(&case["json"]).unwrap();
            let parsed = Data::from_json_str(&text);
            if json_level.iter().any(|n| note.starts_with(n)) {
                let top_level_ok =
                    note.starts_with("top-level") && !matches!(parsed, Ok(Data::Object(_)));
                assert!(
                    parsed.is_err() || top_level_ok,
                    "expected rejection for {note}"
                );
            }
        }
    }

    #[test]
    fn valid_fixtures_parse() {
        // Includes `123.0`, which JSON does not tell apart from `123`.
        for case in fixture("data-model-valid.json").as_array().unwrap() {
            let text = serde_json::to_string(&case["json"]).unwrap();
            let parsed = Data::from_json_str(&text);
            assert!(parsed.is_ok(), "{}: {parsed:?}", case["note"]);
        }
    }

    #[test]
    fn json_numbers_are_integers_or_nothing() {
        for (text, want) in [("123.0", 123), ("1e2", 100), ("-0.0", 0), ("-7", -7)] {
            assert_eq!(Data::from_json_str(text).unwrap(), Data::Integer(want));
        }
        for text in [
            "1.5",
            "1e-1",
            "1e400",
            "9007199254740992",
            "-9007199254740992",
        ] {
            assert!(Data::from_json_str(text).is_err(), "{text}");
        }
        assert_eq!(
            Data::from_json_str("9007199254740991").unwrap(),
            Data::Integer(MAX_SAFE_INTEGER)
        );
    }

    #[test]
    fn json_decode_limits() {
        let limit = |text: &str| match Data::from_json_str(text) {
            Err(JsonError::Limit(e)) => Some(e),
            _ => None,
        };
        // Depth: 32 containers pass, 33 do not.
        let nested = |n: usize| format!("{}0{}", "[".repeat(n), "]".repeat(n));
        assert!(Data::from_json_str(&nested(32)).is_ok());
        assert_eq!(limit(&nested(33)), Some(LimitError::TooDeep));
        // Container elements.
        let wide = |n: usize| format!("[{}]", vec!["0"; n].join(","));
        assert!(Data::from_json_str(&wide(131_072)).is_ok());
        assert_eq!(limit(&wide(131_073)), Some(LimitError::ContainerTooLong));
        // Object key.
        let keyed = |n: usize| format!("{{\"{}\":0}}", "k".repeat(n));
        assert!(Data::from_json_str(&keyed(8192)).is_ok());
        assert_eq!(limit(&keyed(8193)), Some(LimitError::KeyTooLong));
        // Size: 2 MiB of JSON.
        let big = format!("\"{}\"", "a".repeat(2 * 1024 * 1024));
        assert_eq!(limit(&big), Some(LimitError::TooLong));
        // A link whose CID is longer than 100 bytes: an identity hash of 120 bytes.
        let hash = cid::multihash::Multihash::<128>::wrap(0x00, &[7u8; 120]).unwrap();
        let long = cid::CidGeneric::<128>::new_v1(0x55, hash).to_string();
        assert!(Data::from_json_str(&format!("{{\"$link\":\"{long}\"}}")).is_err());
    }

    #[test]
    fn base64_is_decoded_loosely() {
        // Unused bits that are not zero, as the interop vector `full` has.
        assert_eq!(decode_base64("123").unwrap(), vec![0xd7, 0x6d]);
        assert_eq!(decode_base64("AQ==").unwrap(), vec![1]);
        assert_eq!(decode_base64("AQ").unwrap(), vec![1]);
        assert_eq!(decode_base64("").unwrap(), Vec::<u8>::new());
        for bad in ["A", "AQ===", "A-_=", "AQ I", "AQ=A"] {
            assert!(decode_base64(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn data_value_is_small() {
        assert_eq!(
            std::mem::size_of::<Data>(),
            32,
            "Data must stay 32 bytes; box large payloads"
        );
    }

    #[test]
    fn bytes_accepts_padding_on_input_and_omits_it_on_output() {
        let v: Data = serde_json::from_str(r#"{"$bytes":"AQID"}"#).unwrap();
        assert_eq!(v, Data::Bytes(Bytes(vec![1, 2, 3])));
        let v2: Data = serde_json::from_str(r#"{"$bytes":"AQ=="}"#).unwrap();
        assert_eq!(serde_json::to_string(&v2).unwrap(), r#"{"$bytes":"AQ"}"#);
    }
}
