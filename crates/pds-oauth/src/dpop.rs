//! DPoP proofs (RFC 9449), as `dpop-manager.ts` and `dpop-nonce.ts` of
//! `@atproto/oauth-provider` at the pin check them.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit as _, Mac as _};
use pds_identity::{Algorithm, PublicKey};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// How old a nonce may be: 3 minutes. It also bounds the clock difference
/// that a proof's `iat` may show.
pub const NONCE_MAX_AGE_MS: i64 = 3 * 60 * 1000;
/// How long a proof is valid after its `iat`: 10 seconds, before the
/// allowance for clocks.
const PROOF_MAX_AGE_S: i64 = 10;

/// Why a proof is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DpopError {
    /// The proof is not one: `invalid_dpop_proof`.
    #[error("{0}")]
    Invalid(String),
    /// The proof lacks a nonce, or carries an old one: `use_dpop_nonce`.
    /// The client asks again with the nonce of the answer.
    #[error("{0}")]
    UseNonce(String),
}

fn invalid(message: &str) -> DpopError {
    DpopError::Invalid(message.to_owned())
}

/// A proof that verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    /// The id of the proof. A proof is used once.
    pub jti: String,
    /// The thumbprint of the key that signed it. A token is bound to it.
    pub jkt: String,
}

/// The thumbprint of a public key in JWK form (RFC 7638): the SHA-256 of
/// its required members in lexical order, in base64url.
pub fn jwk_thumbprint(jwk: &Value) -> Option<String> {
    let text = |key: &str| jwk.get(key)?.as_str();
    let canonical = match text("kty")? {
        "EC" => format!(
            r#"{{"crv":{},"kty":"EC","x":{},"y":{}}}"#,
            Value::from(text("crv")?),
            Value::from(text("x")?),
            Value::from(text("y")?)
        ),
        _ => return None,
    };
    let digest: [u8; 32] = Sha256::digest(canonical.as_bytes()).into();
    Some(URL_SAFE_NO_PAD.encode(digest))
}

/// The public key of a JWK and the algorithm it signs with. A JWK with a
/// private part is refused.
pub(crate) fn key_of(jwk: &Value) -> Option<(PublicKey, &'static str)> {
    if jwk.get("d").is_some() || jwk.get("kty")?.as_str()? != "EC" {
        return None;
    }
    let (algorithm, alg) = match jwk.get("crv")?.as_str()? {
        "P-256" => (Algorithm::P256, "ES256"),
        "secp256k1" => (Algorithm::Secp256k1, "ES256K"),
        _ => return None,
    };
    let coordinate = |key: &str| -> Option<Vec<u8>> {
        let bytes = URL_SAFE_NO_PAD.decode(jwk.get(key)?.as_str()?).ok()?;
        (bytes.len() == 32).then_some(bytes)
    };
    let mut sec1 = vec![4_u8];
    sec1.extend(coordinate("x")?);
    sec1.extend(coordinate("y")?);
    Some((PublicKey::from_sec1_bytes(algorithm, &sec1).ok()?, alg))
}

/// The URL a proof is for: scheme, host and path, without query and
/// fragment.
fn normalized(url: &str) -> Result<String, DpopError> {
    let url = url::Url::parse(url).map_err(|_| invalid("DPoP \"htu\" is not a valid URL"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("DPoP \"htu\" must not contain credentials"));
    }
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(invalid("DPoP \"htu\" must be http or https"));
    }
    Ok(format!(
        "{}{}",
        url.origin().ascii_serialization(),
        url.path()
    ))
}

/// What a proof is checked against.
#[derive(Debug, Clone, Copy)]
pub struct Expect<'a> {
    /// The method of the request.
    pub method: &'a str,
    /// The URL of the request as the client names it.
    pub url: &'a str,
    /// The access token of the request, when it carries one.
    pub access_token: Option<&'a str>,
    /// The time in seconds since the epoch.
    pub now_s: i64,
}

