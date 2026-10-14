//! Operations of the `did:plc` method, as `@did-plc/lib` at the commit the
//! parity stack pins forms them.
//!
//! An operation is signed over its DAG-CBOR without `sig`. The DID of a
//! genesis operation is the first 24 characters of the base32 SHA-256 of
//! the signed operation's DAG-CBOR.

use std::collections::BTreeMap;

use data_encoding::{BASE32_NOPAD, BASE64URL_NOPAD};
use pds_types::Data;
use sha2::{Digest as _, Sha256};

use crate::Keypair;

/// A genesis operation and the DID it creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Genesis {
    /// The DID.
    pub did: String,
    /// The signed operation, as the directory takes it.
    pub operation: serde_json::Value,
}

fn text(value: &str) -> Data {
    Data::String(value.to_owned())
}

fn object(entries: &[(&str, Data)]) -> Data {
    let map: BTreeMap<String, Data> = entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect();
    Data::Object(map)
}

/// The genesis operation of an atproto account: its signing key, its
/// rotation keys in order of authority, its handle and its PDS. `signer`
/// is one of the rotation keys.
pub fn create_op(
    signing_key: &str,
    rotation_keys: &[String],
    handle: &str,
    pds: &str,
    signer: &Keypair,
) -> Genesis {
    let also_known_as = at_prefixed(handle);
    let endpoint = http_prefixed(pds);
    let mut fields = vec![
        ("type", text("plc_operation")),
        (
            "rotationKeys",
            Data::Array(rotation_keys.iter().map(|key| text(key)).collect()),
        ),
        (
            "verificationMethods",
            object(&[("atproto", text(signing_key))]),
        ),
        ("alsoKnownAs", Data::Array(vec![text(&also_known_as)])),
        (
            "services",
            object(&[(
                "atproto_pds",
                object(&[
                    ("type", text("AtprotoPersonalDataServer")),
                    ("endpoint", text(&endpoint)),
                ]),
            )]),
        ),
        ("prev", Data::Null),
    ];
    let unsigned = object(&fields).to_dag_cbor();
    let signature = BASE64URL_NOPAD.encode(&signer.sign(&unsigned));
    fields.push(("sig", text(&signature)));
    let complete = object(&fields);
    let digest: [u8; 32] = Sha256::digest(complete.to_dag_cbor()).into();
    let hash = BASE32_NOPAD.encode(&digest).to_lowercase();
    let did = format!("did:plc:{}", hash.get(..24).unwrap_or_default());
    Genesis {
        did,
        // The data model has no value that JSON lacks here.
        operation: serde_json::to_value(&complete).unwrap_or_default(),
    }
}

/// Whether `key`, a `did:key`, signed the operation: the signature is
/// over the DAG-CBOR of the operation without its `sig`, as
/// `assureValidSig` of the library checks it.
pub fn is_signed_by(operation: &serde_json::Value, key: &str) -> bool {
    let Ok(Data::Object(mut fields)) = Data::from_json_str(&operation.to_string()) else {
        return false;
    };
    let Some(Data::String(signature)) = fields.remove("sig") else {
        return false;
    };
    let (Ok(signature), Ok(key)) = (
        BASE64URL_NOPAD.decode(signature.as_bytes()),
        crate::PublicKey::from_did_key(key),
    ) else {
        return false;
    };
    key.verify(&Data::Object(fields).to_dag_cbor(), &signature)
        .is_ok()
}

/// The DID that a genesis operation creates: `did:plc:` and the first 24
/// characters of the base32 SHA-256 of the signed operation.
pub fn did_of_genesis(operation: &serde_json::Value) -> Option<String> {
    let complete = Data::from_json_str(&operation.to_string()).ok()?;
    let digest: [u8; 32] = Sha256::digest(complete.to_dag_cbor()).into();
    let hash = BASE32_NOPAD.encode(&digest).to_lowercase();
    Some(format!("did:plc:{}", hash.get(..24)?))
}

