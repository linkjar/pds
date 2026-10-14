//! The engine against the Reference's own library.
//!
//! `tests/fixtures/reference.json` and the CAR files beside it were written
//! by `@atproto/repo` at the pin (`parity/src/tools/repo-fixtures.ts`), the
//! library that the repository CID check of the parity harness uses. Each test replays
//! what the library did and requires the same bytes: tree roots, commit
//! CIDs, the blocks of every commit, and the export.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use std::collections::BTreeSet;
use std::path::PathBuf;

use cid::Cid;
use data_encoding::HEXLOWER;
use k256::ecdsa::signature::{Signer, Verifier};
use k256::ecdsa::{Signature, SigningKey};
use pds_repo::mst::{Guard, Tree, export_blocks, read_tree};
use pds_repo::{
    BlockMap, CommitData, Expect, MemoryBlockStore, Repo, RepoError, SignError, TreeDefect, Write,
    read_car, verify_diff_car, verify_repo_car, write_car,
};
use pds_types::content::{cid_for_cbor, sha256};
use pds_types::{Data, Did, Tid};
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn reference() -> Value {
    let text = std::fs::read_to_string(fixtures().join("reference.json")).expect("fixture");
    serde_json::from_str(&text).expect("json")
}

fn cid(v: &Value) -> Cid {
    Cid::try_from(v.as_str().expect("a CID string")).expect("a CID")
}

fn cid_set(v: &Value) -> BTreeSet<String> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|c| c.as_str().expect("string").to_owned())
        .collect()
}

fn key_of(reference: &Value) -> SigningKey {
    let bytes = HEXLOWER
        .decode(reference["privateKeyHex"].as_str().unwrap().as_bytes())
        .unwrap();
    SigningKey::from_slice(&bytes).unwrap()
}

fn signer(key: &SigningKey) -> impl Fn(&[u8]) -> Result<Vec<u8>, SignError> + '_ {
    |message| {
        let sig: Signature = key.sign(message);
        Ok(sig.to_bytes().to_vec())
    }
}

#[test]
fn secp256k1_signatures_are_the_reference_bytes() {
    // Commit CIDs can only match if signing is the same function: SHA-256,
    // an RFC 6979 nonce and a low s.
    let reference = reference();
    let key = key_of(&reference);
    for case in reference["signatures"].as_array().unwrap() {
        let message = HEXLOWER
            .decode(case["messageHex"].as_str().unwrap().as_bytes())
            .unwrap();
        let sig: Signature = key.sign(&message);
        assert_eq!(
            HEXLOWER.encode(&sig.to_bytes()),
            case["secp256k1"].as_str().unwrap()
        );
    }
}

#[test]
fn tree_roots() {
    for tree in reference()["trees"].as_array().unwrap() {
        let name = tree["name"].as_str().unwrap();
        let mut built = Tree::empty(MemoryBlockStore::new());
        for entry in tree["entries"].as_array().unwrap() {
            built
                .add(entry[0].as_str().unwrap(), cid(&entry[1]))
                .unwrap();
        }
        let changes = built.changes().unwrap();
        assert_eq!(changes.root, cid(&tree["root"]), "{name}");
        assert_eq!(
            changes.new_blocks.len() as u64,
            tree["nodes"].as_u64().unwrap(),
            "{name}"
        );
        // The tree it wrote passes the import checks.
        let contents = read_tree(
            &changes.new_blocks,
            &changes.root,
            &mut Guard::new(changes.new_blocks.len()),
        )
        .unwrap();
        assert_eq!(
            contents.leaves.len(),
            tree["entries"].as_array().unwrap().len(),
            "{name}"
        );
    }
}