/// Checks a proof. `nonce` answers whether a nonce is one the server
/// issued lately; `None` means the server asks for no nonce.
pub fn check_proof(
    proof: &str,
    expect: Expect<'_>,
    nonce: Option<&dyn Fn(&str) -> bool>,
) -> Result<Proof, DpopError> {
    let failed = |what: &str| DpopError::Invalid(format!("Failed to verify DPoP proof: {what}"));
    let parts: Vec<&str> = proof.split('.').collect();
    let [head, body, signature] = parts.as_slice() else {
        return Err(failed("Invalid Compact JWS"));
    };
    let json = |part: &str| -> Option<Value> {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).ok()?).ok()
    };
    let header = json(head).ok_or_else(|| failed("Invalid Compact JWS"))?;
    let claims = json(body).ok_or_else(|| failed("Invalid Compact JWS"))?;
    if header.get("typ").and_then(Value::as_str) != Some("dpop+jwt") {
        return Err(failed("unexpected \"typ\" JWT header value"));
    }
    let jwk = header
        .get("jwk")
        .ok_or_else(|| failed("\"jwk\" (JSON Web Key) Header Parameter must be a JSON object"))?;
    let (key, alg) = key_of(jwk).ok_or_else(|| failed("unsupported or private key"))?;
    if header.get("alg").and_then(Value::as_str) != Some(alg) {
        return Err(failed("the \"alg\" does not fit the key"));
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| failed("signature verification failed"))?;
    key.verify_any_s(format!("{head}.{body}").as_bytes(), &signature)
        .map_err(|_| failed("signature verification failed"))?;
    let iat = claims
        .get("iat")
        .and_then(Value::as_i64)
        .ok_or_else(|| failed("\"iat\" claim must be a number"))?;
    let tolerance = NONCE_MAX_AGE_MS / 1000;
    if expect.now_s - iat > PROOF_MAX_AGE_S + tolerance {
        return Err(failed(
            "\"iat\" claim timestamp check failed (too far in the past)",
        ));
    }
    if iat - expect.now_s > tolerance {
        return Err(failed(
            "\"iat\" claim timestamp check failed (it should be in the past)",
        ));
    }

    let given = claims.get("nonce");
    if given.is_some_and(|nonce| !nonce.is_string()) {
        return Err(invalid("Invalid DPoP \"nonce\" type"));
    }
    let jti = claims
        .get("jti")
        .and_then(Value::as_str)
        .filter(|jti| !jti.is_empty())
        .ok_or_else(|| invalid("DPoP \"jti\" missing"))?;
    if claims.get("htm").and_then(Value::as_str) != Some(expect.method) {
        return Err(invalid("DPoP \"htm\" mismatch"));
    }
    let htu = claims
        .get("htu")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("Invalid DPoP \"htu\" type"))?;
    if normalized(htu)? != normalized(expect.url)? {
        return Err(invalid("DPoP \"htu\" mismatch"));
    }
    if let Some(known) = nonce {
        match given
            .and_then(Value::as_str)
            .filter(|nonce| !nonce.is_empty())
        {
            None => return Err(DpopError::UseNonce("DPoP nonce required".to_owned())),
            Some(nonce) if !known(nonce) => {
                return Err(DpopError::UseNonce("DPoP \"nonce\" mismatch".to_owned()));
            }
            Some(_) => {}
        }
    }
    let ath = claims.get("ath");
    match expect.access_token {
        Some(token) => {
            let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
            if ath.and_then(Value::as_str) != Some(URL_SAFE_NO_PAD.encode(digest).as_str()) {
                return Err(invalid("DPoP \"ath\" mismatch"));
            }
        }
        None if ath.is_some() => return Err(invalid("DPoP \"ath\" claim not allowed")),
        None => {}
    }
    let jkt = jwk_thumbprint(jwk).ok_or_else(|| invalid("Failed to calculate jkt"))?;
    Ok(Proof {
        jti: jti.to_owned(),
        jkt,
    })
}

/// The server's nonces: one for each interval, computed from a secret, so
/// that every process with the secret knows them and none is stored.
#[derive(Clone)]
pub struct Nonces {
    secret: [u8; 32],
    interval_ms: i64,
}

impl std::fmt::Debug for Nonces {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Nonces").finish_non_exhaustive()
    }
}

impl Nonces {
    /// The nonces of `secret` (`PDS_DPOP_SECRET`). A nonce changes every
    /// minute and is taken for three.
    pub const fn new(secret: [u8; 32]) -> Self {
        Self {
            secret,
            interval_ms: NONCE_MAX_AGE_MS / 3,
        }
    }

    fn compute(&self, counter: i64) -> String {
        // HMAC takes a key of any length, so there is no other arm.
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&self.secret) else {
            return String::new();
        };
        mac.update(&counter.to_be_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    /// The nonce a client should use next.
    pub fn next(&self, now_ms: i64) -> String {
        self.compute(now_ms / self.interval_ms + 1)
    }

    /// Whether `nonce` is the next, the current or the previous one.
    pub fn check(&self, nonce: &str, now_ms: i64) -> bool {
        let counter = now_ms / self.interval_ms;
        (counter - 1..=counter + 1).any(|counter| self.compute(counter) == nonce)
    }
}

#[cfg(test)]
mod tests {
    use pds_identity::Keypair;

    use super::*;

    fn jwk(keypair: &Keypair) -> Value {
        let point = keypair.public_key().to_uncompressed();
        serde_json::json!({
            "kty": "EC", "crv": "P-256",
            "x": URL_SAFE_NO_PAD.encode(&point[1..33]),
            "y": URL_SAFE_NO_PAD.encode(&point[33..65]),
        })
    }

