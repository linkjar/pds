//! The linkjar profile: LinkJar account behaviour as native extension implementations.
//!
//! Specified in `docs/SPEC.md` §13. Built so far: the hosted handle policy
//! of patch 093.

pub mod handle;
pub mod providers;
pub mod reserved;

use std::fmt::Write;

use pds_ext::{
    Boxed, Fetch, HandlePolicyRow, Profile, ProviderAnswer, ProviderInfo, Rename, Suggested,
    VerifiedIdentity,
};

/// The `linkjar` profile.
#[derive(Debug, Clone, Default)]
pub struct Linkjar {
    /// The identity providers that have credentials.
    pub providers: Vec<providers::Provider>,
}

impl Linkjar {
    fn provider(&self, id: &str) -> Option<&providers::Provider> {
        self.providers
            .iter()
            .find(|provider| provider.id.id() == id)
    }
}

async fn exchange(
    provider: &providers::Provider,
    fetch: &dyn Fetch,
    answer: ProviderAnswer<'_>,
) -> Result<VerifiedIdentity, &'static str> {
    let form = provider.token_form(answer.redirect_uri, answer.code, answer.verifier)?;
    let token = fetch.post_form(provider.id.token_endpoint(), &form).await?;
    let identity = if let Some((_, jwks)) = provider.id.oidc() {
        let keys = fetch.get_json(jwks, None).await?;
        provider.identity_of_id_token(&token, &keys, answer.nonce, answer.user, answer.now_s)?
    } else {
        let access = providers::github_access_token(&token)?.to_owned();
        let user = fetch
            .get_json(providers::GITHUB_USER, Some(&access))
            .await?;
        let emails = fetch
            .get_json(providers::GITHUB_EMAILS, Some(&access))
            .await?;
        providers::github_identity(&user, &emails)?
    };
    Ok(VerifiedIdentity {
        provider: identity.provider.id(),
        subject: identity.subject,
        email: identity.email,
        email_verified: identity.email_verified,
        display_name: identity.display_name,
    })
}

impl Profile for Linkjar {
    fn name(&self) -> &'static str {
        "linkjar"
    }

    fn reserves(&self, domain: &str, front: &str) -> bool {
        domain == handle::DOMAIN && reserved::is_reserved_label(front)
    }

    fn keeps_handle_policy(&self) -> bool {
        true
    }

    fn rename(
        &self,
        policy: Option<&HandlePolicyRow>,
        previous: Option<&str>,
        handle: &str,
        now_ms: i64,
    ) -> Rename {
        handle::rename(policy, previous, handle, now_ms)
    }

    fn providers(&self) -> Vec<ProviderInfo> {
        self.providers
            .iter()
            .map(|provider| ProviderInfo {
                id: provider.id.id(),
                label: provider.id.label(),
                form_post: provider.id == providers::ProviderId::Apple,
            })
            .collect()
    }

    fn provider_start(
        &self,
        provider: &str,
        redirect_uri: &str,
        state: &str,
        nonce: &str,
        code_challenge: &str,
    ) -> Option<Result<String, &'static str>> {
        Some(
            self.provider(provider)?
                .authorization_url(redirect_uri, state, nonce, code_challenge),
        )
    }

    fn provider_exchange<'a>(
        &'a self,
        provider: &'a str,
        fetch: &'a dyn Fetch,
        answer: ProviderAnswer<'a>,
    ) -> Boxed<'a, Result<VerifiedIdentity, &'static str>> {
        Box::pin(async move {
            let provider = self
                .provider(provider)
                .ok_or("The provider is not configured")?;
            exchange(provider, fetch, answer).await
        })
    }

    fn suggested_label(
        &self,
        display_name: Option<&str>,
        email: Option<&str>,
        attempt: usize,
        suffix: [u8; 3],
    ) -> Suggested {
        if attempt >= handle::ATTEMPTS {
            return Suggested::End;
        }
        let base = handle::base_label(display_name, email);
        let label = handle::attempt_label(&base, attempt, suffix);
        if handle::allowed(&label) {
            Suggested::Handle(format!("{label}{}", handle::DOMAIN))
        } else {
            Suggested::Skip
        }
    }

    fn has_handle_hosts(&self) -> bool {
        true
    }

    fn profile_page(&self, handle: &str, did: &str) -> Option<String> {
        let label = handle.strip_suffix(handle::DOMAIN)?;
        if !handle::is_linkjar_handle(handle) || reserved::is_reserved_label(label) {
            return None;
        }
        // The DID is the only input in the address, and it is encoded.
        let mut encoded = String::new();
        for byte in did.bytes() {
            if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
                encoded.push(char::from(byte));
            } else {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
        Some(format!("https://linkjar.io/jar/{encoded}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_host_leads_to_the_page_of_its_account() {
        assert_eq!(
            Linkjar::default()
                .profile_page("nova.linkjar.social", "did:plc:abc")
                .as_deref(),
            Some("https://linkjar.io/jar/did%3Aplc%3Aabc")
        );
        assert_eq!(
            Linkjar::default().profile_page("support.linkjar.social", "did:plc:abc"),
            None
        );
        assert_eq!(
            Linkjar::default().profile_page("nova.example.org", "did:plc:abc"),
            None
        );
        assert!(Linkjar::default().reserves(".linkjar.social", "pds2"));
        assert!(!Linkjar::default().reserves(".example.org", "pds2"));
        assert!(!pds_ext::Stock.reserves(".linkjar.social", "pds2"));
    }
}
