//! The tokens of the authorization server (`docs/SPEC.md` §11.3).
//!
//! Access tokens are HS256 JWTs under `PDS_JWT_SECRET`, as the Reference's
//! are, so a token from before the cutover verifies after it. Everything
//! else is a random identifier with a prefix.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::dpop::{jwk_thumbprint, key_of};

/// An access token lives 60 minutes.
pub const TOKEN_MAX_AGE_MS: i64 = 60 * 60 * 1000;
/// A pushed request lives 5 minutes, and 5 more after each step.
pub const PAR_EXPIRES_IN_MS: i64 = 5 * 60 * 1000;
/// A session of a public client lives 2 weeks.
pub const SESSION_LIFETIME_MS: i64 = 14 * 24 * 60 * 60 * 1000;
/// A refresh token of a public client is good for 2 weeks.
pub const REFRESH_LIFETIME_MS: i64 = SESSION_LIFETIME_MS;
/// A session of a confidential or first-party client lives 2 years.
pub const SESSION_LIFETIME_EXTENDED_MS: i64 = 2 * 31_557_600_000;
/// A refresh token of a confidential or first-party client is good for 3
/// months.
pub const REFRESH_LIFETIME_EXTENDED_MS: i64 = 31_557_600_000 / 4;
/// A sign-in on a device is good for 7 days.
pub const AUTHENTICATION_MAX_AGE_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// A sign-in that the device does not remember is good for 15 minutes.
pub const EPHEMERAL_SESSION_MAX_AGE_S: i64 = 15 * 60;
/// A client assertion is at most 1 minute old.
pub const CLIENT_ASSERTION_MAX_AGE_S: i64 = 60;
/// A code challenge is refused for reuse for 1 day.
pub const CODE_CHALLENGE_REPLAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The assertion type of `private_key_jwt`.
pub const ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";
/// The prefix of a request URI.
pub const REQUEST_URI_PREFIX: &str = "urn:ietf:params:oauth:request_uri:";

/// The kinds of identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Id {
    /// A device: `dev-` and 16 bytes.
    Device,
    /// A device session: `ses-` and 16 bytes.
    Session,
    /// A pushed request: `req-` and 16 bytes.
    Request,
    /// An access token's `jti`: `tok-` and 16 bytes.
    Token,
    /// An authorization code: `cod-` and 32 bytes.
    Code,
    /// A refresh token: `ref-` and 32 bytes.
    Refresh,
}

impl Id {
    const fn parts(self) -> (&'static str, usize) {
        match self {
            Self::Device => ("dev-", 16),
            Self::Session => ("ses-", 16),
            Self::Request => ("req-", 16),
            Self::Token => ("tok-", 16),
            Self::Code => ("cod-", 32),
            Self::Refresh => ("ref-", 32),
        }
    }

    /// How many random bytes the identifier holds.
    pub const fn bytes(self) -> usize {
        self.parts().1
    }

    /// The identifier over `random`, which holds [`Id::bytes`] bytes.
    pub fn format(self, random: &[u8]) -> String {
        use std::fmt::Write;
        let (prefix, _) = self.parts();
        let mut out = String::from(prefix);
        for byte in random {
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    /// Whether `text` has the shape of this identifier.
    pub fn matches(self, text: &str) -> bool {
        let (prefix, bytes) = self.parts();
        text.strip_prefix(prefix)
            .is_some_and(|hex| hex.len() == bytes * 2 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
    }
}

fn sign(secret: &[u8], header: &Value, claims: &Value) -> String {
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let signature = Hmac::<Sha256>::new_from_slice(secret)
        .map(|mut mac| {
            mac.update(input.as_bytes());
            mac.finalize().into_bytes().to_vec()
        })
        .unwrap_or_default();
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature))
}

fn verify(secret: &[u8], token: &str, typ: &str) -> Option<Value> {
    let mut parts = token.split('.');
    let (Some(head), Some(body), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let json = |part: &str| -> Option<Value> {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).ok()?).ok()
    };
    let header = json(head)?;
    if header.get("alg").and_then(Value::as_str) != Some("HS256")
        || header.get("typ").and_then(Value::as_str) != Some(typ)
    {
        return None;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).ok()?;
    mac.update(head.as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    mac.verify_slice(&URL_SAFE_NO_PAD.decode(signature).ok()?)
        .ok()?;
    json(body)
}

/// The claims of an access token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessClaims {
    /// The token row: `tok-…`.
    pub jti: String,
    /// The account.
    pub sub: String,
    /// The issue time, in seconds.
    pub iat: i64,
    /// The end time, in seconds.
    pub exp: i64,
    /// The thumbprint of the DPoP key.
    pub jkt: Option<String>,
    /// The client.
    pub client_id: String,
}

/// Signs an access token. `aud` is the service DID and `iss` the issuer.
pub fn mint_access(secret: &[u8], issuer: &str, aud: &str, claims: &AccessClaims) -> String {
    let mut body = json!({
        "jti": claims.jti,
        "sub": claims.sub,
        "iat": claims.iat,
        "exp": claims.exp,
        "aud": aud,
        "client_id": claims.client_id,
        "iss": issuer,
    });
    if let Some(jkt) = &claims.jkt {
        body["cnf"] = json!({ "jkt": jkt });
    }
    sign(secret, &json!({ "alg": "HS256", "typ": "at+jwt" }), &body)
}