/// A commit's writes with the record blocks encoded from the fixture JSON.
fn encoded_writes(commit: &Value) -> Vec<(String, String, Option<Vec<u8>>)> {
    commit["writes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            let action = w["action"].as_str().unwrap().to_owned();
            let key = w["key"].as_str().unwrap().to_owned();
            let record = w.get("record").map(|r| {
                let data =
                    Data::from_json_str(&serde_json::to_string(r).unwrap()).expect("a record");
                let bytes = data.to_dag_cbor();
                // The encoder agrees with the Reference's on every record.
                assert_eq!(cid_for_cbor(&bytes), cid(&w["cid"]), "{key}");
                bytes
            });
            (action, key, record)
        })
        .collect()
}

fn check_commit(name: &str, got: &CommitData, want: &Value) {
    assert_eq!(got.data, cid(&want["data"]), "{name}: tree root");
    assert_eq!(got.cid, cid(&want["cid"]), "{name}: commit CID");
    let cids = |blocks: &BlockMap| {
        blocks
            .cids()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        cids(&got.new_blocks),
        cid_set(&want["newBlocks"]),
        "{name}: new blocks"
    );
    assert_eq!(
        got.removed_cids
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>(),
        cid_set(&want["removedCids"]),
        "{name}: removed blocks"
    );
    assert_eq!(
        cids(&got.relevant_blocks),
        cid_set(&want["relevantBlocks"]),
        "{name}: event blocks"
    );
    // The event's CAR is the Reference's byte for byte, block order included.
    let car = write_car(Some(&got.cid), &got.relevant_blocks);
    assert_eq!(
        car.len() as u64,
        want["relevantCarLength"].as_u64().unwrap(),
        "{name}: CAR length"
    );
    assert_eq!(
        HEXLOWER.encode(&sha256(&car)),
        want["relevantCarSha256"].as_str().unwrap(),
        "{name}: CAR bytes"
    );
    assert_eq!(
        got.since.as_ref().map(Tid::as_str),
        want["since"].as_str(),
        "{name}: since"
    );
    assert_eq!(
        got.prev.map(|c| c.to_string()).as_deref(),
        want["prev"].as_str(),
        "{name}: prev"
    );
}

/// Replays every commit of the fixture and returns the store and the last
/// commit CID.
fn replay(reference: &Value) -> (MemoryBlockStore, Cid) {
    let key = key_of(reference);
    let did = Did::parse(reference["did"].as_str().unwrap()).unwrap();
    let mut store = MemoryBlockStore::new();
    let mut head: Option<Cid> = None;
    for commit in reference["commits"].as_array().unwrap() {
        let name = commit["name"].as_str().unwrap();
        let rev = Tid::parse(commit["rev"].as_str().unwrap()).unwrap();
        let encoded = encoded_writes(commit);
        let writes: Vec<Write<'_>> = encoded
            .iter()
            .map(|(action, key, record)| match (action.as_str(), record) {
                ("create", Some(record)) => Write::Create { key, record },
                ("update", Some(record)) => Write::Update { key, record },
                ("delete", None) => Write::Delete { key },
                other => panic!("unexpected write {other:?}"),
            })
            .collect();
        let data = match head {
            None => Repo::format_init_commit(&store, &did, rev, &writes, signer(&key)).unwrap(),
            Some(cid) => Repo::load(&store, cid)
                .unwrap()
                .format_commit(rev, &writes, signer(&key))
                .unwrap(),
        };
        check_commit(name, &data, commit);
        for removed in &data.removed_cids {
            store.remove(removed);
        }
        store.put_all(&data.new_blocks);
        head = Some(data.cid);
        // What the store holds is what the commit reaches: no block is
        // leaked and none is missing.
        let repo = Repo::load(&store, data.cid).unwrap();
        let contents =
            read_tree(&store, &repo.commit().data, &mut Guard::new(store.len())).unwrap();
        assert_eq!(
            contents.leaves.len() as u64,
            commit["records"].as_u64().unwrap(),
            "{name}"
        );
        let records: BTreeSet<Cid> = contents.leaves.iter().map(|(_, c)| *c).collect();
        assert_eq!(
            store.len(),
            1 + contents.nodes.len() + records.len(),
            "{name}: stored blocks"
        );
    }
    (store, head.expect("commits"))
}