    fn proof(keypair: &Keypair, claims: &Value) -> String {
        let header = serde_json::json!({"typ": "dpop+jwt", "alg": "ES256", "jwk": jwk(keypair)});
        let signed = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let signature = keypair.sign(signed.as_bytes());
        format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature))
    }

    /// The example key of RFC 7638 is RSA; this is the EC example of
    /// RFC 9449, section 4.
    #[test]
    fn a_thumbprint_is_the_one_of_rfc_9449() {
        let jwk = serde_json::json!({
            "kty": "EC", "crv": "P-256",
            "x": "l8tFrhx-34tV3hRICRDY9zCkDlpBhF42UQUfWVAWBFs",
            "y": "9VE4jf_Ok_o64zbTTlcuNJajHmt6v9TDVrU0CdvGRDA",
        });
        assert_eq!(
            jwk_thumbprint(&jwk).unwrap(),
            "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I"
        );
    }

    #[test]
    fn a_proof_is_checked_claim_by_claim() {
        let keypair = Keypair::generate(Algorithm::P256, |bytes| bytes.fill(3));
        let nonces = Nonces::new([9; 32]);
        let now_s = 1_760_000_000;
        let nonce = nonces.next(now_s * 1000);
        let known = |given: &str| nonces.check(given, now_s * 1000);
        let expect = Expect {
            method: "POST",
            url: "https://pds.test/oauth/token?x=1",
            access_token: None,
            now_s,
        };
        let claims = |change: &dyn Fn(&mut Value)| {
            let mut claims = serde_json::json!({
                "jti": "one", "htm": "POST", "htu": "https://pds.test/oauth/token",
                "iat": now_s, "nonce": nonce,
            });
            change(&mut claims);
            claims
        };
        let check = |claims: &Value| check_proof(&proof(&keypair, claims), expect, Some(&known));

        let good = check(&claims(&|_| {})).unwrap();
        assert_eq!(good.jti, "one");
        assert_eq!(good.jkt, jwk_thumbprint(&jwk(&keypair)).unwrap());

        for (change, message) in [
            (
                Box::new(|c: &mut Value| c["htm"] = "GET".into()) as Box<dyn Fn(&mut Value)>,
                "DPoP \"htm\" mismatch",
            ),
            (
                Box::new(|c: &mut Value| c["htu"] = "https://other.test/oauth/token".into()),
                "DPoP \"htu\" mismatch",
            ),
            (
                Box::new(|c: &mut Value| {
                    c.as_object_mut().unwrap().remove("jti");
                }),
                "DPoP \"jti\" missing",
            ),
            (
                Box::new(|c: &mut Value| c["ath"] = "x".into()),
                "DPoP \"ath\" claim not allowed",
            ),
        ] {
            assert_eq!(
                check(&claims(&*change)),
                Err(DpopError::Invalid(message.to_owned()))
            );
        }
        assert!(matches!(
            check(&claims(&|c| {
                c.as_object_mut().unwrap().remove("nonce");
            })),
            Err(DpopError::UseNonce(_))
        ));
        assert!(matches!(
            check(&claims(&|c| c["nonce"] = "stale".into())),
            Err(DpopError::UseNonce(_))
        ));
        assert!(matches!(
            check(&claims(&|c| c["iat"] = (now_s - 400).into())),
            Err(DpopError::Invalid(_))
        ));

        // A proof for a request with a token names the token's hash.
        let digest: [u8; 32] = Sha256::digest(b"token").into();
        let bound = claims(&|c| c["ath"] = URL_SAFE_NO_PAD.encode(digest).into());
        let with_token = Expect {
            access_token: Some("token"),
            ..expect
        };
        assert!(check_proof(&proof(&keypair, &bound), with_token, Some(&known)).is_ok());
        assert_eq!(
            check_proof(&proof(&keypair, &claims(&|_| {})), with_token, Some(&known)),
            Err(DpopError::Invalid("DPoP \"ath\" mismatch".to_owned()))
        );

        // Another key's signature, and a nonce from another minute.
        let other = Keypair::generate(Algorithm::P256, |bytes| bytes.fill(4));
        let mut forged = proof(&other, &claims(&|_| {}));
        let own = proof(&keypair, &claims(&|_| {}));
        forged = format!(
            "{}.{}",
            own.rsplit_once('.').unwrap().0,
            forged.rsplit_once('.').unwrap().1
        );
        assert!(matches!(
            check_proof(&forged, expect, Some(&known)),
            Err(DpopError::Invalid(_))
        ));
        assert!(nonces.check(&nonce, now_s * 1000 + 60_000));
        assert!(!nonces.check(&nonce, now_s * 1000 + 200_000));
    }
}
