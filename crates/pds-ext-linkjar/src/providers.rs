//! Sign-in with Apple, Google and GitHub (patch 099, `docs/SPEC.md` §12.3
//! and §13).
//!
//! This module holds what needs no network: the address a browser is sent
//! to, the request that exchanges a code, and the checks on what a
//! provider answers. The endpoints are fixed. An identity is the pair of
//! provider and subject, never an email address.

use aws_lc_rs::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use url::Url;

/// An ID token is at most 10 minutes old.
const MAX_TOKEN_AGE_S: i64 = 600;

/// A provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderId {
    /// Sign in with Apple.
    Apple,
    /// Google.
    Google,
    /// GitHub.
    GitHub,
}

impl ProviderId {
    /// The three providers, in the order the pages show them.
    pub const ALL: [Self; 3] = [Self::Apple, Self::Google, Self::GitHub];

    /// The name in a path and in a row: `apple`, `google`, `github`.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Apple => "apple",
            Self::Google => "google",
            Self::GitHub => "github",
        }
    }

    /// The name on a button.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Apple => "Apple",
            Self::Google => "Google",
            Self::GitHub => "GitHub",
        }
    }

    /// The provider of a path segment or of a row.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|provider| provider.id() == id)
    }

    const fn authorization(self) -> &'static str {
        match self {
            Self::Apple => "https://appleid.apple.com/auth/authorize",
            Self::Google => "https://accounts.google.com/o/oauth2/v2/auth",
            Self::GitHub => "https://github.com/login/oauth/authorize",
        }
    }

    /// The token endpoint.
    pub const fn token_endpoint(self) -> &'static str {
        match self {
            Self::Apple => "https://appleid.apple.com/auth/token",
            Self::Google => "https://oauth2.googleapis.com/token",
            Self::GitHub => "https://github.com/login/oauth/access_token",
        }
    }

    /// The issuer and the address of the key set of a provider that
    /// answers with an ID token.
    pub const fn oidc(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Apple => Some((
                "https://appleid.apple.com",
                "https://appleid.apple.com/auth/keys",
            )),
            Self::Google => Some((
                "https://accounts.google.com",
                "https://www.googleapis.com/oauth2/v3/certs",
            )),
            Self::GitHub => None,
        }
    }

    const fn scope(self) -> &'static str {
        match self {
            Self::Apple => "name email",
            Self::Google => "openid email profile",
            Self::GitHub => "read:user user:email",
        }
    }
}

/// GitHub's account and email addresses of the token's owner.
pub const GITHUB_USER: &str = "https://api.github.com/user";
/// GitHub's list of the owner's email addresses.
pub const GITHUB_EMAILS: &str = "https://api.github.com/user/emails?per_page=100";

/// A configured provider.
#[derive(Clone)]
pub struct Provider {
    /// Which one.
    pub id: ProviderId,
    /// `PDS_EXTERNAL_<PROVIDER>_CLIENT_ID`.
    pub client_id: String,
    /// `PDS_EXTERNAL_<PROVIDER>_CLIENT_SECRET`.
    pub client_secret: String,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Who a provider says signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The provider.
    pub provider: ProviderId,
    /// The provider's identifier of the person.
    pub subject: String,
    /// The email address, when the provider gave one.
    pub email: Option<String>,
    /// Whether the provider says the address is the person's.
    pub email_verified: bool,
    /// A name to suggest a handle from. It proves nothing.
    pub display_name: Option<String>,
}

/// Why a provider's answer is refused. The text is for the log, not for
/// the page.
pub type Refusal = &'static str;

/// A text of a provider: not empty, bounded, without surrounding space
/// and without control characters.
fn required_text(value: Option<&Value>, max: usize) -> Result<&str, Refusal> {
    let text = value
        .and_then(Value::as_str)
        .ok_or("Invalid external provider value")?;
    if text.is_empty()
        || text.chars().count() > max
        || text.trim() != text
        || text.chars().any(char::is_control)
    {
        return Err("Invalid external provider value");
    }
    Ok(text)
}

