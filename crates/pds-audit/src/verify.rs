//! The verifier of an export (`docs/SPEC.md` §20.4).
//!
//! It recomputes the chain, the Merkle roots and the signatures from a
//! bundle, offline. It shares the entry format with the writer and
//! nothing else: the tree and the signed bytes are computed here a second
//! time, in another way, so that one mistake does not hide in both.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::signature::{ED25519, UnparsedPublicKey};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::entry::Entry;

/// What a verification found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// How many entries the bundle has.
    pub entries: usize,
    /// How many checkpoints the bundle has.
    pub checkpoints: usize,
    /// How many of them cover entries that are all in the bundle, so
    /// their root was recomputed.
    pub roots_recomputed: usize,
    /// How many inclusion proofs the bundle has.
    pub proofs: usize,
    /// What is wrong. An export that verifies has none.
    pub problems: Vec<String>,
}

impl Report {
    /// Whether nothing is wrong.
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

fn bytes_of(value: Option<&Value>) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(value?.as_str()?).ok()
}

fn hash32(value: Option<&Value>) -> Option<[u8; 32]> {
    bytes_of(value)?.try_into().ok()
}

/// The id of a key from the first eight bytes of its hash.
fn hex_id(head: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut id = String::from("ed25519:");
    for byte in head {
        id.push(char::from(DIGITS[usize::from(byte >> 4)]));
        id.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    id
}

fn leaf_hash(entry: &[u8]) -> [u8; 32] {
    let mut input = Vec::with_capacity(entry.len() + 1);
    input.push(0);
    input.extend_from_slice(entry);
    Sha256::digest(input).into()
}

fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut input = [0_u8; 65];
    input[0] = 1;
    input[1..33].copy_from_slice(left);
    input[33..].copy_from_slice(right);
    Sha256::digest(input).into()
}

/// The root of a tree, built from the leaves upward with a stack: each
/// leaf joins the subtrees of its own size that are complete.
fn tree_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    let mut stack: Vec<(usize, [u8; 32])> = Vec::new();
    for leaf in leaves {
        let mut current = (1_usize, *leaf);
        while let Some(top) = stack.last().copied() {
            if top.0 != current.0 {
                break;
            }
            stack.pop();
            current = (top.0 * 2, node_hash(&top.1, &current.1));
        }
        stack.push(current);
    }
    let Some((_, mut root)) = stack.pop() else {
        return Sha256::digest([]).into();
    };
    while let Some((_, left)) = stack.pop() {
        root = node_hash(&left, &root);
    }
    root
}

/// The root that an inclusion proof gives for a leaf at `index` of a tree
/// of `size` leaves, as RFC 9162 section 2.1.3.2 computes it. `None` when
/// the path has the wrong length.
fn root_of_proof(leaf: [u8; 32], index: u64, size: u64, path: &[[u8; 32]]) -> Option<[u8; 32]> {
    if index >= size {
        return None;
    }
    let (mut position, mut last) = (index, size - 1);
    let mut hash = leaf;
    for sibling in path {
        if last == 0 {
            return None;
        }
        if position % 2 == 1 || position == last {
            hash = node_hash(sibling, &hash);
            while position % 2 == 0 && position != 0 {
                position /= 2;
                last /= 2;
            }
        } else {
            hash = node_hash(&hash, sibling);
        }
        position /= 2;
        last /= 2;
    }
    (last == 0).then_some(hash)
}

