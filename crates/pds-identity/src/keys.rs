//! Keys and signatures (SPEC §6.1, §9.4).
//!
//! Two curves: secp256k1 (`ES256K`) and NIST P-256 (`ES256`). A signature is
//! ECDSA over the SHA-256 digest of the message, as 64 bytes `r || s`, with
//! `s` in the lower half of the curve order on both curves. DER signatures
//! and high-`s` signatures do not verify. Signing is deterministic (RFC
//! 6979), so one key and one message give one signature. All arithmetic is
//! RustCrypto's `k256` and `p256`.

use std::fmt;

use k256::ecdsa::signature::{Signer, Verifier};
use zeroize::Zeroizing;

/// The longest base58btc key string that is decoded. A compressed key of
/// either curve is 48 or 49 characters; upstream bounds the same input.
const MAX_MULTIBASE_KEY_LEN: usize = 128;

/// The `did:key:` prefix.
const DID_KEY_PREFIX: &str = "did:key:";

/// A supported signature algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Algorithm {
    /// ECDSA on secp256k1 with SHA-256, `ES256K`.
    Secp256k1,
    /// ECDSA on NIST P-256 with SHA-256, `ES256`.
    P256,
}

impl Algorithm {
    /// The JWT `alg` name.
    pub const fn jwt_alg(self) -> &'static str {
        match self {
            Self::Secp256k1 => "ES256K",
            Self::P256 => "ES256",
        }
    }

    /// The algorithm for a JWT `alg` name.
    pub fn from_jwt_alg(alg: &str) -> Option<Self> {
        match alg {
            "ES256K" => Some(Self::Secp256k1),
            "ES256" => Some(Self::P256),
            _ => None,
        }
    }

    /// The multicodec prefix of a compressed public key, as a varint:
    /// `secp256k1-pub` is 0xe7 and `p256-pub` is 0x1200.
    const fn multicodec(self) -> [u8; 2] {
        match self {
            Self::Secp256k1 => [0xe7, 0x01],
            Self::P256 => [0x80, 0x24],
        }
    }
}

/// A key that cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// Not a `did:key:` string.
    #[error("not a did:key")]
    NotDidKey,
    /// Not base58btc multibase, or longer than a key can be.
    #[error("key is not base58btc multibase")]
    Multibase,
    /// A key type other than secp256k1 and P-256.
    #[error("unsupported key type")]
    UnsupportedType,
    /// The bytes are not a point on the curve, or not a valid secret scalar.
    #[error("key bytes are not valid for the curve")]
    Invalid,
}

/// Why a signature does not verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    /// Not 64 bytes of `r || s` in range. A DER signature fails here.
    #[error("signature is not 64 bytes of r and s")]
    Malformed,
    /// `s` is in the upper half of the curve order.
    #[error("signature is not low-S")]
    HighS,
    /// The signature is well formed and does not match the key and message.
    #[error("signature does not match")]
    Mismatch,
}

/// A public key of either curve.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PublicKey {
    /// A secp256k1 key.
    Secp256k1(k256::ecdsa::VerifyingKey),
    /// A P-256 key.
    P256(p256::ecdsa::VerifyingKey),
}

impl PublicKey {
    /// The key's algorithm.
    pub const fn algorithm(&self) -> Algorithm {
        match self {
            Self::Secp256k1(_) => Algorithm::Secp256k1,
            Self::P256(_) => Algorithm::P256,
        }
    }

    /// Reads SEC1 bytes, compressed (33 bytes) or uncompressed (65 bytes).
    pub fn from_sec1_bytes(algorithm: Algorithm, bytes: &[u8]) -> Result<Self, KeyError> {
        match algorithm {
            Algorithm::Secp256k1 => k256::ecdsa::VerifyingKey::from_sec1_bytes(bytes)
                .map(Self::Secp256k1)
                .map_err(|_| KeyError::Invalid),
            Algorithm::P256 => p256::ecdsa::VerifyingKey::from_sec1_bytes(bytes)
                .map(Self::P256)
                .map_err(|_| KeyError::Invalid),
        }
    }

