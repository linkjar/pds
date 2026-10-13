//! Password hashes, in the Reference's format
//! (`account-manager/helpers/scrypt.ts` at the pin).
//!
//! A stored hash is `<salt>:<hash>`. The hash is scrypt with Node's
//! defaults (N = 16384, r = 8, p = 1) and 64 bytes of output, in hex. The
//! salt goes into scrypt as the text it is stored as, not as the bytes
//! that text encodes.

use data_encoding::HEXLOWER;
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;

/// The longest password a session is opened with.
pub const OLD_PASSWORD_MAX_LENGTH: usize = 512;
/// The longest password an account is given.
pub const NEW_PASSWORD_MAX_LENGTH: usize = 256;

fn derive(password: &str, salt: &str) -> Option<String> {
    // log2(16384) = 14.
    let params = scrypt::Params::new(14, 8, 1).ok()?;
    let mut out = [0_u8; 64];
    scrypt::scrypt(password.as_bytes(), salt.as_bytes(), &params, &mut out).ok()?;
    Some(HEXLOWER.encode(&out))
}

/// The stored hash of `password` with `salt`.
pub fn hash_with_salt(password: &str, salt: &str) -> Option<String> {
    Some(format!("{salt}:{}", derive(password, salt)?))
}

/// Whether `password` is the one that `stored` was made from.
pub fn verify(password: &str, stored: &str) -> bool {
    let Some((salt, hash)) = stored.split_once(':') else {
        return false;
    };
    derive(password, salt)
        .is_some_and(|derived| bool::from(derived.as_bytes().ct_eq(hash.as_bytes())))
}

/// The stored hash of an app password. Its salt is a function of the DID,
/// so the hash finds its row.
pub fn hash_app_password(did: &str, password: &str) -> Option<String> {
    let digest: [u8; 32] = Sha256::digest(did.as_bytes()).into();
    let salt = HEXLOWER.encode(digest.get(..16)?);
    hash_with_salt(password, &salt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hash that Node's `crypto.scrypt(password, salt, 64)` gives.
    #[test]
    fn the_hash_is_nodes() {
        // RFC 7914, section 12, third vector: the same parameters.
        let derived = derive("pleaseletmein", "SodiumChloride").unwrap();
        assert_eq!(
            derived,
            "7023bdcb3afd7348461c06cd81fd38ebfda8fbba904f8e3ea9b543f6545da1f2\
             d5432955613f0fcf62d49705242a9af9e61e85dc0d651e40dfcf017b45575887"
        );
        let stored = hash_with_salt("pleaseletmein", "SodiumChloride").unwrap();
        assert!(verify("pleaseletmein", &stored));
        assert!(!verify("pleaseletmeout", &stored));
        assert!(!verify("pleaseletmein", "no colon"));
        let app = hash_app_password("did:plc:abc", "abcd-efgh").unwrap();
        assert_eq!(app.split_once(':').unwrap().0.len(), 32);
        assert_eq!(app, hash_app_password("did:plc:abc", "abcd-efgh").unwrap());
    }
}
