// Included by vectors.rs once for each version of rsky-repo. `rr`, `rc` and
// `rcr` name the three rsky crates, `adapter` the module of pds_bench that
// drives them, and the RSKY_* constants hold what was observed.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use data_encoding::{BASE64_NOPAD, HEXLOWER};
use lexicon_cid::Cid;
use adapter::{keypair, memory_storage, record, runtime, storage_from_car};
use rr::block_map::BlockMap;
use rr::mst::MST;
use rr::repo::Repo;
use rr::sync::consumer::{VerifyRepoInput, verify_repo};
use rr::sync::provider::get_full_repo;
use rr::types::{
    Commit, Lex, RecordCreateOrUpdateOp, RecordDeleteOp, RecordWriteEnum, RecordWriteOp,
    UnsignedCommit, WriteOpAction,
};
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn json(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(root().join(path)).expect(path)).expect("json")
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(root().join("crates/pds-repo/tests/fixtures").join(name)).expect(name)
}

fn cid(v: &Value) -> Cid {
    Cid::try_from(v.as_str().expect("a CID string")).expect("a CID")
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|s| s.as_str().expect("string").to_owned())
        .collect()
}

fn set(v: &Value) -> BTreeSet<String> {
    strings(v).into_iter().collect()
}

/// One line of the comparison, printed with `--nocapture`.
fn report(group: &str, inhouse: &str, rsky: &str) {
    eprintln!("D2 | {group} | in-house: {inhouse} | rsky-repo {VERSION}: {rsky}");
}

/// Runs `f` on its own thread and gives up after `limit`. A walk that does
/// not end keeps its thread until the test process exits.
fn within<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<(T, Duration)> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let start = Instant::now();
        let out = f();
        let _ = tx.send((out, start.elapsed()));
    });
    rx.recv_timeout(limit).ok()
}

#[test]
fn mst_key_heights_and_common_prefix() {
    let heights = json("interop/mst/key_heights.json");
    let prefixes = json("interop/mst/common_prefix.json");
    let (mut ours, mut theirs) = (0, 0);
    for case in heights.as_array().unwrap() {
        let key = case["key"].as_str().unwrap();
        let want = case["height"].as_u64().unwrap();
        ours += usize::from(u64::from(pds_repo::mst::layer_of(key.as_bytes())) == want);
        theirs += usize::from(
            rr::mst::util::leading_zeros_on_hash(key.as_bytes()).is_ok_and(|h| u64::from(h) == want),
        );
    }
    for case in prefixes.as_array().unwrap() {
        let (a, b) = (case["left"].as_str().unwrap(), case["right"].as_str().unwrap());
        let want = case["len"].as_u64().unwrap();
        ours += usize::from(pds_repo::mst::node::common_prefix_len(a, b) as u64 == want);
        theirs += usize::from(
            rr::mst::util::count_prefix_len(a.to_owned(), b.to_owned()).is_ok_and(|n| n as u64 == want),
        );
    }
    report("MST key heights and common prefix (22)", &format!("{ours} pass"), &format!("{theirs} pass"));
    assert_eq!(ours, 22);
    assert_eq!(theirs, 22);
}

