//! The Merkle tree of a checkpoint (`docs/SPEC.md` §20.3), as RFC 6962
//! builds one.
//!
//! A leaf is the SHA-256 of a zero byte and the bytes of an entry. A node
//! is the SHA-256 of a one byte and its two children. A tree of `n`
//! leaves splits at the largest power of two below `n`.

use sha2::{Digest, Sha256};

/// The hash of a leaf.
pub fn leaf(entry: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update([0]);
    hash.update(entry);
    hash.finalize().into()
}

fn node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update([1]);
    hash.update(left);
    hash.update(right);
    hash.finalize().into()
}

/// The largest power of two that is less than `n`, for `n` of at least 2.
fn split(n: usize) -> usize {
    let mut k = 1;
    while k * 2 < n {
        k *= 2;
    }
    k
}

/// The root of the tree over the leaves. The root of no leaves is the
/// hash of nothing.
pub fn root(leaves: &[[u8; 32]]) -> [u8; 32] {
    match leaves {
        [] => Sha256::digest([]).into(),
        [only] => *only,
        _ => {
            let (left, right) = leaves.split_at(split(leaves.len()));
            node(&root(left), &root(right))
        }
    }
}

/// The proof that the leaf at `index` is in the tree: the hashes of the
/// subtrees beside its path, from the leaf up.
pub fn proof(leaves: &[[u8; 32]], index: usize) -> Vec<[u8; 32]> {
    if leaves.len() < 2 || index >= leaves.len() {
        return Vec::new();
    }
    let k = split(leaves.len());
    let (left, right) = leaves.split_at(k);
    if index < k {
        let mut path = proof(left, index);
        path.push(root(right));
        path
    } else {
        let mut path = proof(right, index - k);
        path.push(root(left));
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roots_are_those_of_rfc_6962() {
        // The test vectors of the Certificate Transparency reference
        // implementation: the tree over its first leaves.
        let inputs: [&[u8]; 3] = [&[], &[0x00], &[0x10]];
        let leaves: Vec<[u8; 32]> = inputs.iter().map(|input| leaf(input)).collect();
        let hex = |hash: [u8; 32]| {
            use std::fmt::Write;
            hash.iter().fold(String::new(), |mut out, byte| {
                let _ = write!(out, "{byte:02x}");
                out
            })
        };
        assert_eq!(
            hex(root(&[])),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(root(&leaves[..1])),
            "6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d"
        );
        assert_eq!(
            hex(root(&leaves[..2])),
            "fac54203e7cc696cf0dfcb42c92a1d9dbaf70ad9e621f4bd8d98662f00e3c125"
        );
        assert_eq!(
            hex(root(&leaves[..3])),
            "aeb6bcfe274b70a14fb067a5e5578264db0fa9b51af5e0ba159158f329e06e77"
        );
    }

    #[test]
    fn a_proof_has_one_hash_for_each_level() {
        let leaves: Vec<[u8; 32]> = (0_u8..7).map(|n| leaf(&[n])).collect();
        assert_eq!(proof(&leaves, 0).len(), 3);
        assert_eq!(proof(&leaves, 6).len(), 2);
        assert_eq!(proof(&leaves[..1], 0), Vec::<[u8; 32]>::new());
        assert_eq!(proof(&leaves, 7), Vec::<[u8; 32]>::new());
    }
}
