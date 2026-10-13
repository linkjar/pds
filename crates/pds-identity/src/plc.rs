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
    let also_known_as = if handle.starts_with("at://") {
        handle.to_owned()
    } else {
        format!("at://{handle}")
    };
    let endpoint = if pds.starts_with("http://") || pds.starts_with("https://") {
        pds.to_owned()
    } else {
        format!("https://{pds}")
    };
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