#[test]
fn firehose_commit_proofs() {
    let cases = json("interop/firehose/commit-proof-fixtures.json");
    let rt = runtime();
    let (mut ours_roots, mut ours_proofs, mut theirs_roots, mut theirs_proofs) = (0, 0, 0, 0);
    let mut theirs_missing = 0usize;
    for case in cases.as_array().unwrap() {
        let value = cid(&case["leafValue"]);
        let (keys, adds, dels) = (strings(&case["keys"]), strings(&case["adds"]), strings(&case["dels"]));
        let want = set(&case["blocksInProof"]);

        // In-house.
        let mut store = pds_repo::MemoryBlockStore::new();
        let mut tree = pds_repo::mst::Tree::empty(&store);
        for key in &keys {
            tree.add(key, value).unwrap();
        }
        let before = tree.changes().unwrap();
        drop(tree);
        store.put_all(&before.new_blocks);
        let mut tree = pds_repo::mst::Tree::load(&store, before.root).unwrap();
        for key in &adds {
            tree.add(key, value).unwrap();
        }
        for key in &dels {
            tree.delete(key).unwrap();
        }
        let after = tree.root().unwrap();
        let mut proof = pds_repo::BlockMap::new();
        for key in adds.iter().chain(&dels) {
            tree.covering_proof(key, &mut proof).unwrap();
        }
        let got: BTreeSet<String> = proof.cids().map(ToString::to_string).collect();
        ours_roots += usize::from(before.root == cid(&case["rootBeforeCommit"]) && after == cid(&case["rootAfterCommit"]));
        ours_proofs += usize::from(got == want);

        // rsky-repo: the blocks its `format_commit` puts in a commit for
        // each key, which differ between the two versions.
        let (roots_ok, got) = rt.block_on(async {
            let (_, storage) = memory_storage();
            let mut mst = MST::create(storage, None, None).await.unwrap();
            for key in &keys {
                mst = mst.add(key, value, None).await.unwrap();
            }
            let before = mst.get_pointer().await.unwrap();
            for key in &adds {
                mst = mst.add(key, value, None).await.unwrap();
            }
            for key in &dels {
                mst = mst.delete(key).await.unwrap();
            }
            let after = mst.get_pointer().await.unwrap();
            let mut blocks = BlockMap::new();
            for key in adds.iter().chain(&dels) {
                proof_blocks(&mut mst, key, &mut blocks).await;
            }
            let got: BTreeSet<String> = blocks.cids().unwrap().iter().map(ToString::to_string).collect();
            (before == cid(&case["rootBeforeCommit"]) && after == cid(&case["rootAfterCommit"]), got)
        });
        theirs_roots += usize::from(roots_ok);
        theirs_proofs += usize::from(got == want);
        theirs_missing += want.difference(&got).count();
    }
    report(
        "firehose commit proofs (6): roots, proof blocks",
        &format!("{ours_roots} roots, {ours_proofs} proofs"),
        &format!("{theirs_roots} roots, {theirs_proofs} proofs, {theirs_missing} proof blocks missing"),
    );
    assert_eq!((ours_roots, ours_proofs), (6, 6));
    assert_eq!(theirs_roots, 6);
    assert_eq!((theirs_proofs, theirs_missing), RSKY_COMMIT_PROOFS);
}