#[test]
fn commits_and_export_are_the_reference_bytes() {
    let reference = reference();
    let (store, head) = replay(&reference);
    assert_eq!(head, cid(&reference["export"]["commit"]));
    let repo = Repo::load(&store, head).unwrap();
    let exported = repo.export(Vec::new(), store.len()).unwrap();
    let want = std::fs::read(fixtures().join("export.car")).unwrap();
    assert_eq!(exported.len(), want.len());
    assert!(exported == want, "the export differs from the Reference's");
    // A record can be read back through the tree.
    let (key, value) = {
        let contents =
            read_tree(&store, &repo.commit().data, &mut Guard::new(store.len())).unwrap();
        contents.leaves[contents.leaves.len() / 2].clone()
    };
    let (found, bytes) = repo.get_record(&key).unwrap().unwrap();
    assert_eq!(found, value);
    assert_eq!(cid_for_cbor(&bytes), value);
    assert_eq!(
        repo.get_record("com.example.parity.note/absent").unwrap(),
        None
    );
}

#[test]
fn the_reference_export_imports_and_rebuilds_to_the_signed_root() {
    // The repository CID check in Rust: verify the signature, read the tree, and rebuild
    // the root from nothing but the record keys and CIDs.
    let reference = reference();
    let key = key_of(&reference);
    let did = Did::parse(reference["did"].as_str().unwrap()).unwrap();
    let verify = |message: &[u8], sig: &[u8]| {
        Signature::from_slice(sig).is_ok_and(|s| key.verifying_key().verify(message, &s).is_ok())
    };
    let bytes = std::fs::read(fixtures().join("export.car")).unwrap();
    let expect = Expect {
        did: Some(&did),
        verify: Some(&verify),
    };
    let repo = verify_repo_car(&bytes, &expect, true).unwrap();
    assert_eq!(repo.cid, cid(&reference["export"]["commit"]));
    assert_eq!(
        repo.records.len() as u64,
        reference["export"]["records"].as_u64().unwrap()
    );
    let mut rebuilt = Tree::empty(MemoryBlockStore::new());
    for (k, v) in &repo.records {
        rebuilt.add(k, *v).unwrap();
    }
    assert_eq!(rebuilt.root().unwrap(), repo.commit.data);

    // The wrong account and the wrong key are refused.
    let other = Did::parse("did:plc:aaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let wrong_did = Expect {
        did: Some(&other),
        verify: None,
    };
    assert!(matches!(
        verify_repo_car(&bytes, &wrong_did, true),
        Err(RepoError::Commit(_))
    ));
    let reject = |_: &[u8], _: &[u8]| false;
    let wrong_key = Expect {
        did: None,
        verify: Some(&reject),
    };
    assert!(matches!(
        verify_repo_car(&bytes, &wrong_key, true),
        Err(RepoError::Signature)
    ));

    // A CAR that lacks a record block fails when records are required.
    let car = read_car(&bytes).unwrap();
    let (_, some_record) = repo.records[0];
    let mut without = BlockMap::new();
    for (c, b) in car.blocks.iter().filter(|(c, _)| **c != some_record) {
        without.insert(*c, b.to_vec());
    }
    let partial = write_car(Some(&repo.cid), &without);
    let anyone = Expect {
        did: None,
        verify: None,
    };
    assert!(matches!(
        verify_repo_car(&partial, &anyone, true),
        Err(RepoError::MissingBlock(c)) if *c == some_record
    ));
    assert!(verify_repo_car(&partial, &anyone, false).is_ok());
}