    /// The uncompressed SEC1 form, 65 bytes: `04`, `x`, `y`. A JWK holds
    /// the two coordinates.
    pub fn to_uncompressed(&self) -> Vec<u8> {
        match self {
            Self::Secp256k1(k) => k.to_sec1_point(false).as_bytes().to_vec(),
            Self::P256(k) => k.to_sec1_point(false).as_bytes().to_vec(),
        }
    }

    /// The compressed SEC1 form, 33 bytes.
    pub fn to_compressed(&self) -> Vec<u8> {
        match self {
            Self::Secp256k1(k) => k.to_sec1_point(true).as_bytes().to_vec(),
            Self::P256(k) => k.to_sec1_point(true).as_bytes().to_vec(),
        }
    }

    /// Reads a Multikey string: base58btc multibase over the multicodec
    /// prefix and the compressed key.
    pub fn from_multikey(s: &str) -> Result<Self, KeyError> {
        let bytes = decode_base58btc(s)?;
        for algorithm in [Algorithm::Secp256k1, Algorithm::P256] {
            if let Some(key) = bytes.strip_prefix(&algorithm.multicodec()) {
                // A did:key carries the compressed form only.
                if key.len() != 33 {
                    return Err(KeyError::Invalid);
                }
                return Self::from_sec1_bytes(algorithm, key);
            }
        }
        Err(KeyError::UnsupportedType)
    }

    /// The Multikey string.
    pub fn to_multikey(&self) -> String {
        let mut bytes = self.algorithm().multicodec().to_vec();
        bytes.extend_from_slice(&self.to_compressed());
        multibase::encode(multibase::Base::Base58Btc, bytes)
    }

    /// Reads a `did:key`.
    pub fn from_did_key(did: &str) -> Result<Self, KeyError> {
        let key = did
            .strip_prefix(DID_KEY_PREFIX)
            .ok_or(KeyError::NotDidKey)?;
        Self::from_multikey(key)
    }

    /// The `did:key` of this key.
    pub fn to_did_key(&self) -> String {
        format!("{DID_KEY_PREFIX}{}", self.to_multikey())
    }

    /// Reads the `publicKeyMultibase` of a legacy verification method: the
    /// SEC1 bytes without a multicodec prefix, compressed or uncompressed.
    pub fn from_legacy_multibase(algorithm: Algorithm, s: &str) -> Result<Self, KeyError> {
        Self::from_sec1_bytes(algorithm, &decode_base58btc(s)?)
    }

    /// Verifies `signature` over `message`.
    ///
    /// The signature must be 64 bytes of `r || s` with a low `s`; the
    /// message is hashed with SHA-256.
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), SignatureError> {
        match self {
            Self::Secp256k1(key) => {
                let sig = k256::ecdsa::Signature::from_slice(signature)
                    .map_err(|_| SignatureError::Malformed)?;
                if sig.normalize_s() != sig {
                    return Err(SignatureError::HighS);
                }
                key.verify(message, &sig)
                    .map_err(|_| SignatureError::Mismatch)
            }
            Self::P256(key) => {
                let sig = p256::ecdsa::Signature::from_slice(signature)
                    .map_err(|_| SignatureError::Malformed)?;
                // The curve crate accepts a high `s` on P-256; atproto does not.
                if sig.normalize_s() != sig {
                    return Err(SignatureError::HighS);
                }
                key.verify(message, &sig)
                    .map_err(|_| SignatureError::Mismatch)
            }
        }
    }
}