#[test]
fn reference_tree_roots() {
    let reference = json("crates/pds-repo/tests/fixtures/reference.json");
    let rt = runtime();
    let (mut ours, mut theirs) = (0, 0);
    for tree in reference["trees"].as_array().unwrap() {
        let entries: Vec<(String, Cid)> = tree["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e[0].as_str().unwrap().to_owned(), cid(&e[1])))
            .collect();
        let want = cid(&tree["root"]);
        let mut built = pds_repo::mst::Tree::empty(pds_repo::MemoryBlockStore::new());
        for (k, v) in &entries {
            built.add(k, *v).unwrap();
        }
        ours += usize::from(built.root().unwrap() == want);
        let got = rt.block_on(async {
            let (_, storage) = memory_storage();
            let mut mst = MST::create(storage, None, None).await.unwrap();
            for (k, v) in &entries {
                mst = mst.add(k, *v, None).await.unwrap();
            }
            mst.get_pointer().await.unwrap()
        });
        theirs += usize::from(got == want);
    }
    report("@atproto/repo tree roots (5)", &format!("{ours} pass"), &format!("{theirs} pass"));
    assert_eq!((ours, theirs), (5, 5));
}

#[test]
fn reference_records_and_commit_blocks() {
    let reference = json("crates/pds-repo/tests/fixtures/reference.json");
    let rt = runtime();
    let kp = keypair();
    let did = reference["did"].as_str().unwrap().to_owned();
    let commits = reference["commits"].as_array().unwrap();

    // Record encoding: the CID of every record body.
    let (mut records, mut record_mismatch) = (0, 0);
    for write in commits.iter().flat_map(|c| c["writes"].as_array().unwrap()) {
        let Some(body) = write.get("record") else { continue };
        records += 1;
        let ipld = rr::util::lex_to_ipld(Lex::Map(record(body)));
        let got = rc::ipld::cid_for_cbor(&ipld).unwrap();
        record_mismatch += usize::from(got != cid(&write["cid"]));
    }

    // Commit encoding and signing, with the fixture's `rev` and tree root.
    let mut commit_cids = 0;
    for commit in commits {
        let unsigned = UnsignedCommit {
            did: did.clone(),
            rev: commit["rev"].as_str().unwrap().to_owned(),
            data: cid(&commit["data"]),
            prev: None,
            version: 3,
        };
        let signed = rr::util::sign_commit(unsigned, &kp).unwrap();
        commit_cids += usize::from(rc::ipld::cid_for_cbor(&signed).unwrap() == cid(&commit["cid"]));
    }

    // The commits replayed through `format_commit`. Its `rev` comes from
    // the wall clock and cannot be given, so the commit block itself is
    // left out of the comparison.
    let (mut roots, mut new_sets, mut removed_sets, mut relevant_sets) = (0, 0, 0, 0);
    let mut first_failure: Option<String> = None;
    rt.block_on(async {
        let (_, storage) = memory_storage();
        let mut head: Option<Cid> = None;
        for commit in commits {
            let name = commit["name"].as_str().unwrap();
            let ops: Vec<RecordWriteOp> = commit["writes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| {
                    let (collection, rkey) = w["key"].as_str().unwrap().split_once('/').unwrap();
                    let (collection, rkey) = (collection.to_owned(), rkey.to_owned());
                    match w["action"].as_str().unwrap() {
                        "create" => RecordWriteOp::Create(RecordCreateOrUpdateOp {
                            action: WriteOpAction::Create,
                            collection,
                            rkey,
                            record: record(&w["record"]),
                        }),
                        "update" => RecordWriteOp::Update(RecordCreateOrUpdateOp {
                            action: WriteOpAction::Update,
                            collection,
                            rkey,
                            record: record(&w["record"]),
                        }),
                        _ => RecordWriteOp::Delete(RecordDeleteOp {
                            action: WriteOpAction::Delete,
                            collection,
                            rkey,
                        }),
                    }
                })
                .collect();
            let formatted = match head {
                None => {
                    let creates = ops
                        .into_iter()
                        .filter_map(|op| match op {
                            RecordWriteOp::Create(c) => Some(c),
                            _ => None,
                        })
                        .collect();
                    Repo::format_init_commit(storage.clone(), did.clone(), &kp, Some(creates)).await
                }
                Some(cid) => {
                    let mut repo = Repo::load(storage.clone(), Some(cid)).await.unwrap();
                    repo.format_commit(RecordWriteEnum::List(ops), &kp).await
                }
            };
            let data = match formatted {
                Ok(data) => data,
                Err(e) => {
                    first_failure.get_or_insert(format!("{name}: {e}"));
                    break;
                }
            };
            let without = |cids: Vec<Cid>, skip: &[Option<Cid>]| -> BTreeSet<String> {
                cids.into_iter()
                    .filter(|c| !skip.contains(&Some(*c)))
                    .map(|c| c.to_string())
                    .collect()
            };
            let want_without = |v: &Value, skip: &[&Value]| -> BTreeSet<String> {
                let skip: Vec<&str> = skip.iter().filter_map(|s| s.as_str()).collect();
                set(v).into_iter().filter(|c| !skip.contains(&c.as_str())).collect()
            };
            let ours = [Some(data.cid), head];
            let theirs = [&commit["cid"], &commit["prev"]];
            let new_ok = without(data.new_blocks.cids().unwrap(), &ours) == want_without(&commit["newBlocks"], &theirs);
            let removed_ok =
                without(data.removed_cids.to_list(), &ours) == want_without(&commit["removedCids"], &theirs);
            let relevant_ok =
                without(data.relevant_blocks.cids().unwrap(), &ours) == want_without(&commit["relevantBlocks"], &theirs);
            storage.read().await.apply_commit(data.clone(), None).await.unwrap();
            head = Some(data.cid);
            let root_ok = Repo::load(storage.clone(), head).await.unwrap().commit.data == cid(&commit["data"]);
            roots += usize::from(root_ok);
            new_sets += usize::from(new_ok);
            removed_sets += usize::from(removed_ok);
            relevant_sets += usize::from(relevant_ok);
            if !(root_ok && new_ok && removed_ok) {
                first_failure.get_or_insert(format!("{name}: root {root_ok}, new {new_ok}, removed {removed_ok}"));
            }
        }
    });
    report(
        &format!("@atproto/repo commits ({}): record CIDs of {records}, commit CIDs, roots, new, removed, firehose blocks", commits.len()),
        "all (crates/pds-repo/tests/reference.rs, byte for byte)",
        &format!(
            "{} record CIDs differ; {commit_cids} commit CIDs; {roots} roots, {new_sets} new, {removed_sets} removed, {relevant_sets} firehose block sets; first failure: {first_failure:?}",
            record_mismatch
        ),
    );
    assert_eq!(
        (record_mismatch, commit_cids, roots, new_sets, removed_sets, relevant_sets),
        RSKY_COMMITS,
        "{first_failure:?}"
    );
}


