//! The client half of OAuth with DPoP: what a client of an atproto
//! authorization server signs. The two sidecars use it (`docs/SPEC.md`
//! §22).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pds_identity::Keypair;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The public JWK of a P-256 key.
pub fn public_jwk(key: &Keypair) -> Value {
    let point = key.public_key().to_uncompressed();
    json!({
        "kty": "EC",
        "crv": "P-256",
        "x": URL_SAFE_NO_PAD.encode(&point[1..33]),
        "y": URL_SAFE_NO_PAD.encode(&point[33..65]),
    })
}

fn sign(key: &Keypair, header: &Value, claims: &Value) -> String {
    let signed = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    format!(
        "{signed}.{}",
        URL_SAFE_NO_PAD.encode(key.sign(signed.as_bytes()))
    )
}

/// What a DPoP proof says of its request (RFC 9449).
#[derive(Debug, Clone, Copy)]
pub struct ProofOf<'a> {
    /// The HTTP method.
    pub method: &'a str,
    /// The address the server knows itself by, without the query.
    pub htu: &'a str,
    /// The nonce the server gave last.
    pub nonce: Option<&'a str>,
    /// The access token of the request.
    pub token: Option<&'a str>,
    /// The time, in seconds.
    pub now_s: u64,
    /// An identifier that is used once.
    pub jti: &'a str,
}

/// The DPoP proof of a request, signed with a P-256 key.
pub fn dpop_proof(key: &Keypair, of: ProofOf<'_>) -> String {
    let header = json!({ "typ": "dpop+jwt", "alg": "ES256", "jwk": public_jwk(key) });
    let mut claims = json!({
        "jti": of.jti,
        "htm": of.method,
        "htu": of.htu,
        "iat": of.now_s,
    });
    if let Some(nonce) = of.nonce {
        claims["nonce"] = json!(nonce);
    }
    if let Some(token) = of.token {
        claims["ath"] = json!(URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes())));
    }
    sign(key, &header, &claims)
}

/// The `private_key_jwt` assertion of a confidential client (RFC 7523),
/// signed with the P-256 key that the client's key set names `kid`. It is
/// good for one minute.
pub fn client_assertion(
    key: &Keypair,
    kid: &str,
    client_id: &str,
    issuer: &str,
    now_s: u64,
    jti: &str,
) -> String {
    let header = json!({ "typ": "JWT", "alg": "ES256", "kid": kid });
    let claims = json!({
        "iss": client_id,
        "sub": client_id,
        "aud": issuer,
        "jti": jti,
        "iat": now_s,
        "exp": now_s + 60,
    });
    sign(key, &header, &claims)
}

#[cfg(test)]
mod tests {
    use pds_identity::Algorithm;

    use super::*;

    fn key() -> Keypair {
        Keypair::generate(Algorithm::P256, |bytes| bytes.fill(5))
    }

    fn part(jwt: &str, index: usize) -> Value {
        let part = jwt.split('.').nth(index).unwrap();
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
    }

    #[test]
    fn a_proof_is_one_that_the_server_half_accepts() {
        let key = key();
        let of = ProofOf {
            method: "POST",
            htu: "https://pds.example/oauth/par",
            nonce: Some("n-1"),
            token: Some("tok"),
            now_s: 1_000,
            jti: "j-1",
        };
        let proof = dpop_proof(&key, of);
        assert_eq!(part(&proof, 0)["typ"], "dpop+jwt");
        let claims = part(&proof, 1);
        assert_eq!(claims["htm"], "POST");
        assert_eq!(claims["nonce"], "n-1");
        assert_eq!(
            claims["ath"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(b"tok")).as_str()
        );
        let expect = crate::dpop::Expect {
            method: "POST",
            url: "https://pds.example/oauth/par",
            now_s: 1_000,
            access_token: Some("tok"),
        };
        let checked = crate::dpop::check_proof(&proof, expect, None).unwrap();
        assert_eq!(checked.jti, "j-1");
        assert_eq!(
            Some(checked.jkt),
            crate::dpop::jwk_thumbprint(&public_jwk(&key))
        );
    }

    #[test]
    fn an_assertion_is_one_that_the_server_half_accepts() {
        let key = key();
        let mut jwk = public_jwk(&key);
        jwk["kid"] = json!("key-1");
        let jwks = json!({ "keys": [jwk] });
        let id = "https://mcp.example.com/oauth-client-metadata.json";
        let assertion = client_assertion(&key, "key-1", id, "https://pds.example", 1_000, "a-1");
        let checked =
            crate::token::check_assertion(&assertion, &jwks, id, "https://pds.example", 1_010)
                .unwrap();
        assert_eq!(checked.jti, "a-1");
        assert!(
            crate::token::check_assertion(&assertion, &jwks, id, "https://other.example", 1_010)
                .is_err()
        );
    }
}
