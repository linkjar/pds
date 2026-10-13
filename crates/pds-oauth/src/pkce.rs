//! Proof Key for Code Exchange with `S256` (RFC 7636), the one method the
//! server takes (`docs/SPEC.md` §11.1).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};

/// Whether `verifier` is the code verifier of `challenge`. A verifier is
/// 43 to 128 unreserved characters.
pub fn verify_s256(verifier: &str, challenge: &str) -> bool {
    let shape = (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b));
    let digest: [u8; 32] = Sha256::digest(verifier.as_bytes()).into();
    shape && URL_SAFE_NO_PAD.encode(digest) == challenge
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example of RFC 7636, appendix B.
    #[test]
    fn the_rfc_example_verifies() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(verify_s256(verifier, challenge));
        assert!(!verify_s256(
            verifier,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cN"
        ));
        assert!(!verify_s256("short", challenge));
    }
}