#[test]
fn reference_export_and_import() {
    let reference = json("crates/pds-repo/tests/fixtures/reference.json");
    let bytes = fixture("export.car");
    let rt = runtime();
    let (root, mut blocks, storage) = storage_from_car(&rt, &bytes);
    let exported = rt.block_on(async {
        use futures::StreamExt;
        let stream = get_full_repo(storage, root).await.unwrap();
        futures::pin_mut!(stream);
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            out.extend(chunk.unwrap());
        }
        out
    });
    let export_same = exported == bytes;
    let did = reference["did"].as_str().unwrap().to_owned();
    let did_key = reference["didKey"].as_str().unwrap().to_owned();
    let verified = rt.block_on(verify_repo(&mut blocks, root, Some(&did), Some(&did_key), None));
    let import = match &verified {
        Ok(v) => format!("{} records", v.creates.len()),
        Err(e) => format!("error: {e}"),
    };
    report(
        "@atproto/repo export: re-export is the same bytes; import verifies",
        "same bytes; 308 records",
        &format!("same bytes: {export_same} ({} of {}); import: {import}", exported.len(), bytes.len()),
    );
    assert_eq!(export_same, RSKY_EXPORT_SAME_BYTES);
    assert_eq!(verified.map(|v| v.creates.len()).ok(), Some(308));
}