/// Why an operation cannot follow the last one of a DID.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UpdateError {
    /// The DID is tombstoned.
    #[error("Cannot apply op to tombstone")]
    Tombstone,
    /// The last operation is not one this server can read.
    #[error("the last operation of the DID is malformed")]
    Malformed,
}

/// The fields of an operation in today's form, without `sig` and `prev`.
/// An operation of the first form of the method is translated, as
/// `normalizeOp` of the library does.
fn normalized(last: &Data) -> Result<BTreeMap<String, Data>, UpdateError> {
    let Data::Object(fields) = last else {
        return Err(UpdateError::Malformed);
    };
    let field = |key: &str| fields.get(key).cloned().ok_or(UpdateError::Malformed);
    let kind = match fields.get("type") {
        Some(Data::String(kind)) => kind.as_str(),
        _ => return Err(UpdateError::Malformed),
    };
    match kind {
        "plc_tombstone" => Err(UpdateError::Tombstone),
        "plc_operation" => {
            let mut fields = fields.clone();
            fields.remove("sig");
            fields.remove("prev");
            Ok(fields)
        }
        "create" => {
            let (Data::String(handle), Data::String(service)) =
                (field("handle")?, field("service")?)
            else {
                return Err(UpdateError::Malformed);
            };
            let Data::Object(map) = object(&[
                ("type", text("plc_operation")),
                (
                    "verificationMethods",
                    object(&[("atproto", field("signingKey")?)]),
                ),
                (
                    "rotationKeys",
                    Data::Array(vec![field("recoveryKey")?, field("signingKey")?]),
                ),
                (
                    "alsoKnownAs",
                    Data::Array(vec![text(&at_prefixed(&handle))]),
                ),
                (
                    "services",
                    object(&[(
                        "atproto_pds",
                        object(&[
                            ("type", text("AtprotoPersonalDataServer")),
                            ("endpoint", text(&http_prefixed(&service))),
                        ]),
                    )]),
                ),
            ]) else {
                return Err(UpdateError::Malformed);
            };
            Ok(map)
        }
        _ => Err(UpdateError::Malformed),
    }
}

fn at_prefixed(handle: &str) -> String {
    if handle.starts_with("at://") {
        handle.to_owned()
    } else {
        format!("at://{handle}")
    }
}

fn http_prefixed(pds: &str) -> String {
    if pds.starts_with("http://") || pds.starts_with("https://") {
        pds.to_owned()
    } else {
        format!("https://{pds}")
    }
}

/// The operation that follows `last` with the fields that `change` edits,
/// signed by `signer`. `last` is the last operation of the DID as the
/// directory serves it.
pub fn update_op(
    last: &serde_json::Value,
    signer: &Keypair,
    change: impl FnOnce(&mut BTreeMap<String, Data>),
) -> Result<serde_json::Value, UpdateError> {
    let last = Data::from_json_str(&last.to_string()).map_err(|_| UpdateError::Malformed)?;
    let mut fields = normalized(&last)?;
    change(&mut fields);
    let prev = pds_types::content::cid_for_cbor(&last.to_dag_cbor()).to_string();
    fields.insert("prev".to_owned(), text(&prev));
    let unsigned = Data::Object(fields.clone()).to_dag_cbor();
    let signature = BASE64URL_NOPAD.encode(&signer.sign(&unsigned));
    fields.insert("sig".to_owned(), text(&signature));
    Ok(serde_json::to_value(Data::Object(fields)).unwrap_or_default())
}