impl PublicKey {
    /// Verifies a signature whose `s` may be high or low, as JOSE allows.
    /// Web browsers sign that way; the protocol's own signatures go
    /// through [`PublicKey::verify`].
    pub fn verify_any_s(&self, message: &[u8], signature: &[u8]) -> Result<(), SignatureError> {
        match self {
            Self::Secp256k1(key) => {
                let sig = k256::ecdsa::Signature::from_slice(signature)
                    .map_err(|_| SignatureError::Malformed)?;
                key.verify(message, &sig.normalize_s())
                    .map_err(|_| SignatureError::Mismatch)
            }
            Self::P256(key) => {
                let sig = p256::ecdsa::Signature::from_slice(signature)
                    .map_err(|_| SignatureError::Malformed)?;
                key.verify(message, &sig)
                    .or_else(|_| key.verify(message, &sig.normalize_s()))
                    .map_err(|_| SignatureError::Mismatch)
            }
        }
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.to_did_key())
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_did_key())
    }
}

fn decode_base58btc(s: &str) -> Result<Vec<u8>, KeyError> {
    if s.len() > MAX_MULTIBASE_KEY_LEN || !s.starts_with('z') {
        return Err(KeyError::Multibase);
    }
    match multibase::decode(s) {
        Ok((multibase::Base::Base58Btc, bytes)) => Ok(bytes),
        _ => Err(KeyError::Multibase),
    }
}

enum Secret {
    Secp256k1(k256::ecdsa::SigningKey),
    P256(p256::ecdsa::SigningKey),
}

/// A private key with its public key. The secret is erased when the value
/// is dropped, and `Debug` prints the public key only.
pub struct Keypair(Secret);

impl Keypair {
    /// Reads a secret scalar: 32 bytes, big endian, in the range of the
    /// curve order. This is the form of an actor's `key` file.
    pub fn from_secret_bytes(algorithm: Algorithm, bytes: &[u8]) -> Result<Self, KeyError> {
        // The curve crates pad a shorter input with zeros. A key file of
        // another length is damaged, not short.
        if bytes.len() != 32 {
            return Err(KeyError::Invalid);
        }
        match algorithm {
            Algorithm::Secp256k1 => k256::ecdsa::SigningKey::from_slice(bytes)
                .map(|k| Self(Secret::Secp256k1(k)))
                .map_err(|_| KeyError::Invalid),
            Algorithm::P256 => p256::ecdsa::SigningKey::from_slice(bytes)
                .map(|k| Self(Secret::P256(k)))
                .map_err(|_| KeyError::Invalid),
        }
    }

    /// Makes a key from the caller's random source, which fills the buffer
    /// it is given. Randomness is injected so that simulation can replay it
    /// (SPEC §3.2).
    pub fn generate(algorithm: Algorithm, mut fill: impl FnMut(&mut [u8])) -> Self {
        let mut bytes = Zeroizing::new([0u8; 32]);
        loop {
            fill(bytes.as_mut_slice());
            // All but a negligible share of 32-byte strings are valid scalars.
            if let Ok(key) = Self::from_secret_bytes(algorithm, bytes.as_slice()) {
                return key;
            }
        }
    }

    /// The key's algorithm.
    pub const fn algorithm(&self) -> Algorithm {
        match self.0 {
            Secret::Secp256k1(_) => Algorithm::Secp256k1,
            Secret::P256(_) => Algorithm::P256,
        }
    }

    /// The public key.
    pub fn public_key(&self) -> PublicKey {
        match &self.0 {
            Secret::Secp256k1(k) => PublicKey::Secp256k1(*k.verifying_key()),
            Secret::P256(k) => PublicKey::P256(*k.verifying_key()),
        }
    }

    /// The `did:key` of the public key.
    pub fn did_key(&self) -> String {
        self.public_key().to_did_key()
    }