fn optional_email(value: Option<&Value>) -> Result<Option<String>, Refusal> {
    match value {
        None | Some(Value::Null) => Ok(None),
        some => {
            let email = required_text(some, 254)?;
            let shaped = email.split_once('@').is_some_and(|(local, domain)| {
                !local.is_empty()
                    && !domain.contains('@')
                    && domain
                        .rsplit_once('.')
                        .is_some_and(|(name, tld)| !name.is_empty() && !tld.is_empty())
                    && !email.chars().any(char::is_whitespace)
            });
            if shaped {
                Ok(Some(email.to_owned()))
            } else {
                Err("Invalid external provider email")
            }
        }
    }
}

fn optional_name(value: Option<&Value>) -> Result<Option<String>, Refusal> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.is_empty() => Ok(None),
        some => Ok(Some(required_text(some, 256)?.to_owned())),
    }
}

fn check_redirect_uri(uri: &str) -> Result<(), Refusal> {
    let url = Url::parse(uri).map_err(|_| "Invalid external provider redirect URI")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("Invalid external provider redirect URI");
    }
    Ok(())
}

impl Provider {
    /// The address the browser is sent to. Apple answers with a form post
    /// and takes a nonce; Google takes a nonce and PKCE; GitHub takes
    /// PKCE.
    pub fn authorization_url(
        &self,
        redirect_uri: &str,
        state: &str,
        nonce: &str,
        code_challenge: &str,
    ) -> Result<String, Refusal> {
        check_redirect_uri(redirect_uri)?;
        let mut url = Url::parse(self.id.authorization()).map_err(|_| "Invalid endpoint")?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("client_id", &self.client_id)
                .append_pair("redirect_uri", redirect_uri)
                .append_pair("response_type", "code")
                .append_pair("scope", self.id.scope())
                .append_pair("state", state);
            if self.id != ProviderId::GitHub {
                query.append_pair("nonce", nonce);
            }
            if self.id == ProviderId::Apple {
                query.append_pair("response_mode", "form_post");
            } else {
                query
                    .append_pair("code_challenge", code_challenge)
                    .append_pair("code_challenge_method", "S256");
            }
        }
        Ok(url.into())
    }

    /// The form that exchanges a code at the token endpoint.
    pub fn token_form(
        &self,
        redirect_uri: &str,
        code: &str,
        verifier: &str,
    ) -> Result<Vec<(&'static str, String)>, Refusal> {
        check_redirect_uri(redirect_uri)?;
        if code.is_empty() || code.len() > 4096 {
            return Err("Invalid external provider value");
        }
        let mut form = vec![
            ("grant_type", "authorization_code".to_owned()),
            ("client_id", self.client_id.clone()),
            ("client_secret", self.client_secret.clone()),
            ("redirect_uri", redirect_uri.to_owned()),
            ("code", code.to_owned()),
        ];
        if self.id != ProviderId::Apple {
            form.push(("code_verifier", verifier.to_owned()));
        }
        Ok(form)
    }

    /// The identity of an ID token, after its signature and its claims are
    /// checked: RS256 under a key of `jwks`, this issuer, this client as
    /// the audience, this nonce, and at most 10 minutes of age. `user` is
    /// the form field that Apple sends with a first sign-in.
    pub fn identity_of_id_token(
        &self,
        token: &Value,
        jwks: &Value,
        nonce: &str,
        user: Option<&str>,
        now_s: i64,
    ) -> Result<Identity, Refusal> {
        let (issuer, _) = self.id.oidc().ok_or("The provider issues no ID token")?;
        if token.get("error").is_some() {
            return Err("External provider rejected authorization code");
        }
        let id_token = required_text(token.get("id_token"), 32_768)?;
        let claims = verified_claims(id_token, jwks)?;
        let text = |name: &str| claims.get(name).and_then(Value::as_str);
        let number = |name: &str| claims.get(name).and_then(Value::as_i64);
        if text("iss") != Some(issuer) {
            return Err("Invalid external provider token issuer");
        }
        let audiences: Vec<&str> = match claims.get("aud") {
            Some(Value::String(aud)) => vec![aud.as_str()],
            Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !audiences.contains(&self.client_id.as_str()) {
            return Err("Invalid external provider token audience");
        }
        let (Some(iat), Some(exp)) = (number("iat"), number("exp")) else {
            return Err("Invalid external provider token times");
        };
        if exp <= now_s || iat > now_s + 60 || now_s - iat > MAX_TOKEN_AGE_S {
            return Err("Invalid external provider token times");
        }
        let authorized = text("azp");
        if text("nonce") != Some(nonce)
            || authorized.is_some_and(|azp| azp != self.client_id)
            || (audiences.len() > 1 && authorized != Some(self.client_id.as_str()))
        {
            return Err("Invalid external provider token binding");
        }
        let email = optional_email(claims.get("email"))?;
        let verified = match claims.get("email_verified") {
            Some(Value::Bool(true)) => true,
            // Apple sends the flag as text.
            Some(Value::String(flag)) => self.id == ProviderId::Apple && flag == "true",
            _ => false,
        };
        Ok(Identity {
            provider: self.id,
            subject: required_text(claims.get("sub"), 255)?.to_owned(),
            email_verified: email.is_some() && verified,
            email,
            display_name: if self.id == ProviderId::Apple {
                apple_display_name(user)?
            } else {
                optional_name(claims.get("name"))?
            },
        })
    }
}

