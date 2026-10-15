//! Handle rules beyond syntax (SPEC §9.2).
//!
//! [`pds_types::Handle`] checks the syntax. This module holds what a PDS
//! adds: which top-level domains it refuses, and how the two verification
//! answers are read. The DNS and HTTPS requests themselves are the server's
//! (`pds-xrpc`); the parsing of what they return is here so that it can
//! be tested and fuzzed without a network.

use pds_types::{Did, Handle};

/// The name of the DNS TXT record that verifies a handle, without the
/// handle: `_atproto.<handle>`.
pub const TXT_PREFIX: &str = "_atproto.";
/// The path of the HTTPS verification document.
pub const WELL_KNOWN_PATH: &str = "/.well-known/atproto-did";
/// The longest handle that can be verified through DNS: 253 characters
/// less the `_atproto.` prefix.
pub const DNS_VERIFIABLE_MAX_LEN: usize = 253 - TXT_PREFIX.len();

/// Why a handle is refused for an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HandlePolicyError {
    /// The top-level domain is one a PDS never accepts.
    #[error("handle uses a disallowed top-level domain")]
    DisallowedTld,
    /// `.test` outside development.
    #[error("handle uses .test outside development")]
    TestTld,
    /// `handle.invalid` is a marker, not a handle an account can take.
    #[error("handle.invalid is a marker only")]
    InvalidMarker,
}

/// Checks the top-level domain rules for a handle an account wants.
///
/// `.alt`, `.arpa`, `.example`, `.internal`, `.invalid`, `.local`,
/// `.localhost` and `.onion` are refused; `.test` passes only in
/// development. `handle.invalid` is refused here because it is never an
/// account's choice; the server writes it itself as a marker.
pub fn check_handle_policy(handle: &Handle, dev_mode: bool) -> Result<(), HandlePolicyError> {
    if handle.normalized().as_str() == Handle::INVALID {
        return Err(HandlePolicyError::InvalidMarker);
    }
    if handle.is_disallowed_tld() {
        return Err(HandlePolicyError::DisallowedTld);
    }
    if handle.is_test_tld() && !dev_mode {
        return Err(HandlePolicyError::TestTld);
    }
    Ok(())
}

/// The DNS name of a handle's TXT record, or `None` when the handle is too
/// long for one (more than 244 characters).
pub fn txt_record_name(handle: &Handle) -> Option<String> {
    if handle.as_str().len() > DNS_VERIFIABLE_MAX_LEN {
        None
    } else {
        Some(format!("{TXT_PREFIX}{}", handle.normalized()))
    }
}

/// The TXT records of a handle name more than one DID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("TXT records name more than one DID")]
pub struct ConflictingTxtRecords;

/// Reads the TXT records at `_atproto.<handle>`.
///
/// A record counts when it starts with `did=` and the rest is a DID. No
/// such record is `Ok(None)`. Records that name different DIDs are an
/// error; the same DID twice is one answer.
pub fn did_from_txt_records<S: AsRef<str>>(
    records: &[S],
) -> Result<Option<Did>, ConflictingTxtRecords> {
    let mut found: Option<Did> = None;
    for record in records {
        let Some(value) = record.as_ref().strip_prefix("did=") else {
            continue;
        };
        let Ok(did) = Did::parse(value) else {
            continue;
        };
        match &found {
            Some(prev) if *prev != did => return Err(ConflictingTxtRecords),
            Some(_) => {}
            None => found = Some(did),
        }
    }
    Ok(found)
}

/// Reads the body of `https://<handle>/.well-known/atproto-did`: a DID with
/// optional whitespace around it. The caller has checked the 2xx status.
pub fn did_from_well_known(body: &str) -> Option<Did> {
    Did::parse(body.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(s: &str) -> Handle {
        Handle::parse(s).unwrap()
    }

    #[test]
    fn tld_policy() {
        for tld in [
            "alt",
            "arpa",
            "example",
            "internal",
            "invalid",
            "local",
            "localhost",
            "onion",
        ] {
            let h = handle(&format!("Alice.{tld}"));
            assert_eq!(
                check_handle_policy(&h, true),
                Err(HandlePolicyError::DisallowedTld),
                "{tld}"
            );
        }
        assert_eq!(
            check_handle_policy(&handle("alice.test"), false),
            Err(HandlePolicyError::TestTld)
        );
        assert_eq!(check_handle_policy(&handle("alice.test"), true), Ok(()));
        assert_eq!(
            check_handle_policy(&handle("alice.example.com"), false),
            Ok(())
        );
        // A name that only ends like a refused domain is fine.
        assert_eq!(
            check_handle_policy(&handle("alice.notlocal"), false),
            Ok(())
        );
        for marker in ["handle.invalid", "Handle.Invalid"] {
            assert_eq!(
                check_handle_policy(&handle(marker), true),
                Err(HandlePolicyError::InvalidMarker)
            );
        }
    }

    #[test]
    fn txt_name_and_length() {
        assert_eq!(
            txt_record_name(&handle("Alice.Example.com")).unwrap(),
            "_atproto.alice.example.com"
        );
        let label = "a".repeat(60);
        // 60 + 60 + 60 + 59 characters, four dots and one more: 244.
        let fits = format!("{label}.{label}.{label}.{}.b", "a".repeat(59));
        assert_eq!(fits.len(), 244);
        assert_eq!(txt_record_name(&handle(&fits)).unwrap().len(), 253);
        let long = format!("{label}.{label}.{label}.{label}.b");
        assert_eq!(long.len(), 245);
        assert_eq!(txt_record_name(&handle(&long)), None);
    }

    #[test]
    fn txt_records() {
        let did = "did:plc:ewvi7nxzyoun6zhxrhs64oiz";
        let other = "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa";
        let got = |records: &[&str]| did_from_txt_records(records);
        assert_eq!(got(&[]), Ok(None));
        assert_eq!(got(&["v=spf1 -all"]), Ok(None));
        assert_eq!(
            got(&[&format!("did={did}")]).unwrap().unwrap().as_str(),
            did
        );
        // Other records beside it do not matter.
        assert_eq!(
            got(&["v=spf1 -all", &format!("did={did}"), "did=nonsense"])
                .unwrap()
                .unwrap()
                .as_str(),
            did
        );
        // The same DID twice is one answer; two DIDs are a conflict.
        assert!(
            got(&[&format!("did={did}"), &format!("did={did}")])
                .unwrap()
                .is_some()
        );
        assert_eq!(
            got(&[&format!("did={did}"), &format!("did={other}")]),
            Err(ConflictingTxtRecords)
        );
        // The prefix is exact.
        assert_eq!(got(&[&format!(" did={did}")]), Ok(None));
        assert_eq!(got(&[&format!("DID={did}")]), Ok(None));
    }

    #[test]
    fn well_known_body() {
        let did = "did:web:example.com";
        assert_eq!(did_from_well_known(did).unwrap().as_str(), did);
        assert_eq!(
            did_from_well_known(&format!("  {did}\r\n"))
                .unwrap()
                .as_str(),
            did
        );
        assert_eq!(did_from_well_known(""), None);
        assert_eq!(did_from_well_known("<html>not found</html>"), None);
        assert_eq!(did_from_well_known(&format!("{did}\nsecond line")), None);
    }
}
