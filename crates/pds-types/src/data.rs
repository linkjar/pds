//! The atproto data model (SPEC §6.1, §6.3).
//!
//! One value type with two wire forms. JSON uses `{"$link": "<cid>"}` for
//! links and `{"$bytes": "<base64>"}` for bytes; DAG-CBOR uses CID tag 42 and
//! byte strings. The serializer's `is_human_readable` flag picks the form,
//! so `serde_json` and `serde_ipld_dagcbor` both work on the same types.
//! Floats are not part of the model and are rejected on input.

use std::collections::BTreeMap;
use std::fmt;

use data_encoding::BASE64_NOPAD;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
            decode_base64(&r.bytes).map(Bytes).map_err(de::Error::custom)
        } else {
            struct V;
            impl<'de> Visitor<'de> for V {
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

/// Decodes standard base64 with or without padding, as the reference accepts.
pub fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    let trimmed = s.trim_end_matches('=');
    BASE64_NOPAD.decode(trimmed.as_bytes()).map_err(|e| format!("invalid base64: {e}"))
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
            let c = cid::Cid::try_from(r.link.as_str()).map_err(de::Error::custom)?;
            Ok(CidLink(c))
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
        d.deserialize_bytes(RawBytes).and_then(|b| cid_from_tagged_bytes(&b).map_err(de::Error::custom))
    }
    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<CidLink, E> {
        cid_from_tagged_bytes(v).map_err(de::Error::custom)
    }
    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<CidLink, E> {
        cid_from_tagged_bytes(&v).map_err(de::Error::custom)
    }
}

struct RawBytes;

impl<'de> Visitor<'de> for RawBytes {
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
        if s == "blob" { Ok(BlobType) } else { Err(de::Error::custom("expected $type \"blob\"")) }
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

    /// Parses a JSON document into the data model, rejecting floats.
    pub fn from_json_str(s: &str) -> Result<Data, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Encodes as canonical DAG-CBOR.
    pub fn to_dag_cbor(&self) -> Result<Vec<u8>, serde_ipld_dagcbor::EncodeError<std::collections::TryReserveError>> {
        serde_ipld_dagcbor::to_vec(self)
    }

    /// Decodes DAG-CBOR.
    pub fn from_dag_cbor(bytes: &[u8]) -> Result<Data, serde_ipld_dagcbor::DecodeError<std::convert::Infallible>> {
        serde_ipld_dagcbor::from_slice(bytes)
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
        i64::try_from(v).map(Data::Integer).map_err(|_| de::Error::custom("integer out of range"))
    }
    fn visit_i128<E: de::Error>(self, v: i128) -> Result<Data, E> {
        i64::try_from(v).map(Data::Integer).map_err(|_| de::Error::custom("integer out of range"))
    }
    fn visit_u128<E: de::Error>(self, v: u128) -> Result<Data, E> {
        i64::try_from(v).map(Data::Integer).map_err(|_| de::Error::custom("integer out of range"))
    }
    fn visit_f64<E: de::Error>(self, _v: f64) -> Result<Data, E> {
        Err(de::Error::custom("floats are not allowed in the atproto data model"))
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
        LinkVisitor.visit_newtype_struct(d).map(|l| Data::Link(Box::new(l)))
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
                    let c = cid::Cid::try_from(s.as_str()).map_err(de::Error::custom)?;
                    return Ok(Data::Link(Box::new(CidLink(c))));
                }
                if let Some(Data::String(s)) = out.get("$bytes") {
                    let b = decode_base64(s).map_err(de::Error::custom)?;
                    return Ok(Data::Bytes(Bytes(b)));
                }
            }
            if out.contains_key("$link") || out.contains_key("$bytes") {
                return Err(de::Error::custom("$link and $bytes objects may have no other keys"));
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
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../interop/data-model").join(name);
        serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).expect("json")
    }

    #[test]
    fn fixtures_roundtrip_json_and_cbor() {
        let cases = fixture("data-model-fixtures.json");
        for case in cases.as_array().expect("array") {
            let json_text = serde_json::to_string(&case["json"]).unwrap();
            let data = Data::from_json_str(&json_text).expect("json parses into the data model");
            let cbor = data.to_dag_cbor().expect("encodes");
            let expected = decode_base64(case["cbor_base64"].as_str().unwrap()).unwrap();
            assert_eq!(cbor, expected, "cbor bytes differ for {json_text}");
            let back = Data::from_dag_cbor(&cbor).expect("decodes");
            assert_eq!(back, data);
            let json_again: serde_json::Value = serde_json::from_str(&serde_json::to_string(&back).unwrap()).unwrap();
            assert_eq!(json_again, case["json"]);
            let hash = cid::multihash::Multihash::<64>::wrap(0x12, &sha256(&cbor)).unwrap();
            let c = cid::Cid::new_v1(0x71, hash);
            assert_eq!(c.to_string(), case["cid"].as_str().unwrap());
        }
    }

    fn sha256(b: &[u8]) -> [u8; 32] {
        // Avoid a crypto dependency in this crate's tests: use the cid
        // crate's bundled sha2 through multihash's Code when available.
        use std::io::Write;
        let mut h = Sha256Lite::new();
        h.write_all(b).unwrap();
        h.finish()
    }

    // Minimal SHA-256 for the fixture check (the server uses RustCrypto).
    struct Sha256Lite {
        state: [u32; 8],
        buf: Vec<u8>,
        len: u64,
    }
    impl Sha256Lite {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
            0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
            0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
            0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
            0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
            0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
            0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
        ];
        fn new() -> Self {
            Self {
                state: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
                buf: Vec::new(),
                len: 0,
            }
        }
        fn block(&mut self, chunk: &[u8]) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
            }
            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(Self::K[i]).wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                h = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
                *s = s.wrapping_add(v);
            }
        }
        fn finish(mut self) -> [u8; 32] {
            let bits = self.len * 8;
            self.buf.push(0x80);
            while self.buf.len() % 64 != 56 {
                self.buf.push(0);
            }
            self.buf.extend_from_slice(&bits.to_be_bytes());
            let data = std::mem::take(&mut self.buf);
            for chunk in data.chunks(64) {
                self.block(chunk);
            }
            let mut out = [0u8; 32];
            for (i, s) in self.state.iter().enumerate() {
                out[4 * i..4 * i + 4].copy_from_slice(&s.to_be_bytes());
            }
            out
        }
    }
    impl std::io::Write for Sha256Lite {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.len += b.len() as u64;
            self.buf.extend_from_slice(b);
            while self.buf.len() >= 64 {
                let chunk: Vec<u8> = self.buf.drain(..64).collect();
                self.block(&chunk);
            }
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
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
                let top_level_ok = note.starts_with("top-level") && !matches!(parsed, Ok(Data::Object(_)));
                assert!(parsed.is_err() || top_level_ok, "expected rejection for {note}");
            }
        }
    }

    #[test]
    fn data_value_is_small() {
        assert_eq!(std::mem::size_of::<Data>(), 32, "Data must stay 32 bytes; box large payloads");
    }

    #[test]
    fn bytes_accepts_padding_on_input_and_omits_it_on_output() {
        let v: Data = serde_json::from_str(r#"{"$bytes":"AQID"}"#).unwrap();
        assert_eq!(v, Data::Bytes(Bytes(vec![1, 2, 3])));
        let v2: Data = serde_json::from_str(r#"{"$bytes":"AQ=="}"#).unwrap();
        assert_eq!(serde_json::to_string(&v2).unwrap(), r#"{"$bytes":"AQ"}"#);
    }
}