/// The access token of GitHub's answer to a code.
pub fn github_access_token(token: &Value) -> Result<&str, Refusal> {
    if token.get("error").is_some() {
        return Err("External provider rejected authorization code");
    }
    if !token
        .get("token_type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("bearer"))
    {
        return Err("Invalid external provider token type");
    }
    required_text(token.get("access_token"), 4096)
}

/// The identity of a GitHub account: the numeric id, and the primary
/// email address when GitHub says it is verified. Another address of the
/// account is no evidence.
pub fn github_identity(user: &Value, emails: &Value) -> Result<Identity, Refusal> {
    let id = user
        .get("id")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0 && *id <= 9_007_199_254_740_991)
        .ok_or("Invalid GitHub subject")?;
    let rows = emails
        .as_array()
        .filter(|rows| rows.len() <= 100)
        .ok_or("Invalid GitHub email list")?;
    if rows.iter().any(|row| !row.is_object()) {
        return Err("Invalid external provider response");
    }
    let primary: Vec<&Value> = rows
        .iter()
        .filter(|row| row.get("primary") == Some(&Value::Bool(true)))
        .collect();
    if primary.len() > 1 {
        return Err("Ambiguous GitHub primary email");
    }
    let email = match primary.first() {
        Some(row) if row.get("verified") == Some(&Value::Bool(true)) => {
            optional_email(row.get("email"))?
        }
        _ => None,
    };
    let name = match user.get("name") {
        None | Some(Value::Null) => user.get("login"),
        some => some,
    };
    Ok(Identity {
        provider: ProviderId::GitHub,
        subject: id.to_string(),
        email_verified: email.is_some(),
        email,
        display_name: optional_name(name)?,
    })
}

/// The name of Apple's `user` form field: the first and the last name.
fn apple_display_name(user: Option<&str>) -> Result<Option<String>, Refusal> {
    let Some(user) = user else {
        return Ok(None);
    };
    if user.len() > 4096 {
        return Err("Invalid Apple user data");
    }
    let data: Value = serde_json::from_str(user).map_err(|_| "Invalid Apple user data")?;
    if !data.is_object() {
        return Err("Invalid external provider response");
    }
    let Some(name) = data.get("name") else {
        return Ok(None);
    };
    if !name.is_object() {
        return Err("Invalid external provider response");
    }
    let parts: Vec<String> = [
        optional_name(name.get("firstName"))?,
        optional_name(name.get("lastName"))?,
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        Ok(None)
    } else {
        Ok(Some(parts.join(" ")))
    }
}