#[test]
fn p256_commit_signature() {
    // SPEC 6.1: a commit is signed with the actor's key, secp256k1 or
    // P-256. Sign one commit with each and ask both engines to verify.
    let secret = HEXLOWER.decode(pds_bench::KEY_HEX.as_bytes()).unwrap();
    let data = pds_types::content::cid_for_cbor(&[0xa2, 0x61, b'e', 0x80, 0x61, b'l', 0xf6]);
    let mut outcomes = Vec::new();
    for algorithm in [pds_identity::Algorithm::Secp256k1, pds_identity::Algorithm::P256] {
        let pair = pds_identity::Keypair::from_secret_bytes(algorithm, &secret).unwrap();
        let unsigned = pds_repo::UnsignedCommit::new(
            pds_types::Did::parse(pds_bench::DID).unwrap(),
            pds_types::Tid::parse("3jqfcqzm3fo2j").unwrap(),
            data,
        );
        let commit = unsigned.with_signature(pair.sign(&pds_repo::UnsignedCommit::new(
            pds_types::Did::parse(pds_bench::DID).unwrap(),
            pds_types::Tid::parse("3jqfcqzm3fo2j").unwrap(),
            data,
        ).to_bytes()).to_vec());
        let public = pair.public_key();
        let ours = commit.verify(|message, sig| public.verify(message, sig).is_ok()).is_ok();
        let theirs = rr::util::verify_commit_sig(
            Commit {
                did: pds_bench::DID.to_owned(),
                rev: "3jqfcqzm3fo2j".to_owned(),
                data,
                prev: None,
                version: 3,
                sig: commit.sig.clone(),
            },
            &pair.did_key(),
        );
        outcomes.push((algorithm.jwt_alg(), ours, theirs.map_err(|e| e.to_string())));
    }
    report(
        "commit signature verifies: ES256K, ES256",
        &format!("{}, {}", outcomes[0].1, outcomes[1].1),
        &format!("{:?}, {:?}", outcomes[0].2, outcomes[1].2),
    );
    assert!(outcomes[0].1 && outcomes[1].1);
    assert_eq!(outcomes[0].2, Ok(true));
    assert_eq!(outcomes[1].2.clone().unwrap_or(false), RSKY_VERIFIES_P256_COMMITS);
}


#[test]
fn crypto_signature_fixtures() {
    let cases = json("interop/crypto/signature-fixtures.json");
    let (mut ours, mut theirs) = (0, 0);
    let mut theirs_wrong = Vec::new();
    for case in cases.as_array().unwrap() {
        let did = case["publicKeyDid"].as_str().unwrap().to_owned();
        let message = BASE64_NOPAD.decode(case["messageBase64"].as_str().unwrap().as_bytes()).unwrap();
        let sig = BASE64_NOPAD.decode(case["signatureBase64"].as_str().unwrap().as_bytes()).unwrap();
        let want = case["validSignature"].as_bool().unwrap();
        let key = pds_identity::PublicKey::from_did_key(&did).unwrap();
        ours += usize::from(key.verify(&message, &sig).is_ok() == want);
        let digest = pds_types::content::sha256(&message);
        let got = rcr::verify::verify_signature_digest(&did, &digest, &sig, None).unwrap_or(false);
        if got == want {
            theirs += 1;
        } else {
            theirs_wrong.push(case["comment"].as_str().unwrap().to_owned());
        }
    }
    report("crypto signature fixtures (6)", &format!("{ours} pass"), &format!("{theirs} pass; wrong: {theirs_wrong:?}"));
    assert_eq!(ours, 6);
    assert_eq!(theirs, RSKY_SIGNATURE_FIXTURES);
}


#[test]
fn data_model_fixtures() {
    // JSON to DAG-CBOR bytes and CID, for the three upstream fixtures, and
    // whether a float is refused.
    let cases = json("interop/data-model/data-model-fixtures.json");
    let (mut ours, mut theirs) = (0, 0);
    for case in cases.as_array().unwrap() {
        let want = BASE64_NOPAD
            .decode(case["cbor_base64"].as_str().unwrap().trim_end_matches('=').as_bytes())
            .unwrap();
        let data = pds_types::Data::from_json_str(&case["json"].to_string()).unwrap();
        ours += usize::from(data.to_dag_cbor() == want);
        let got = serde_json::from_value::<Lex>(case["json"].clone())
            .ok()
            .and_then(|lex| rc::struct_to_cbor(&rr::util::lex_to_ipld(lex)).ok());
        theirs += usize::from(got.as_deref() == Some(want.as_slice()));
    }
    let float = serde_json::json!({ "$type": "com.example.blah", "a": 123.456 });
    let ours_refuses = pds_types::Data::from_json_str(&float.to_string()).is_err();
    let theirs_encodes = serde_json::from_value::<Lex>(float)
        .ok()
        .and_then(|lex| rc::struct_to_cbor(&rr::util::lex_to_ipld(lex)).ok())
        .is_some();
    report(
        "data model fixtures (3); a float in a record",
        &format!("{ours} pass; float refused: {ours_refuses}"),
        &format!("{theirs} pass; float encoded: {theirs_encodes}"),
    );
    assert_eq!(ours, 3);
    assert!(ours_refuses);
    assert_eq!((theirs, theirs_encodes), RSKY_DATA_MODEL);
}


