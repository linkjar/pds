//! The linkjar profile: LinkJar account behaviour as native extension implementations.
//!
//! Specified in `docs/SPEC.md` §13. Built so far: the hosted handle policy
//! of patch 093.

pub mod handle;
pub mod reserved;

use std::fmt::Write;

use pds_ext::{HandlePolicyRow, Profile, Rename};

/// The `linkjar` profile.
#[derive(Debug, Clone, Copy, Default)]
pub struct Linkjar;

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
            Linkjar
                .profile_page("nova.linkjar.social", "did:plc:abc")
                .as_deref(),
            Some("https://linkjar.io/jar/did%3Aplc%3Aabc")
        );
        assert_eq!(
            Linkjar.profile_page("support.linkjar.social", "did:plc:abc"),
            None
        );
        assert_eq!(
            Linkjar.profile_page("nova.example.org", "did:plc:abc"),
            None
        );
        assert!(Linkjar.reserves(".linkjar.social", "pds2"));
        assert!(!Linkjar.reserves(".example.org", "pds2"));
        assert!(!pds_ext::Stock.reserves(".linkjar.social", "pds2"));
    }
}
