#![no_main]
//! A tree node block and a commit block. Whatever decodes is canonical: it
//! encodes back to the input.
use libfuzzer_sys::fuzz_target;
use pds_repo::Commit;
use pds_repo::mst::WireNode;

fuzz_target!(|data: &[u8]| {
    if let Ok(node) = WireNode::decode(data) {
        assert_eq!(node.encode(), data);
    }
    if let Ok(commit) = Commit::decode(data) {
        assert_eq!(commit.to_bytes(), data);
        let _ = commit.unsigned_bytes();
    }
});