#[test]
fn trees_that_share_a_node() {
    // The shapes of upstream's mst-dag.test.ts, as CAR files written by
    // @atproto/repo at the pin. The in-house engine refuses each within
    // the CAR's block count. For rsky-repo the time of an import is
    // recorded; a walk that has not ended after ten seconds is abandoned.
    let reference = json("crates/pds-repo/tests/fixtures/reference.json");
    let anyone = pds_repo::Expect { did: None, verify: None };
    let mut lines = Vec::new();
    let mut rsky_refused = 0;
    for dag in reference["dags"].as_array().unwrap() {
        let file = dag["file"].as_str().unwrap().to_owned();
        let bytes = fixture(&file);
        let start = Instant::now();
        let ours = pds_repo::verify_repo_car(&bytes, &anyone, false);
        let ours_time = start.elapsed();
        assert!(ours.is_err(), "{file}");
        assert!(ours_time < Duration::from_secs(1), "{file}");

        let car = bytes.clone();
        let import = within(Duration::from_secs(10), move || {
            let rt = runtime();
            let (root, mut blocks, _) = storage_from_car(&rt, &car);
            rt.block_on(verify_repo(
                &mut blocks,
                root,
                None,
                None,
                Some(VerifyRepoInput { ensure_leaves: Some(false) }),
            ))
            .map(|v| v.creates.len())
            .map_err(|e| e.to_string())
        });
        let car = bytes.clone();
        let export = within(Duration::from_secs(10), move || {
            let rt = runtime();
            let (root, _, storage) = storage_from_car(&rt, &car);
            rt.block_on(async {
                use futures::StreamExt;
                let stream = match get_full_repo(storage, root).await {
                    Ok(s) => s,
                    Err(e) => return Err(e.to_string()),
                };
                futures::pin_mut!(stream);
                let mut total = 0usize;
                while let Some(chunk) = stream.next().await {
                    total += chunk.map_err(|e| e.to_string())?.len();
                }
                Ok(total)
            })
        });
        let show = |r: &Option<(Result<usize, String>, Duration)>, unit: &str| match r {
            None => "no end after 10 s".to_owned(),
            Some((Ok(n), t)) => format!("accepted, {n} {unit}, {:.1} ms", t.as_secs_f64() * 1000.0),
            Some((Err(e), t)) => format!("refused in {:.1} ms ({})", t.as_secs_f64() * 1000.0, e.chars().take(60).collect::<String>()),
        };
        if matches!(import, Some((Err(_), _))) {
            rsky_refused += 1;
        }
        lines.push(format!(
            "{file} ({} blocks): in-house refused in {:.2} ms ({}); rsky import: {}; rsky export: {}",
            dag["blocks"],
            ours_time.as_secs_f64() * 1000.0,
            ours.err().map(|e| e.to_string()).unwrap_or_default().chars().take(70).collect::<String>(),
            show(&import, "records"),
            show(&export, "bytes"),
        ));
    }
    for line in &lines {
        report("shared-node trees", "", line);
    }
    assert_eq!(rsky_refused, RSKY_REFUSES_SHARED_NODE_CARS, "{lines:#?}");
}