/// The operation that gives the DID the handle `handle`: it replaces the
/// first `at://` name, or becomes the first name.
pub fn update_handle_op(
    last: &serde_json::Value,
    signer: &Keypair,
    handle: &str,
) -> Result<serde_json::Value, UpdateError> {
    let name = text(&at_prefixed(handle));
    update_op(last, signer, |fields| {
        let mut names = match fields.remove("alsoKnownAs") {
            Some(Data::Array(names)) => names,
            _ => Vec::new(),
        };
        let at = names
            .iter()
            .position(|known| matches!(known, Data::String(known) if known.starts_with("at://")));
        match at {
            Some(at) => names[at] = name,
            None => names.insert(0, name),
        }
        fields.insert("alsoKnownAs".to_owned(), Data::Array(names));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_genesis_operation_names_its_did_and_its_signer() {
        let signer = Keypair::generate(crate::Algorithm::Secp256k1, |bytes| bytes.fill(3));
        let other = Keypair::generate(crate::Algorithm::Secp256k1, |bytes| bytes.fill(4));
        let genesis = create_op(
            &other.did_key(),
            &[signer.did_key()],
            "alice.test",
            "https://pds.test",
            &signer,
        );
        assert_eq!(
            did_of_genesis(&genesis.operation).as_deref(),
            Some(genesis.did.as_str())
        );
        assert!(is_signed_by(&genesis.operation, &signer.did_key()));
        assert!(!is_signed_by(&genesis.operation, &other.did_key()));
        let mut changed = genesis.operation;
        changed["alsoKnownAs"] = serde_json::json!(["at://mallory.test"]);
        assert!(!is_signed_by(&changed, &signer.did_key()));
        assert!(!is_signed_by(
            &serde_json::json!({"type": "plc_operation"}),
            &signer.did_key()
        ));
    }
    use crate::Algorithm;

    #[test]
    fn a_genesis_operation_names_its_did_and_carries_a_signature_of_the_signer() {
        let signer = Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(7));
        let account = Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(9));
        let rotation = vec![signer.did_key()];
        let genesis = create_op(
            &account.did_key(),
            &rotation,
            "alice.pds.test",
            "https://pds.test",
            &signer,
        );
        assert!(genesis.did.starts_with("did:plc:"));
        assert_eq!(genesis.did.len(), 32);
        let op = &genesis.operation;
        assert_eq!(op["type"], "plc_operation");
        assert_eq!(op["prev"], serde_json::Value::Null);
        assert_eq!(op["alsoKnownAs"][0], "at://alice.pds.test");
        assert_eq!(
            op["services"]["atproto_pds"]["endpoint"],
            "https://pds.test"
        );
        assert_eq!(op["verificationMethods"]["atproto"], account.did_key());

        // The signature is over the operation without `sig`.
        let mut unsigned = Data::from_json_str(&op.to_string()).unwrap();
        if let Data::Object(map) = &mut unsigned {
            map.remove("sig");
        }
        let signature = BASE64URL_NOPAD
            .decode(op["sig"].as_str().unwrap().as_bytes())
            .unwrap();
        signer
            .public_key()
            .verify(&unsigned.to_dag_cbor(), &signature)
            .unwrap();
        // The same input gives the same DID: signatures are deterministic.
        let again = create_op(
            &account.did_key(),
            &rotation,
            "alice.pds.test",
            "pds.test",
            &signer,
        );
        assert_eq!(again, genesis);
    }

    #[test]
    fn an_update_follows_the_last_operation_and_keeps_the_other_fields() {
        let signer = Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(7));
        let account = Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(9));
        let genesis = create_op(
            &account.did_key(),
            &[signer.did_key()],
            "alice.pds.test",
            "https://pds.test",
            &signer,
        );
        let next = update_handle_op(&genesis.operation, &signer, "alicia.pds.test").unwrap();
        assert_eq!(
            next["alsoKnownAs"],
            serde_json::json!(["at://alicia.pds.test"])
        );
        assert_eq!(next["services"], genesis.operation["services"]);
        assert_eq!(next["rotationKeys"], genesis.operation["rotationKeys"]);
        let last = Data::from_json_str(&genesis.operation.to_string()).unwrap();
        assert_eq!(
            next["prev"],
            pds_types::content::cid_for_cbor(&last.to_dag_cbor()).to_string()
        );
        assert_ne!(next["sig"], genesis.operation["sig"]);
        let tombstone = serde_json::json!({"type": "plc_tombstone", "prev": "x", "sig": "y"});
        assert_eq!(
            update_handle_op(&tombstone, &signer, "a.pds.test"),
            Err(UpdateError::Tombstone)
        );
    }
}