#[test]
fn a_commit_car_verifies_as_a_difference() {
    // Replay all commits but the last, then hand the last one over as a
    // CAR of its new blocks, as an account migration with `since` does.
    let reference = reference();
    let key = key_of(&reference);
    let did = Did::parse(reference["did"].as_str().unwrap()).unwrap();
    let commits = reference["commits"].as_array().unwrap();
    let mut shorter = reference.clone();
    shorter["commits"] = Value::Array(commits[..commits.len() - 1].to_vec());
    let (store, head) = replay(&shorter);
    let last = &commits[commits.len() - 1];
    let encoded = encoded_writes(last);
    let writes: Vec<Write<'_>> = encoded
        .iter()
        .map(|(_, key, record)| Write::Create {
            key,
            record: record.as_deref().unwrap(),
        })
        .collect();
    let current = Repo::load(&store, head).unwrap();
    let rev = Tid::parse(last["rev"].as_str().unwrap()).unwrap();
    let data = current.format_commit(rev, &writes, signer(&key)).unwrap();
    let car = write_car(Some(&data.cid), &data.new_blocks);
    let expect = Expect {
        did: Some(&did),
        verify: None,
    };
    let diff = verify_diff_car(&current, &car, &expect, store.len()).unwrap();
    assert_eq!(diff.cid, data.cid);
    assert_eq!(diff.changes, data.changes);
    assert_eq!(diff.removed_cids, data.removed_cids);
    let cids = |blocks: &BlockMap| blocks.cids().copied().collect::<BTreeSet<_>>();
    assert_eq!(cids(&diff.new_blocks), cids(&data.new_blocks));
}

#[test]
fn trees_that_share_a_node_are_refused_everywhere() {
    let reference = reference();
    let anyone = Expect {
        did: None,
        verify: None,
    };
    for dag in reference["dags"].as_array().unwrap() {
        let file = dag["file"].as_str().unwrap();
        let bytes = std::fs::read(fixtures().join(file)).unwrap();
        let car = read_car(&bytes).unwrap();
        assert_eq!(
            car.blocks.len() as u64,
            dag["blocks"].as_u64().unwrap(),
            "{file}"
        );

        // Import. The work is bounded by the blocks of the CAR whatever the
        // shape promises: 2^40 leaves for the deepest diamond.
        let started = std::time::Instant::now();
        let err = verify_repo_car(&bytes, &anyone, false).unwrap_err();
        assert!(
            started.elapsed().as_secs() < 5,
            "{file}: the import did not stop early"
        );
        assert!(err.is_invalid_input(), "{file}: {err}");

        // The walk alone, with the node codec out of the way: the upstream
        // shapes also break the key rules, so name the defect that stops
        // each one.
        let commit =
            pds_repo::Commit::decode(car.blocks.get(&car.root().unwrap()).unwrap()).unwrap();
        let read =
            read_tree(&car.blocks, &commit.data, &mut Guard::new(car.blocks.len())).unwrap_err();
        assert!(
            matches!(read, RepoError::Tree { .. } | RepoError::Node { .. }),
            "{file}: {read}"
        );

        // Export copies blocks and does not judge keys: it is the shared
        // node itself that stops it.
        let mut copied = 0usize;
        let export = export_blocks(
            &car.blocks,
            &commit.data,
            &mut Guard::new(car.blocks.len()),
            |_, _| {
                copied += 1;
                Ok(())
            },
        );
        match export {
            Err(
                RepoError::Tree {
                    defect: TreeDefect::SharedNode,
                    ..
                }
                | RepoError::Node { .. },
            ) => {}
            other => panic!("{file}: export gave {other:?}"),
        }
        assert!(
            copied <= car.blocks.len(),
            "{file}: export copied {copied} blocks"
        );

        // Comparison with a repository the account already has.
        let (store, head) = {
            let did = Did::parse(reference["did"].as_str().unwrap()).unwrap();
            let key = key_of(&reference);
            let mut store = MemoryBlockStore::new();
            let rev = Tid::parse("3jqfcqzm3fn2j").unwrap();
            let init = Repo::format_init_commit(&store, &did, rev, &[], signer(&key)).unwrap();
            store.put_all(&init.new_blocks);
            (store, init.cid)
        };
        let current = Repo::load(&store, head).unwrap();
        let diff = verify_diff_car(&current, &bytes, &anyone, store.len()).unwrap_err();
        assert!(diff.is_invalid_input(), "{file}: {diff}");
    }
}