    /// Signs `message`: SHA-256, a deterministic nonce (RFC 6979), low `s`.
    /// Returns 64 bytes of `r || s`.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        let mut out = [0u8; 64];
        match &self.0 {
            Secret::Secp256k1(k) => {
                let sig: k256::ecdsa::Signature = k.sign(message);
                out.copy_from_slice(&sig.to_bytes());
            }
            Secret::P256(k) => {
                let sig: p256::ecdsa::Signature = k.sign(message);
                // The curve crate does not normalise P-256 signatures.
                out.copy_from_slice(&sig.normalize_s().to_bytes());
            }
        }
        out
    }

    /// The secret scalar, for the export paths the Reference has (SPEC
    /// §15.2). The copy is erased when dropped.
    pub fn secret_bytes(&self) -> Zeroizing<[u8; 32]> {
        let mut out = Zeroizing::new([0u8; 32]);
        match &self.0 {
            Secret::Secp256k1(k) => out.copy_from_slice(&k.to_bytes()),
            Secret::P256(k) => out.copy_from_slice(&k.to_bytes()),
        }
        out
    }
}

impl fmt::Debug for Keypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Keypair({}, secret redacted)", self.did_key())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use data_encoding::{BASE64_NOPAD, HEXLOWER};
    use serde_json::Value;

    use super::*;

    fn fixture(name: &str) -> Vec<Value> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../interop/crypto")
            .join(name);
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
    }

    /// Interop vectors that do not pass, with the finding for each. None.
    const KNOWN_FAILURES: &[(&str, &str)] = &[];

    #[test]
    fn signature_fixtures() {
        let cases = fixture("signature-fixtures.json");
        assert_eq!(cases.len(), 6);
        for case in cases {
            let comment = case["comment"].as_str().unwrap();
            assert!(!KNOWN_FAILURES.iter().any(|(name, _)| *name == comment));
            let algorithm = Algorithm::from_jwt_alg(case["algorithm"].as_str().unwrap()).unwrap();
            let key = PublicKey::from_did_key(case["publicKeyDid"].as_str().unwrap()).unwrap();
            assert_eq!(key.algorithm(), algorithm, "{comment}");
            assert_eq!(key.to_did_key(), case["publicKeyDid"], "{comment}");
            // The legacy DID document form is the same key without the
            // multicodec prefix.
            let legacy = PublicKey::from_legacy_multibase(
                algorithm,
                case["publicKeyMultibase"].as_str().unwrap(),
            )
            .unwrap();
            assert_eq!(legacy, key, "{comment}");

            let message = BASE64_NOPAD
                .decode(case["messageBase64"].as_str().unwrap().as_bytes())
                .unwrap();
            let signature = BASE64_NOPAD
                .decode(case["signatureBase64"].as_str().unwrap().as_bytes())
                .unwrap();
            let result = key.verify(&message, &signature);
            let tags: Vec<&str> = case["tags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t.as_str().unwrap())
                .collect();
            let want = if case["validSignature"].as_bool().unwrap() {
                Ok(())
            } else if tags.contains(&"high-s") {
                Err(SignatureError::HighS)
            } else if tags.contains(&"der-encoded") {
                Err(SignatureError::Malformed)
            } else {
                Err(SignatureError::Mismatch)
            };
            assert_eq!(result, want, "{comment}");
        }
    }

    #[test]
    fn w3c_did_key_fixtures() {
        let k256 = fixture("w3c_didkey_K256.json");
        assert_eq!(k256.len(), 5);
        for case in k256 {
            let secret = HEXLOWER
                .decode(case["privateKeyBytesHex"].as_str().unwrap().as_bytes())
                .unwrap();
            let pair = Keypair::from_secret_bytes(Algorithm::Secp256k1, &secret).unwrap();
            assert_eq!(pair.did_key(), case["publicDidKey"]);
        }
        let p256 = fixture("w3c_didkey_P256.json");
        assert_eq!(p256.len(), 1);
        for case in p256 {
            let text = format!("z{}", case["privateKeyBytesBase58"].as_str().unwrap());
            let (_, secret) = multibase::decode(text).unwrap();
            let pair = Keypair::from_secret_bytes(Algorithm::P256, &secret).unwrap();
            assert_eq!(pair.did_key(), case["publicDidKey"]);
        }
    }

    fn counter_fill() -> impl FnMut(&mut [u8]) {
        let mut n = 0u8;
        move |buf: &mut [u8]| {
            n = n.wrapping_add(1);
            buf.fill(n);
        }
    }

    #[test]
    fn signatures_are_deterministic_low_s_and_verify() {
        for algorithm in [Algorithm::Secp256k1, Algorithm::P256] {
            let pair = Keypair::generate(algorithm, counter_fill());
            let public = pair.public_key();
            for i in 0..64u8 {
                let message = vec![i; usize::from(i) + 1];
                let sig = pair.sign(&message);
                assert_eq!(sig, pair.sign(&message), "RFC 6979 gives one signature");
                assert_eq!(public.verify(&message, &sig), Ok(()));
                assert_eq!(
                    public.verify(b"another message", &sig),
                    Err(SignatureError::Mismatch)
                );
                // The other half of the curve order is refused.
                let high = flip_s(algorithm, &sig);
                assert_eq!(public.verify(&message, &high), Err(SignatureError::HighS));
            }
            assert_eq!(
                public.verify(b"m", &[0u8; 64]),
                Err(SignatureError::Malformed)
            );
            assert_eq!(
                public.verify(b"m", &[1u8; 63]),
                Err(SignatureError::Malformed)
            );
        }
    }

    /// Replaces `s` by `n - s`, the other valid ECDSA signature.
    fn flip_s(algorithm: Algorithm, sig: &[u8; 64]) -> [u8; 64] {
        let order: [u8; 32] = match algorithm {
            Algorithm::Secp256k1 => HEXLOWER
                .decode(b"fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141")
                .unwrap()
                .try_into()
                .unwrap(),
            Algorithm::P256 => HEXLOWER
                .decode(b"ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551")
                .unwrap()
                .try_into()
                .unwrap(),
        };
        let mut out = *sig;
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let diff = i16::from(order[i]) - i16::from(sig[32 + i]) - borrow;
            borrow = i16::from(diff < 0);
            out[32 + i] = u8::try_from(diff.rem_euclid(256)).unwrap();
        }
        out
    }

    #[test]
    fn keys_round_trip_and_bad_keys_fail() {
        for algorithm in [Algorithm::Secp256k1, Algorithm::P256] {
            let pair = Keypair::generate(algorithm, counter_fill());
            let public = pair.public_key();
            assert_eq!(PublicKey::from_did_key(&pair.did_key()), Ok(public));
            assert_eq!(PublicKey::from_multikey(&public.to_multikey()), Ok(public));
            assert_eq!(
                PublicKey::from_sec1_bytes(algorithm, &public.to_compressed()),
                Ok(public)
            );
            let again = Keypair::from_secret_bytes(algorithm, pair.secret_bytes().as_slice());
            assert_eq!(again.unwrap().did_key(), pair.did_key());
            assert!(format!("{pair:?}").contains("secret redacted"));
        }
        assert_eq!(
            PublicKey::from_did_key("did:plc:abc"),
            Err(KeyError::NotDidKey)
        );
        assert_eq!(
            PublicKey::from_did_key("did:key:mAQID"),
            Err(KeyError::Multibase)
        );
        assert_eq!(
            PublicKey::from_did_key(&format!("did:key:z{}", "1".repeat(200))),
            Err(KeyError::Multibase)
        );
        // An Ed25519 did:key.
        assert_eq!(
            PublicKey::from_did_key("did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"),
            Err(KeyError::UnsupportedType)
        );
        // The right prefix over bytes that are no point: an x coordinate
        // above the field prime.
        let mut bytes = Algorithm::P256.multicodec().to_vec();
        bytes.push(2);
        bytes.extend_from_slice(&[0xff; 32]);
        let bad = multibase::encode(multibase::Base::Base58Btc, bytes);
        assert!(PublicKey::from_multikey(&bad).is_err());
        assert!(Keypair::from_secret_bytes(Algorithm::Secp256k1, &[0u8; 32]).is_err());
        assert!(Keypair::from_secret_bytes(Algorithm::Secp256k1, &[1u8; 31]).is_err());
    }
}
