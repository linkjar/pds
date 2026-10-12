#![no_main]
//! A CAR from an untrusted source, read as a repository import reads it.
//!
//! Three properties: nothing panics; a traversal never enters more nodes
//! than the CAR has blocks, whatever shape the blocks describe; and a tree
//! that the reader accepts is the canonical tree of its keys.
use libfuzzer_sys::fuzz_target;
use pds_repo::mst::{Guard, Tree, export_blocks, read_tree};
use pds_repo::{Commit, Expect, MemoryBlockStore, read_car, verify_repo_car};

fuzz_target!(|data: &[u8]| {
    let Ok(car) = read_car(data) else { return };
    let blocks = car.blocks.len();
    let anyone = Expect {
        did: None,
        verify: None,
    };
    if let Ok(repo) = verify_repo_car(data, &anyone, false) {
        assert!(repo.nodes.len() <= blocks);
    }
    let Ok(root) = car.root() else { return };
    let Some(Ok(commit)) = car.blocks.get(&root).map(Commit::decode) else {
        return;
    };

    let mut guard = Guard::new(blocks);
    let read = read_tree(&car.blocks, &commit.data, &mut guard);
    assert!(guard.entered() <= blocks, "the walk entered more nodes than the CAR holds");
    if let Ok(contents) = read {
        let mut rebuilt = Tree::empty(MemoryBlockStore::new());
        for (key, value) in &contents.leaves {
            rebuilt.add(key, *value).expect("an accepted key is valid and new");
        }
        assert_eq!(
            rebuilt.root().expect("encodes"),
            commit.data,
            "an accepted tree is not the canonical tree of its keys"
        );
    }

    let mut guard = Guard::new(blocks);
    let mut copied = 0usize;
    let _ = export_blocks(&car.blocks, &commit.data, &mut guard, |_, _| {
        copied += 1;
        Ok(())
    });
    assert!(guard.entered() <= blocks);
    // Each node once, then each record once. A block that is a node and
    // also named as a record is copied in both roles.
    assert!(copied <= 2 * blocks, "the export copied more than the CAR holds");
});
