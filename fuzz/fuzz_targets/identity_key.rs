#![no_main]
//! Keys and signatures from outside: a did:key, a Multikey, a DID of a
//! supported method, and a signature to verify.
use libfuzzer_sys::fuzz_target;
use pds_identity::{Algorithm, Keypair, PublicKey, SupportedDid};

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(key) = PublicKey::from_did_key(text) {
            // A did:key has one spelling.
            assert_eq!(key.to_did_key(), text);
        }
        let _ = PublicKey::from_multikey(text);
        for algorithm in [Algorithm::Secp256k1, Algorithm::P256] {
            let _ = PublicKey::from_legacy_multibase(algorithm, text);
        }
        let _ = SupportedDid::parse(text, false);
        let _ = SupportedDid::parse(text, true);
    }
    // The input as a signature over itself, under a fixed key of each curve.
    for algorithm in [Algorithm::Secp256k1, Algorithm::P256] {
        let pair = Keypair::from_secret_bytes(algorithm, &[7u8; 32]).expect("a valid scalar");
        let _ = pair.public_key().verify(data, data);
        let _ = PublicKey::from_sec1_bytes(algorithm, data);
        let _ = Keypair::from_secret_bytes(algorithm, data);
    }
});
