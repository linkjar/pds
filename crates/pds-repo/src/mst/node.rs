//! The wire form of a tree node and the rules for keys.
//!
//! A node is the DAG-CBOR map `{"e": [entry, ...], "l": link or null}`. An
//! entry is `{"k": bytes, "p": int, "t": link or null, "v": link}`: `p` is
//! the number of bytes the key shares with the previous key in the node and
//! `k` the rest of it; `v` is the record and `t` the subtree to the right.
//! `l` is the subtree to the left of the first entry.

use cid::Cid;
use pds_types::cbor::{CborError, Limits, Reader, Writer};

use crate::error::NodeError;

/// The longest key in bytes, as in the Reference.
pub const MAX_KEY_LEN: usize = 1024;

/// The highest layer a key can have: a SHA-256 digest of zeros counts 128
/// pairs of bits. A tree is therefore at most 129 nodes deep.
pub const MAX_LAYER: u32 = 128;

/// The layer of a key: the number of leading zero bit pairs in its SHA-256
/// digest. Each layer keeps a quarter of the keys of the layer below.
pub fn layer_of(key: &[u8]) -> u32 {
    let digest = pds_types::content::sha256(key);
    let mut layer = 0;
    for byte in digest {
        if byte == 0 {
            layer += 4;
            continue;
        }
        // The byte is not zero, so it has at most seven leading zero bits.
        layer += byte.leading_zeros() / 2;
        break;
    }
    layer
}

/// Whether `key` is a repository path: `<collection>/<rkey>`, each part
/// non-empty and of `A-Za-z0-9_~-:.`, at most 1024 bytes in all.
pub fn is_valid_key(key: &str) -> bool {
    let part = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_~-:.".contains(&b))
    };
    key.len() <= MAX_KEY_LEN
        && key
            .split_once('/')
            .is_some_and(|(collection, rkey)| part(collection) && part(rkey))
}

/// The number of leading bytes two keys share.
pub fn common_prefix_len(a: &str, b: &str) -> usize {
    a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count()
}

/// One entry of a decoded node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireEntry {
    /// The full key.
    pub key: String,
    /// The record.
    pub value: Cid,
    /// The subtree between this key and the next.
    pub tree: Option<Cid>,
}

/// A decoded node.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireNode {
    /// The subtree before the first key.
    pub left: Option<Cid>,
    /// The entries in key order.
    pub entries: Vec<WireEntry>,
}

fn expect_key(r: &mut Reader<'_>, want: &str) -> Result<(), NodeError> {
    if r.key()? == want {
        Ok(())
    } else {
        Err(NodeError::Field)
    }
}

fn link_or_null(r: &mut Reader<'_>) -> Result<Option<Cid>, CborError> {
    if r.peek_null() {
        r.null()?;
        Ok(None)
    } else {
        r.link().map(Some)
    }
}

impl WireNode {
    /// Decodes a node block.
    ///
    /// The block must be the canonical encoding: the fields in order, every
    /// key valid and greater than the one before, and every prefix length
    /// the longest common prefix. A block that passes encodes back to the
    /// same bytes. The decode limits of SPEC §6.1 bound its size and its
    /// number of entries.
    pub fn decode(bytes: &[u8]) -> Result<Self, NodeError> {
        let mut r = Reader::new(bytes, Limits::RECORD)?;
        if r.map()? != 2 {
            return Err(NodeError::Field);
        }
        expect_key(&mut r, "e")?;
        let count = r.array()?;
        // An entry takes more than one byte, so the count cannot reserve
        // more than the input holds.
        let mut entries: Vec<WireEntry> = Vec::with_capacity(count.min(r.remaining().len()));
        for _ in 0..count {
            if r.map()? != 4 {
                return Err(NodeError::Field);
            }
            expect_key(&mut r, "k")?;
            let suffix = r.bytes()?;
            expect_key(&mut r, "p")?;
            let prefix_len = usize::try_from(r.integer()?).map_err(|_| NodeError::Prefix)?;
            expect_key(&mut r, "t")?;
            let tree = link_or_null(&mut r)?;
            expect_key(&mut r, "v")?;
            let value = r.link()?;

            let prev = entries.last().map_or("", |e| e.key.as_str());
            let head = prev.get(..prefix_len).ok_or(NodeError::Prefix)?;
            let tail = std::str::from_utf8(suffix).map_err(|_| NodeError::Key)?;
            let mut key = String::with_capacity(head.len() + tail.len());
            key.push_str(head);
            key.push_str(tail);
            if !is_valid_key(&key) {
                return Err(NodeError::Key);
            }
            if !entries.is_empty() && prev >= key.as_str() {
                return Err(NodeError::Order);
            }
            if common_prefix_len(prev, &key) != prefix_len {
                return Err(NodeError::Prefix);
            }
            entries.push(WireEntry { key, value, tree });
        }
        expect_key(&mut r, "l")?;
        let left = link_or_null(&mut r)?;
        r.finish()?;
        Ok(Self { left, entries })
    }