/// Reads an access token this server signed. The expiry is the caller's to
/// check, against the token's row.
pub fn verify_access(secret: &[u8], issuer: &str, token: &str) -> Option<AccessClaims> {
    let body = verify(secret, token, "at+jwt")?;
    if body.get("iss").and_then(Value::as_str) != Some(issuer) {
        return None;
    }
    claims_of(&body)
}

fn claims_of(body: &Value) -> Option<AccessClaims> {
    let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(AccessClaims {
        jti: text("jti")?,
        sub: text("sub")?,
        iat: body.get("iat").and_then(Value::as_i64)?,
        exp: body.get("exp").and_then(Value::as_i64)?,
        jkt: body
            .get("cnf")
            .and_then(|cnf| cnf.get("jkt"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        client_id: text("client_id")?,
    })
}

/// Signs an access token of an entryway with its secp256k1 key (SPEC
/// §21.3). The token carries its scope, because a member host has no row
/// to read it from.
pub fn mint_access_es256k(
    key: &pds_identity::Keypair,
    issuer: &str,
    aud: &str,
    claims: &AccessClaims,
    scope: &str,
) -> String {
    let mut body = json!({
        "jti": claims.jti,
        "sub": claims.sub,
        "iat": claims.iat,
        "exp": claims.exp,
        "aud": aud,
        "client_id": claims.client_id,
        "iss": issuer,
        "scope": scope,
    });
    if let Some(jkt) = &claims.jkt {
        body["cnf"] = json!({ "jkt": jkt });
    }
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(json!({ "alg": "ES256K", "typ": "at+jwt" }).to_string()),
        URL_SAFE_NO_PAD.encode(body.to_string())
    );
    format!(
        "{input}.{}",
        URL_SAFE_NO_PAD.encode(key.sign(input.as_bytes()))
    )
}

/// Reads an access token that the entryway `issuer` signed with `key`.
/// Gives the claims and the scope. The expiry is the caller's to check.
pub fn verify_access_es256k(
    key: &pds_identity::PublicKey,
    issuer: &str,
    token: &str,
) -> Option<(AccessClaims, String)> {
    let mut parts = token.split('.');
    let (Some(head), Some(body), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let json = |part: &str| -> Option<Value> {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).ok()?).ok()
    };
    let header = json(head)?;
    if header.get("alg").and_then(Value::as_str) != Some("ES256K")
        || header.get("typ").and_then(Value::as_str) != Some("at+jwt")
    {
        return None;
    }
    key.verify_any_s(
        &token.as_bytes()[..head.len() + 1 + body.len()],
        &URL_SAFE_NO_PAD.decode(signature).ok()?,
    )
    .ok()?;
    let body = json(body)?;
    if body.get("iss").and_then(Value::as_str) != Some(issuer) {
        return None;
    }
    let scope = body.get("scope").and_then(Value::as_str)?.to_owned();
    Some((claims_of(&body)?, scope))
}

/// Signs the proof of a sign-in that the device does not remember. It
/// names the account, the device and the request, and travels in the form
/// of the consent page.
pub fn mint_ephemeral(secret: &[u8], did: &str, device: &str, request: &str, now_s: i64) -> String {
    sign(
        secret,
        &json!({ "alg": "HS256", "typ": "linkjar-ephemeral+jwt" }),
        &json!({
            "sub": did,
            "dev": device,
            "req": request,
            "exp": now_s + EPHEMERAL_SESSION_MAX_AGE_S,
        }),
    )
}

/// The account of a sign-in proof, when the proof is for this device and
/// this request and has not ended.
pub fn verify_ephemeral(
    secret: &[u8],
    token: &str,
    device: &str,
    request: &str,
    now_s: i64,
) -> Option<String> {
    let body = verify(secret, token, "linkjar-ephemeral+jwt")?;
    let text = |key: &str| body.get(key).and_then(Value::as_str);
    (text("dev") == Some(device)
        && text("req") == Some(request)
        && body.get("exp").and_then(Value::as_i64)? > now_s)
        .then(|| text("sub").map(str::to_owned))?
}

/// A client assertion that verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assertion {
    /// The key that signed it.
    pub kid: String,
    /// Its algorithm.
    pub alg: String,
    /// The thumbprint of the key.
    pub jkt: String,
    /// The assertion's identifier, good once.
    pub jti: String,
    /// When the assertion ends, in seconds.
    pub exp: i64,
}