/// The claims of an ID token whose RS256 signature a key of the set
/// makes.
fn verified_claims(token: &str, jwks: &Value) -> Result<Value, Refusal> {
    const BAD: Refusal = "Invalid external provider ID token";
    let parts: Vec<&str> = token.split('.').collect();
    let [head, body, signature] = parts.as_slice() else {
        return Err(BAD);
    };
    let decode = |part: &str| URL_SAFE_NO_PAD.decode(part).map_err(|_| BAD);
    let header: Value = serde_json::from_slice(&decode(head)?).map_err(|_| BAD)?;
    if header.get("alg").and_then(Value::as_str) != Some("RS256") {
        return Err(BAD);
    }
    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .filter(|keys| !keys.is_empty() && keys.len() <= 32)
        .ok_or("Invalid external provider signing keys")?;
    let kid = header.get("kid").and_then(Value::as_str);
    let signature = decode(signature)?;
    let signed = &token[..head.len() + 1 + body.len()];
    let made_by = |key: &Value| -> bool {
        if key.get("kty").and_then(Value::as_str) != Some("RSA") {
            return false;
        }
        let part = |name: &str| {
            key.get(name)
                .and_then(Value::as_str)
                .and_then(|text| URL_SAFE_NO_PAD.decode(text).ok())
        };
        let (Some(n), Some(e)) = (part("n"), part("e")) else {
            return false;
        };
        RsaPublicKeyComponents { n, e }
            .verify(&RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature)
            .is_ok()
    };
    let candidates = keys
        .iter()
        .filter(|key| kid.is_none_or(|kid| key.get("kid").and_then(Value::as_str) == Some(kid)));
    if !candidates.into_iter().any(made_by) {
        return Err(BAD);
    }
    let claims: Value = serde_json::from_slice(&decode(body)?).map_err(|_| BAD)?;
    if claims.is_object() {
        Ok(claims)
    } else {
        Err(BAD)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn provider(id: ProviderId) -> Provider {
        Provider {
            id,
            client_id: "client-1".to_owned(),
            client_secret: "secret".to_owned(),
        }
    }

    #[test]
    fn each_provider_gets_the_parameters_it_takes() {
        let callback = "https://pds.test/oauth/external/x/callback";
        let query = |id: ProviderId| -> Vec<(String, String)> {
            let url = provider(id)
                .authorization_url(callback, "state-1", "nonce-1", "challenge-1")
                .unwrap();
            Url::parse(&url)
                .unwrap()
                .query_pairs()
                .into_owned()
                .collect()
        };
        let has = |pairs: &[(String, String)], name: &str| pairs.iter().any(|(key, _)| key == name);
        let apple = query(ProviderId::Apple);
        assert!(
            has(&apple, "nonce") && has(&apple, "response_mode") && !has(&apple, "code_challenge")
        );
        let google = query(ProviderId::Google);
        assert!(
            has(&google, "nonce")
                && has(&google, "code_challenge")
                && !has(&google, "response_mode")
        );
        let github = query(ProviderId::GitHub);
        assert!(!has(&github, "nonce") && has(&github, "code_challenge"));
        assert!(
            provider(ProviderId::Google)
                .authorization_url("http://pds.test/cb", "s", "n", "c")
                .is_err()
        );
        let form = provider(ProviderId::Apple)
            .token_form(callback, "code-1", "verifier")
            .unwrap();
        assert!(!form.iter().any(|(name, _)| *name == "code_verifier"));
        let form = provider(ProviderId::GitHub)
            .token_form(callback, "code-1", "verifier")
            .unwrap();
        assert!(form.iter().any(|(name, _)| *name == "code_verifier"));
        assert_eq!(ProviderId::from_id("github"), Some(ProviderId::GitHub));
        assert_eq!(ProviderId::from_id("gitlab"), None);
    }

    #[test]
    fn a_github_identity_needs_a_verified_primary_address() {
        let user = json!({ "id": 4_242_001, "login": "octo", "name": null });
        let verified = json!([
            { "email": "other@example.com", "primary": false, "verified": true },
            { "email": "octo@example.com", "primary": true, "verified": true },
        ]);
        let identity = github_identity(&user, &verified).unwrap();
        assert_eq!(identity.subject, "4242001");
        assert_eq!(identity.email.as_deref(), Some("octo@example.com"));
        assert!(identity.email_verified);
        assert_eq!(identity.display_name.as_deref(), Some("octo"));

        let unverified =
            json!([{ "email": "octo@example.com", "primary": true, "verified": false }]);
        let identity = github_identity(&user, &unverified).unwrap();
        assert_eq!((identity.email, identity.email_verified), (None, false));
        let two = json!([
            { "email": "a@example.com", "primary": true, "verified": true },
            { "email": "b@example.com", "primary": true, "verified": true },
        ]);
        assert_eq!(
            github_identity(&user, &two),
            Err("Ambiguous GitHub primary email")
        );
        assert!(github_identity(&json!({ "id": "7" }), &verified).is_err());
        assert!(
            github_access_token(&json!({ "token_type": "Bearer", "access_token": "t" })).is_ok()
        );
        assert!(github_access_token(&json!({ "token_type": "mac", "access_token": "t" })).is_err());
        assert!(github_access_token(&json!({ "error": "bad_verification_code" })).is_err());
    }

    #[test]
    fn the_apple_name_is_a_suggestion_and_bad_values_are_refused() {
        let user = r#"{"name":{"firstName":"Pomme","lastName":"Verte"},"email":"x@example.com"}"#;
        assert_eq!(
            apple_display_name(Some(user)),
            Ok(Some("Pomme Verte".to_owned()))
        );
        assert_eq!(apple_display_name(Some("{}")), Ok(None));
        assert_eq!(apple_display_name(None), Ok(None));
        assert!(apple_display_name(Some("[]")).is_err());
        assert!(optional_email(Some(&json!("not an email"))).is_err());
        assert!(optional_email(Some(&json!(" a@example.com"))).is_err());
        assert_eq!(
            optional_email(Some(&json!("a@example.com"))),
            Ok(Some("a@example.com".to_owned()))
        );
        assert!(required_text(Some(&json!("a\u{0007}b")), 10).is_err());
    }

    #[test]
    fn an_id_token_without_a_good_signature_is_refused() {
        let encode = |value: &Value| URL_SAFE_NO_PAD.encode(value.to_string());
        let claims = json!({ "iss": "https://accounts.google.com", "aud": "client-1", "sub": "1" });
        let unsigned = format!(
            "{}.{}.{}",
            encode(&json!({ "alg": "RS256" })),
            encode(&claims),
            "AAAA"
        );
        let keys = json!({ "keys": [{ "kty": "RSA", "n": "AQAB", "e": "AQAB" }] });
        let google = provider(ProviderId::Google);
        let token = json!({ "id_token": unsigned });
        assert!(
            google
                .identity_of_id_token(&token, &keys, "n", None, 100)
                .is_err()
        );
        let none = format!("{}.{}.", encode(&json!({ "alg": "none" })), encode(&claims));
        assert!(
            google
                .identity_of_id_token(&json!({ "id_token": none }), &keys, "n", None, 100)
                .is_err()
        );
        assert!(
            google
                .identity_of_id_token(&token, &json!({ "keys": [] }), "n", None, 100)
                .is_err()
        );
        assert!(
            google
                .identity_of_id_token(&json!({ "error": "invalid_grant" }), &keys, "n", None, 100)
                .is_err()
        );
    }

    /// A key that exists for this test only: 2048 bits.
    const TEST_KEY: &[u8] = include_bytes!("../tests/data/test-rsa-key.pk8");
    /// Its modulus and exponent as a JWK carries them, from
    /// `openssl pkey -inform DER -in tests/data/test-rsa-key.pk8 -noout -text`.
    const TEST_N: &str = "zCiVkdpWjwQ0y8GJ1mWX0s9VWxxGl2V-OCWPNs78WqFwYhxpfVqXeePeszHH9mMukqtafCfKvxLB9Gpdxvjxqp0x9iKumMiujADLfRU6Zo1LeXUU8XIfqf8DCgeC_1hPT2yT9aj-yWHFqFV9s6UTQ1nkd7-lea_weeNGkDqAyXvniZhTlJOEJuVX_nuP9HrlekHmhtXXC8Asa8tZHYY_aPF0wEhVRJRWhSyxaEb4xcr1vz-3Ll1HS1VuMiaurWF-oY8ZGD9PLWitYti4qCk37C4ZBejqL1urFxnxVTsD6EnksAHf-a3Z0Epeh_QNV4SZZ5qhQUpN15YHlljI1BULhQ";
    const TEST_E: &str = "AQAB";

    fn signed(claims: &Value) -> (String, Value) {
        use aws_lc_rs::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
        let pair = RsaKeyPair::from_pkcs8(TEST_KEY).unwrap();
        let encode = |bytes: &[u8]| URL_SAFE_NO_PAD.encode(bytes);
        let head = encode(
            json!({ "alg": "RS256", "kid": "k1" })
                .to_string()
                .as_bytes(),
        );
        let body = encode(claims.to_string().as_bytes());
        let mut signature = vec![0; 2048 / 8];
        pair.sign(
            &RSA_PKCS1_SHA256,
            &aws_lc_rs::rand::SystemRandom::new(),
            format!("{head}.{body}").as_bytes(),
            &mut signature,
        )
        .unwrap();
        let jwks = json!({ "keys": [{
            "kty": "RSA",
            "kid": "k1",
            "n": TEST_N,
            "e": TEST_E,
        }] });
        (format!("{head}.{body}.{}", encode(&signature)), jwks)
    }

    #[test]
    fn a_signed_id_token_gives_its_identity_and_each_wrong_claim_is_refused() {
        let good = json!({
            "iss": "https://accounts.google.com",
            "aud": "client-1",
            "sub": "google-1",
            "nonce": "nonce-1",
            "iat": 1000,
            "exp": 1600,
            "email": "nova@example.com",
            "email_verified": true,
            "name": "Nova",
        });
        let google = provider(ProviderId::Google);
        let check = |claims: &Value, nonce: &str, now_s: i64| {
            let (token, jwks) = signed(claims);
            google.identity_of_id_token(&json!({ "id_token": token }), &jwks, nonce, None, now_s)
        };
        let identity = check(&good, "nonce-1", 1010).unwrap();
        assert_eq!(identity.subject, "google-1");
        assert_eq!(identity.email.as_deref(), Some("nova@example.com"));
        assert!(identity.email_verified);
        assert_eq!(identity.display_name.as_deref(), Some("Nova"));

        let with = |name: &str, value: Value| {
            let mut claims = good.clone();
            claims[name] = value;
            claims
        };
        assert!(check(&good, "another", 1010).is_err());
        assert!(check(&good, "nonce-1", 1700).is_err(), "expired");
        assert!(
            check(&good, "nonce-1", 1601 - 700).is_err(),
            "issued in the future"
        );
        assert!(check(&with("aud", json!("another-client")), "nonce-1", 1010).is_err());
        assert!(
            check(
                &with("iss", json!("https://attacker.example.com")),
                "nonce-1",
                1010
            )
            .is_err()
        );
        assert!(check(&with("azp", json!("another-client")), "nonce-1", 1010).is_err());
        assert!(
            check(&with("exp", json!(900_000)), "nonce-1", 5000).is_err(),
            "too old"
        );
        let unverified = check(&with("email_verified", json!(false)), "nonce-1", 1010).unwrap();
        assert!(!unverified.email_verified);
        // Only Apple sends the flag as text.
        let text_flag = check(&with("email_verified", json!("true")), "nonce-1", 1010).unwrap();
        assert!(!text_flag.email_verified);

        // A token that another key signed.
        let (token, _) = signed(&good);
        let other = json!({ "keys": [{ "kty": "RSA", "kid": "k1", "n": "AQAB", "e": "AQAB" }] });
        assert!(
            google
                .identity_of_id_token(&json!({ "id_token": token }), &other, "nonce-1", None, 1010)
                .is_err()
        );
    }
}