/// CBOR of an unsigned integer with major type `major`.
fn cbor_head(major: u8, value: u64, out: &mut Vec<u8>) {
    let major = major << 5;
    match value {
        0..=23 => out.push(major | u8::try_from(value).unwrap_or(0)),
        24..=0xff => out.extend([major | 0x18, u8::try_from(value).unwrap_or(0)]),
        0x100..=0xffff => {
            out.push(major | 0x19);
            out.extend(u16::try_from(value).unwrap_or(0).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(major | 0x1a);
            out.extend(u32::try_from(value).unwrap_or(0).to_be_bytes());
        }
        _ => {
            out.push(major | 0x1b);
            out.extend(value.to_be_bytes());
        }
    }
}

/// The bytes that a checkpoint's signature covers, written here by hand:
/// an array of the domain, the range, the root, the head, the time and
/// the key id.
fn signed_bytes(checkpoint: &Parsed) -> Vec<u8> {
    let mut out = Vec::new();
    let text = |value: &str, out: &mut Vec<u8>| {
        cbor_head(3, value.len() as u64, out);
        out.extend_from_slice(value.as_bytes());
    };
    let bytes = |value: &[u8], out: &mut Vec<u8>| {
        cbor_head(2, value.len() as u64, out);
        out.extend_from_slice(value);
    };
    cbor_head(4, 7, &mut out);
    text("linkjar-pds-audit-checkpoint-v1", &mut out);
    cbor_head(0, checkpoint.first, &mut out);
    cbor_head(0, checkpoint.last, &mut out);
    bytes(&checkpoint.root, &mut out);
    bytes(&checkpoint.head, &mut out);
    text(&checkpoint.time, &mut out);
    text(&checkpoint.key_id, &mut out);
    out
}

struct Parsed {
    first: u64,
    last: u64,
    root: [u8; 32],
    head: [u8; 32],
    time: String,
    key_id: String,
    signature: Vec<u8>,
}

fn parse_checkpoint(value: &Value) -> Option<Parsed> {
    Some(Parsed {
        first: value.get("first")?.as_u64()?,
        last: value.get("last")?.as_u64()?,
        root: hash32(value.get("root"))?,
        head: hash32(value.get("head"))?,
        time: value.get("time")?.as_str()?.to_owned(),
        key_id: value.get("keyId")?.as_str()?.to_owned(),
        signature: bytes_of(value.get("signature"))?,
    })
}

/// Verifies an export bundle.
///
/// The bundle is a JSON object. `entries` are the bytes of consecutive
/// entries. `anchor` is the hash before the first of them. `keys` are
/// the public keys by id. `checkpoints` are the signed checkpoints, and
/// `proofs` say for an entry which checkpoint covers it and by which
/// path. Bytes are base64url without padding.
#[allow(clippy::too_many_lines)] // one pass over each part of the bundle
pub fn verify(bundle: &Value) -> Report {
    let mut report = Report::default();
    let list = |name: &str| {
        bundle
            .get(name)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };

    // The chain.
    let mut entries: Vec<(Entry, Vec<u8>)> = Vec::new();
    for (at, item) in list("entries").iter().enumerate() {
        match bytes_of(Some(item)).map(|bytes| (Entry::from_bytes(&bytes), bytes)) {
            Some((Ok(entry), bytes)) => entries.push((entry, bytes)),
            _ => report
                .problems
                .push(format!("entry {at} of the bundle does not decode")),
        }
    }
    report.entries = entries.len();
    let mut expected = hash32(bundle.get("anchor"));
    if expected.is_none() && !entries.is_empty() {
        report.problems.push("the bundle has no anchor".to_owned());
    }
    let mut previous: Option<u64> = None;
    for (entry, bytes) in &entries {
        if previous.is_some_and(|seq| entry.seq != seq + 1) {
            report.problems.push(format!(
                "entry {} does not follow the entry before it",
                entry.seq
            ));
        }
        if expected.is_some_and(|hash| hash != entry.prev) {
            report.problems.push(format!(
                "entry {} does not name the hash of the entry before it",
                entry.seq
            ));
        }
        expected = Some(Sha256::digest(bytes).into());
        previous = Some(entry.seq);
    }

    // The keys.
    let mut keys: Vec<(String, Vec<u8>)> = Vec::new();
    for key in list("keys") {
        match (
            key.get("keyId").and_then(Value::as_str),
            bytes_of(key.get("publicKey")),
        ) {
            (Some(id), Some(public)) => {
                // An id is the hash of its key, so a bundle cannot name a
                // key by the id of another.
                let digest = Sha256::digest(&public);
                if id == hex_id(&digest[..8]) {
                    keys.push((id.to_owned(), public));
                } else {
                    report
                        .problems
                        .push(format!("the key {id} does not have that id"));
                }
            }
            _ => report
                .problems
                .push("a key of the bundle does not decode".to_owned()),
        }
    }

    // The checkpoints.
    let mut checkpoints: Vec<Parsed> = Vec::new();
    for (at, item) in list("checkpoints").iter().enumerate() {
        match parse_checkpoint(item) {
            Some(checkpoint) => checkpoints.push(checkpoint),
            None => report
                .problems
                .push(format!("checkpoint {at} of the bundle does not decode")),
        }
    }
    report.checkpoints = checkpoints.len();
    let mut covered: Option<u64> = None;
    for checkpoint in &checkpoints {
        let name = format!(
            "the checkpoint of {} to {}",
            checkpoint.first, checkpoint.last
        );
        if checkpoint.first == 0 || checkpoint.last < checkpoint.first {
            report.problems.push(format!("{name} has no range"));
            continue;
        }
        if covered.is_some_and(|last| checkpoint.first != last + 1) {
            report
                .problems
                .push(format!("{name} does not follow the checkpoint before it"));
        }
        covered = Some(checkpoint.last);
        match keys.iter().find(|(id, _)| *id == checkpoint.key_id) {
            None => report
                .problems
                .push(format!("{name} names a key the bundle lacks")),
            Some((_, public)) => {
                let signed = signed_bytes(checkpoint);
                if UnparsedPublicKey::new(&ED25519, public)
                    .verify(&signed, &checkpoint.signature)
                    .is_err()
                {
                    report.problems.push(format!("{name} has a bad signature"));
                }
            }
        }
        // With every entry of the range in the bundle, the root and the
        // head are recomputed.
        let range: Vec<&(Entry, Vec<u8>)> = entries
            .iter()
            .filter(|(entry, _)| (checkpoint.first..=checkpoint.last).contains(&entry.seq))
            .collect();
        let size = usize::try_from(checkpoint.last - checkpoint.first + 1).unwrap_or(usize::MAX);
        if range.len() == size {
            report.roots_recomputed += 1;
            let leaves: Vec<[u8; 32]> = range.iter().map(|(_, bytes)| leaf_hash(bytes)).collect();
            if tree_root(&leaves) != checkpoint.root {
                report
                    .problems
                    .push(format!("{name} has another root than its entries"));
            }
            let head: Option<[u8; 32]> =
                range.last().map(|(_, bytes)| Sha256::digest(bytes).into());
            if head != Some(checkpoint.head) {
                report
                    .problems
                    .push(format!("{name} has another head than its last entry"));
            }
        }
    }

    // The inclusion proofs.
    for proof in list("proofs") {
        report.proofs += 1;
        let seq = proof.get("seq").and_then(Value::as_u64);
        let covering = proof
            .get("checkpoint")
            .and_then(Value::as_u64)
            .and_then(|last| {
                checkpoints
                    .iter()
                    .find(|checkpoint| checkpoint.last == last)
            });
        let path: Option<Vec<[u8; 32]>> = proof
            .get("path")
            .and_then(Value::as_array)
            .map(|path| path.iter().filter_map(|hash| hash32(Some(hash))).collect());
        let entry = seq.and_then(|seq| entries.iter().find(|(entry, _)| entry.seq == seq));
        let (Some(seq), Some(checkpoint), Some(path), Some((_, bytes))) =
            (seq, covering, path, entry)
        else {
            report
                .problems
                .push("a proof of the bundle does not decode".to_owned());
            continue;
        };
        let inside = (checkpoint.first..=checkpoint.last).contains(&seq);
        let root = inside
            .then(|| {
                root_of_proof(
                    leaf_hash(bytes),
                    seq - checkpoint.first,
                    checkpoint.last - checkpoint.first + 1,
                    &path,
                )
            })
            .flatten();
        if root != Some(checkpoint.root) {
            report.problems.push(format!(
                "the proof of entry {seq} does not lead to its checkpoint"
            ));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::checkpoint::Signer;
    use crate::entry::{Draft, GENESIS};
    use crate::merkle;

    fn encode(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    /// A log of `count` entries with a checkpoint for each `per` entries,
    /// and a proof for every entry.
    fn bundle(count: u64, per: usize) -> Value {
        let signer = Signer::from_seed(&[9; 32]).unwrap();
        let mut entries: Vec<Entry> = Vec::new();
        for n in 0..count {
            let what = Draft {
                actor: "admin".to_owned(),
                action: format!("action-{n}"),
                outcome: "ok".to_owned(),
                ..Draft::default()
            };
            let entry = Entry::after(entries.last(), "2026-10-13T00:00:00.000Z", what);
            entries.push(entry);
        }
        let all: Vec<Vec<u8>> = entries.iter().map(Entry::to_bytes).collect();
        let mut checkpoints = Vec::new();
        let mut proofs = Vec::new();
        for (index, chunk) in all.chunks(per).enumerate() {
            let first = (index * per) as u64 + 1;
            let checkpoint = signer
                .checkpoint(first, chunk, "2026-10-13T00:05:00.000Z")
                .unwrap();
            let leaves: Vec<[u8; 32]> = chunk.iter().map(|bytes| merkle::leaf(bytes)).collect();
            for at in 0..chunk.len() {
                let path: Vec<String> = merkle::proof(&leaves, at)
                    .iter()
                    .map(|h| encode(h))
                    .collect();
                proofs.push(json!({ "seq": first + at as u64, "checkpoint": checkpoint.last, "path": path }));
            }
            checkpoints.push(json!({
                "first": checkpoint.first,
                "last": checkpoint.last,
                "root": encode(&checkpoint.root),
                "head": encode(&checkpoint.head),
                "time": checkpoint.time,
                "keyId": checkpoint.key_id,
                "signature": encode(&checkpoint.signature),
            }));
        }
        json!({
            "version": 1,
            "anchor": encode(&GENESIS),
            "keys": [{ "keyId": signer.key_id(), "publicKey": encode(&signer.public_key()) }],
            "entries": all.iter().map(|bytes| encode(bytes)).collect::<Vec<_>>(),
            "checkpoints": checkpoints,
            "proofs": proofs,
        })
    }

    #[test]
    fn an_export_verifies_for_every_size_of_tree() {
        for (count, per) in [(1, 1), (2, 2), (7, 7), (13, 5), (64, 64), (33, 8)] {
            let report = verify(&bundle(count, per));
            assert!(report.ok(), "{count} by {per}: {:?}", report.problems);
            assert_eq!(report.entries as u64, count);
            assert_eq!(report.roots_recomputed, report.checkpoints);
            assert_eq!(report.proofs as u64, count);
        }
        // The two implementations of the tree agree.
        let leaves: Vec<[u8; 32]> = (0_u8..37).map(|n| merkle::leaf(&[n])).collect();
        for size in 0..leaves.len() {
            assert_eq!(
                tree_root(&leaves[..size]),
                merkle::root(&leaves[..size]),
                "{size}"
            );
        }
    }

    #[test]
    fn each_kind_of_tampering_is_found() {
        let good = bundle(13, 5);
        let found = |change: &dyn Fn(&mut Value), what: &str| {
            let mut changed = good.clone();
            change(&mut changed);
            let report = verify(&changed);
            assert!(
                report.problems.iter().any(|problem| problem.contains(what)),
                "{what}: {:?}",
                report.problems
            );
        };
        // An entry is rewritten: the chain breaks after it and its checkpoint has another root.
        let rewritten = {
            let bytes = URL_SAFE_NO_PAD
                .decode(good["entries"][3].as_str().unwrap())
                .unwrap();
            let mut entry = Entry::from_bytes(&bytes).unwrap();
            entry.what.outcome = "refused".to_owned();
            encode(&entry.to_bytes())
        };
        found(
            &|b| b["entries"][3] = json!(rewritten),
            "does not name the hash",
        );
        found(&|b| b["entries"][3] = json!(rewritten), "another root");
        // An entry is removed.
        found(
            &|b| {
                b["entries"].as_array_mut().unwrap().remove(6);
            },
            "does not follow the entry before it",
        );
        // A checkpoint is changed, removed, or signed by another key.
        found(
            &|b| b["checkpoints"][1]["last"] = json!(11),
            "bad signature",
        );
        found(
            &|b| b["checkpoints"][0]["time"] = json!("2020-01-01T00:00:00.000Z"),
            "bad signature",
        );
        found(
            &|b| {
                b["checkpoints"].as_array_mut().unwrap().remove(1);
            },
            "does not follow the checkpoint before it",
        );
        let other = Signer::from_seed(&[1; 32]).unwrap();
        found(
            &|b| b["keys"][0]["publicKey"] = json!(encode(&other.public_key())),
            "does not have that id",
        );
        found(
            &|b| {
                b["keys"] =
                    json!([{ "keyId": other.key_id(), "publicKey": encode(&other.public_key()) }]);
            },
            "names a key the bundle lacks",
        );
        // A proof for another entry, and a first entry that does not follow the anchor.
        found(
            &|b| b["proofs"][2]["seq"] = json!(2),
            "does not lead to its checkpoint",
        );
        found(
            &|b| b["anchor"] = json!(encode(&[1; 32])),
            "does not name the hash",
        );
        found(&|b| b["entries"][0] = json!("AAAA"), "does not decode");
    }
}