    /// Encodes the node.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = NodeWriter::new(self.left.as_ref(), self.entries.len());
        for e in &self.entries {
            w.entry(&e.key, &e.value, e.tree.as_ref());
        }
        w.finish()
    }
}

/// Writes a node block entry by entry, without building a [`WireNode`].
pub(crate) struct NodeWriter<'a> {
    w: Writer,
    left: Option<&'a Cid>,
    prev: String,
}

impl<'a> NodeWriter<'a> {
    pub(crate) fn new(left: Option<&'a Cid>, entries: usize) -> Self {
        let mut w = Writer::with_capacity(16 + entries * 96);
        w.map(2);
        w.text("e");
        w.array(entries);
        Self {
            w,
            left,
            prev: String::new(),
        }
    }

    /// Adds the next entry. Keys must come in ascending order.
    pub(crate) fn entry(&mut self, key: &str, value: &Cid, tree: Option<&Cid>) {
        let shared = common_prefix_len(&self.prev, key);
        self.w.map(4);
        self.w.text("k");
        self.w
            .bytes(key.as_bytes().get(shared..).unwrap_or_default());
        self.w.text("p");
        self.w.integer(i64::try_from(shared).unwrap_or(i64::MAX));
        self.w.text("t");
        match tree {
            Some(cid) => self.w.link(cid),
            None => self.w.null(),
        }
        self.w.text("v");
        self.w.link(value);
        self.prev.clear();
        self.prev.push_str(key);
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.w.text("l");
        match self.left {
            Some(cid) => self.w.link(cid),
            None => self.w.null(),
        }
        self.w.into_vec()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pds_types::content::cid_for_cbor;
    use serde_json::Value;

    use super::*;

    fn interop(name: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../interop/mst")
            .join(name);
        std::fs::read_to_string(path).expect("interop file")
    }

    #[test]
    fn key_heights_vector() {
        let cases: Vec<Value> = serde_json::from_str(&interop("key_heights.json")).unwrap();
        assert_eq!(cases.len(), 9);
        for case in cases {
            let key = case["key"].as_str().unwrap();
            assert_eq!(
                u64::from(layer_of(key.as_bytes())),
                case["height"].as_u64().unwrap(),
                "{key:?}"
            );
        }
    }

    #[test]
    fn common_prefix_vector() {
        let cases: Vec<Value> = serde_json::from_str(&interop("common_prefix.json")).unwrap();
        assert_eq!(cases.len(), 13);
        for case in cases {
            let (left, right) = (
                case["left"].as_str().unwrap(),
                case["right"].as_str().unwrap(),
            );
            // Two cases end in a NUL byte: the count is of bytes, as in
            // the node format.
            assert_eq!(
                common_prefix_len(left, right) as u64,
                case["len"].as_u64().unwrap(),
                "{left:?} {right:?}"
            );
        }
    }

    #[test]
    fn example_keys_are_at_the_layer_their_name_says() {
        // `A0/374913` is a key that upstream generated for layer 0.
        let text = interop("example_keys.txt");
        let mut n = 0;
        for key in text.lines().filter(|l| !l.is_empty()) {
            let digit = key.as_bytes()[1] - b'0';
            assert_eq!(layer_of(key.as_bytes()), u32::from(digit), "{key}");
            assert!(is_valid_key(key));
            n += 1;
        }
        assert_eq!(n, 156);
    }

    #[test]
    fn key_rules() {
        for ok in [
            "com.example.record/3jqfcqzm3fo2j",
            "a/b",
            "app.bsky.feed.post/self",
            "c/a:b~c_d-e.f",
        ] {
            assert!(is_valid_key(ok), "{ok}");
        }
        let long = format!("c/{}", "a".repeat(MAX_KEY_LEN - 1));
        assert!(!is_valid_key(&long));
        assert!(is_valid_key(&long[..MAX_KEY_LEN]));
        for bad in [
            "", "/", "a/", "/b", "a", "a/b/c", "a b/c", "a/b c", "a/é", "a/b#c", "a/b%20",
        ] {
            assert!(!is_valid_key(bad), "{bad:?}");
        }
    }

    fn sample() -> WireNode {
        let v = cid_for_cbor(b"value");
        let t = cid_for_cbor(b"tree");
        WireNode {
            left: Some(t),
            entries: vec![
                WireEntry {
                    key: "com.example.record/3jqfcqzm3fo2j".into(),
                    value: v,
                    tree: None,
                },
                WireEntry {
                    key: "com.example.record/3jqfcqzm3fp2j".into(),
                    value: v,
                    tree: Some(t),
                },
            ],
        }
    }

    #[test]
    fn node_round_trip_and_known_empty_root() {
        let node = sample();
        let bytes = node.encode();
        assert_eq!(WireNode::decode(&bytes).unwrap(), node);
        // The empty tree: `{"e": [], "l": null}`.
        let empty = WireNode::default().encode();
        assert_eq!(empty, [0xa2, 0x61, b'e', 0x80, 0x61, b'l', 0xf6]);
        assert_eq!(
            cid_for_cbor(&empty).to_string(),
            "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm"
        );
        assert_eq!(WireNode::decode(&empty).unwrap(), WireNode::default());
    }

    /// Entries as a key suffix and a prefix length.
    type Entries<'a> = &'a [(&'a [u8], i64)];

    /// Builds a node block by hand so that single rules can be broken.
    fn raw_node(entries: &[(&[u8], i64)], extra_field: bool) -> Vec<u8> {
        let v = cid_for_cbor(b"value");
        let mut w = Writer::new();
        w.map(if extra_field { 3 } else { 2 });
        w.text("e");
        w.array(entries.len());
        for (suffix, prefix) in entries {
            w.map(4);
            w.text("k");
            w.bytes(suffix);
            w.text("p");
            w.integer(*prefix);
            w.text("t");
            w.null();
            w.text("v");
            w.link(&v);
        }
        w.text("l");
        w.null();
        if extra_field {
            w.text("x");
            w.null();
        }
        w.into_vec()
    }

    #[test]
    fn decode_rejects_what_is_not_canonical() {
        assert!(WireNode::decode(&raw_node(&[(b"c/a", 0), (b"b", 2)], false)).is_ok());
        let cases: &[(Entries<'_>, NodeError)] = &[
            (&[(b"c/b", 0), (b"a", 2)], NodeError::Order),
            (&[(b"c/a", 0), (b"a", 2)], NodeError::Order),
            // The prefix is shorter than the keys share.
            (&[(b"c/a", 0), (b"c/b", 0)], NodeError::Prefix),
            // The prefix is longer than the previous key.
            (&[(b"c/a", 0), (b"b", 9)], NodeError::Prefix),
            (&[(b"c/a", 1)], NodeError::Prefix),
            (&[(b"c/a", -1)], NodeError::Prefix),
            (&[(b"nokeyslash", 0)], NodeError::Key),
            (&[(b"c/\xff", 0)], NodeError::Key),
        ];
        for (entries, want) in cases {
            assert_eq!(
                WireNode::decode(&raw_node(entries, false)),
                Err(*want),
                "{entries:?}"
            );
        }
        assert_eq!(
            WireNode::decode(&raw_node(&[], true)),
            Err(NodeError::Field)
        );
        // A record is not a node.
        assert!(WireNode::decode(&[0xa0]).is_err());
        assert!(WireNode::decode(&[]).is_err());
        // Bytes after the node.
        let mut trailing = WireNode::default().encode();
        trailing.push(0);
        assert!(WireNode::decode(&trailing).is_err());
    }
}
