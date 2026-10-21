//! The hCaptcha challenge of a sign-up (`docs/SPEC.md` §12.2).
//!
//! The page shows hCaptcha's widget, the form brings its answer, and the
//! server asks hCaptcha whether the answer is good, as `lib/hcaptcha.ts`
//! of the Reference does. A sign-up without a good answer creates nothing.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::state::AppState;

const VERIFY_URL: &str = "https://api.hcaptcha.com/siteverify";
const TIMEOUT: Duration = Duration::from_secs(10);

/// The hCaptcha settings.
#[derive(Clone)]
pub struct Hcaptcha {
    /// `PDS_HCAPTCHA_SITE_KEY`: public, it goes into the page.
    pub site_key: String,
    /// `PDS_HCAPTCHA_SECRET_KEY`.
    pub secret_key: String,
    /// `PDS_HCAPTCHA_TOKEN_SALT`: the salt of the hashes that tell
    /// hCaptcha which requests belong together.
    pub token_salt: String,
}

impl std::fmt::Debug for Hcaptcha {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hcaptcha")
            .field("site_key", &self.site_key)
            .finish_non_exhaustive()
    }
}

impl Hcaptcha {
    fn hashed(&self, value: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(self.token_salt.as_bytes());
        hash.update(value.as_bytes());
        STANDARD.encode(hash.finalize())
    }

    /// The form that asks hCaptcha about an answer.
    fn form(&self, answer: &str, ip: &str, handle: &str, user_agent: Option<&str>) -> String {
        let mut tokens = json!({
            "hashedIp": self.hashed(ip),
            "hashedHandle": self.hashed(handle),
        });
        if let Some(agent) = user_agent {
            tokens["hashedUserAgent"] = Value::String(self.hashed(agent));
        }
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("secret", &self.secret_key)
            .append_pair("sitekey", &self.site_key)
            .append_pair("behavior_type", "signup")
            .append_pair("response", answer)
            .append_pair("remoteip", ip)
            .append_pair("client_tokens", &tokens.to_string())
            .finish()
    }
}

/// Whether hCaptcha says the answer of a sign-up is good. A request that
/// fails counts as a bad answer.
pub async fn verify(
    state: &AppState,
    hcaptcha: &Hcaptcha,
    answer: &str,
    ip: &str,
    handle: &str,
    user_agent: Option<&str>,
) -> bool {
    if answer.is_empty() {
        return false;
    }
    let response = state
        .http()
        .post(VERIFY_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .timeout(TIMEOUT)
        .body(hcaptcha.form(answer, ip, handle, user_agent))
        .send()
        .await;
    let Ok(response) = response else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    let Ok(bytes) = pds_identity::safe_fetch::read_bounded(response, 64 * 1024).await else {
        return false;
    };
    serde_json::from_slice::<Value>(&bytes)
        .is_ok_and(|result| result.get("success") == Some(&Value::Bool(true)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_carries_salted_hashes_and_no_raw_handle() {
        let hcaptcha = Hcaptcha {
            site_key: "site".to_owned(),
            secret_key: "secret".to_owned(),
            token_salt: "salt".to_owned(),
        };
        let form = hcaptcha.form("answer", "192.0.2.1", "nova.test", Some("agent"));
        let pairs: Vec<(String, String)> = url::form_urlencoded::parse(form.as_bytes())
            .into_owned()
            .collect();
        let value = |name: &str| pairs.iter().find(|(key, _)| key == name).unwrap().1.clone();
        assert_eq!(value("behavior_type"), "signup");
        assert_eq!(value("response"), "answer");
        let tokens: Value = serde_json::from_str(&value("client_tokens")).unwrap();
        // SHA-256 of "salt" and the value, in base64.
        assert_eq!(tokens["hashedHandle"], hcaptcha.hashed("nova.test"));
        assert_eq!(hcaptcha.hashed("x").len(), 44);
        assert!(!value("client_tokens").contains("nova.test"));
        assert!(!format!("{hcaptcha:?}").contains("secret"));
    }
}