/// Checks a `private_key_jwt` assertion against the client's key set.
pub fn check_assertion(
    assertion: &str,
    jwks: &Value,
    client_id: &str,
    issuer: &str,
    now_s: i64,
) -> Result<Assertion, String> {
    let failed = |what: &str| format!("Validation of \"client_assertion\" failed: {what}");
    let parts: Vec<&str> = assertion.split('.').collect();
    let [head, body, signature] = parts.as_slice() else {
        return Err(failed("Invalid Compact JWS"));
    };
    let json = |part: &str| -> Option<Value> {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).ok()?).ok()
    };
    let header = json(head).ok_or_else(|| failed("Invalid Compact JWS"))?;
    let claims = json(body).ok_or_else(|| failed("Invalid Compact JWS"))?;
    let kid = header
        .get("kid")
        .and_then(Value::as_str)
        .ok_or_else(|| failed("\"kid\" required in client_assertion"))?;
    let alg = header
        .get("alg")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let jwk = jwks
        .get("keys")
        .and_then(Value::as_array)
        .and_then(|keys| {
            keys.iter()
                .find(|key| key.get("kid").and_then(Value::as_str) == Some(kid))
        })
        .ok_or_else(|| failed(&format!("No key matching kid \"{kid}\"")))?;
    if jwk.get("revoked") == Some(&Value::Bool(true)) {
        return Err(failed(&format!("The key \"{kid}\" is revoked")));
    }
    let (key, key_alg) = key_of(jwk).ok_or_else(|| failed("unsupported key"))?;
    if alg != key_alg {
        return Err(failed("the \"alg\" does not fit the key"));
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| failed("signature verification failed"))?;
    key.verify_any_s(
        &assertion.as_bytes()[..head.len() + 1 + body.len()],
        &signature,
    )
    .map_err(|_| failed("signature verification failed"))?;
    let text = |name: &str| claims.get(name).and_then(Value::as_str);
    if text("iss") != Some(client_id) || text("sub") != Some(client_id) {
        return Err(failed("unexpected \"iss\" or \"sub\" claim value"));
    }
    let audience_ok = match claims.get("aud") {
        Some(Value::String(aud)) => aud == issuer,
        Some(Value::Array(list)) => list.iter().any(|aud| aud.as_str() == Some(issuer)),
        _ => false,
    };
    if !audience_ok {
        return Err(failed("unexpected \"aud\" claim value"));
    }
    let jti = text("jti").ok_or_else(|| failed("\"jti\" claim is required"))?;
    let exp = claims.get("exp").and_then(Value::as_i64);
    if exp.is_some_and(|exp| exp <= now_s) {
        return Err(failed("\"exp\" claim timestamp check failed"));
    }
    let iat = claims
        .get("iat")
        .and_then(Value::as_i64)
        .ok_or_else(|| failed("\"iat\" claim must be a number"))?;
    if now_s - iat > CLIENT_ASSERTION_MAX_AGE_S {
        return Err(failed(
            "\"iat\" claim timestamp check failed (too far in the past)",
        ));
    }
    Ok(Assertion {
        kid: kid.to_owned(),
        alg: alg.to_owned(),
        jkt: jwk_thumbprint(jwk).ok_or_else(|| failed("Failed to calculate jkt"))?,
        jti: jti.to_owned(),
        exp: exp.unwrap_or(iat + CLIENT_ASSERTION_MAX_AGE_S),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_have_the_shape_of_the_reference() {
        let code = Id::Code.format(&[0xab; 32]);
        assert_eq!(code.len(), 4 + 64);
        assert!(Id::Code.matches(&code));
        assert!(!Id::Refresh.matches(&code));
        assert!(Id::Request.matches(&Id::Request.format(&[7; 16])));
        assert!(!Id::Request.matches("req-zz"));
    }

    #[test]
    fn an_access_token_round_trips_and_a_changed_one_does_not() {
        let claims = AccessClaims {
            jti: Id::Token.format(&[1; 16]),
            sub: "did:plc:abc".to_owned(),
            iat: 100,
            exp: 3700,
            jkt: Some("thumb".to_owned()),
            client_id: "https://app.example.org/client-metadata.json".to_owned(),
        };
        let token = mint_access(b"secret", "https://pds.test", "did:web:pds.test", &claims);
        assert_eq!(
            verify_access(b"secret", "https://pds.test", &token),
            Some(claims)
        );
        assert_eq!(verify_access(b"other", "https://pds.test", &token), None);
        assert_eq!(verify_access(b"secret", "https://other.test", &token), None);
        let mut changed = token.clone();
        changed.insert(token.find('.').unwrap_or_default() + 2, 'x');
        assert_eq!(verify_access(b"secret", "https://pds.test", &changed), None);
    }

    #[test]
    fn a_sign_in_proof_is_for_one_device_and_one_request() {
        let proof = mint_ephemeral(b"secret", "did:plc:abc", "dev-1", "req-1", 1000);
        let check =
            |device, request, now| verify_ephemeral(b"secret", &proof, device, request, now);
        assert_eq!(
            check("dev-1", "req-1", 1001).as_deref(),
            Some("did:plc:abc")
        );
        assert_eq!(check("dev-2", "req-1", 1001), None);
        assert_eq!(check("dev-1", "req-2", 1001), None);
        assert_eq!(
            check("dev-1", "req-1", 1000 + EPHEMERAL_SESSION_MAX_AGE_S),
            None
        );
    }
}
