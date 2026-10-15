//! The DID methods the server supports (SPEC §9.1).
//!
//! `did:plc` for accounts created here, and `did:web` for accounts that
//! migrate in. A `did:web` names a host and nothing else: no path, and a
//! port only on `localhost` in development. Errors keep three cases apart:
//! invalid syntax, an unsupported method, and (in the resolver) a failed
//! resolution.

use pds_types::{Did, SyntaxError};

/// A DID of a method the server supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupportedDid {
    /// `did:plc:<24 base32 characters>`.
    Plc(Did),
    /// `did:web:<host>`, with the host and the URL scheme to resolve it.
    Web {
        /// The DID.
        did: Did,
        /// The host, with `:<port>` for a development `localhost`.
        host: String,
        /// Whether the document is fetched over `https`. Only a development
        /// `localhost` uses `http`.
        https: bool,
    },
}

/// Why a DID is not usable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DidError {
    /// The string is not a DID.
    #[error("invalid DID: {0}")]
    Syntax(#[from] SyntaxError),
    /// A method other than `plc` and `web`.
    #[error("unsupported DID method {0:?}")]
    UnsupportedMethod(String),
    /// A `did:plc` whose identifier is not 24 characters of base32.
    #[error("did:plc identifier must be 24 characters of a-z and 2-7")]
    PlcIdentifier,
    /// A `did:web` with a path. Only the host level is supported.
    #[error("did:web with a path is not supported")]
    WebPath,
    /// A `did:web` whose host is not a domain name, or that names
    /// `localhost` or a port outside development.
    #[error("did:web host is not acceptable")]
    WebHost,
}

impl SupportedDid {
    /// Parses and classifies a DID. `dev_mode` admits `did:web:localhost`
    /// with an optional port.
    pub fn parse(s: &str, dev_mode: bool) -> Result<Self, DidError> {
        Self::classify(Did::parse(s)?, dev_mode)
    }

    /// Classifies a DID that already parsed.
    pub fn classify(did: Did, dev_mode: bool) -> Result<Self, DidError> {
        match did.method() {
            "plc" => {
                let id = did.as_str().strip_prefix("did:plc:").unwrap_or_default();
                let valid = id.len() == 24
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b));
                if valid {
                    Ok(Self::Plc(did))
                } else {
                    Err(DidError::PlcIdentifier)
                }
            }
            "web" => {
                let id = did.as_str().strip_prefix("did:web:").unwrap_or_default();
                if id.contains(':') {
                    return Err(DidError::WebPath);
                }
                let (host, https) = web_host(id, dev_mode)?;
                Ok(Self::Web { did, host, https })
            }
            other => Err(DidError::UnsupportedMethod(other.to_owned())),
        }
    }

    /// The DID.
    pub const fn did(&self) -> &Did {
        match self {
            Self::Plc(did) | Self::Web { did, .. } => did,
        }
    }

    /// For a `did:web`, the URL of its document.
    pub fn web_document_url(&self) -> Option<String> {
        match self {
            Self::Plc(_) => None,
            Self::Web { host, https, .. } => {
                let scheme = if *https { "https" } else { "http" };
                Some(format!("{scheme}://{host}/.well-known/did.json"))
            }
        }
    }
}

/// Reads the identifier of a host-level `did:web`: a domain name, or in
/// development `localhost` with an optional percent-encoded port.
fn web_host(id: &str, dev_mode: bool) -> Result<(String, bool), DidError> {
    let lower = id.to_ascii_lowercase();
    let (name, port) = match lower.split_once("%3a") {
        Some((name, port)) => (name, Some(port)),
        None => (lower.as_str(), None),
    };
    if name == "localhost" {
        if !dev_mode {
            return Err(DidError::WebHost);
        }
        return match port {
            None => Ok((name.to_owned(), false)),
            Some(p) if !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) => {
                let port: u16 = p.parse().map_err(|_| DidError::WebHost)?;
                Ok((format!("{name}:{port}"), false))
            }
            Some(_) => Err(DidError::WebHost),
        };
    }
    // Every other host is a public domain name: the handle syntax is the
    // DNS name syntax with at least two labels. No port and no escapes.
    if port.is_some() || name.contains('%') || pds_types::Handle::parse(name).is_err() {
        return Err(DidError::WebHost);
    }
    Ok((name.to_owned(), true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plc() {
        let ok = SupportedDid::parse("did:plc:ewvi7nxzyoun6zhxrhs64oiz", false).unwrap();
        assert!(matches!(ok, SupportedDid::Plc(_)));
        assert_eq!(ok.web_document_url(), None);
        for bad in [
            "did:plc:ewvi7nxzyoun6zhxrhs64oi",   // 23 characters
            "did:plc:ewvi7nxzyoun6zhxrhs64oizz", // 25
            "did:plc:EWVI7NXZYOUN6ZHXRHS64OIZ",  // upper case
            "did:plc:ewvi7nxzyoun6zhxrhs64oi1",  // 1 is not base32
        ] {
            assert_eq!(
                SupportedDid::parse(bad, false),
                Err(DidError::PlcIdentifier),
                "{bad}"
            );
        }
    }

    #[test]
    fn web() {
        let ok = SupportedDid::parse("did:web:Example.COM", false).unwrap();
        assert_eq!(
            ok.web_document_url().unwrap(),
            "https://example.com/.well-known/did.json"
        );
        assert_eq!(ok.did().as_str(), "did:web:Example.COM");
        assert_eq!(
            SupportedDid::parse("did:web:example.com:user:alice", false),
            Err(DidError::WebPath)
        );
        for bad in [
            "did:web:localhost",
            "did:web:localhost%3A2583",
            "did:web:example.com%3A8443",
            "did:web:example",
            "did:web:exa%20mple.com",
            "did:web:-bad.example.com",
        ] {
            assert_eq!(
                SupportedDid::parse(bad, false),
                Err(DidError::WebHost),
                "{bad}"
            );
        }
        // Development admits localhost and a port on it, and nothing more.
        assert_eq!(
            SupportedDid::parse("did:web:localhost%3A2583", true)
                .unwrap()
                .web_document_url()
                .unwrap(),
            "http://localhost:2583/.well-known/did.json"
        );
        assert_eq!(
            SupportedDid::parse("did:web:localhost", true)
                .unwrap()
                .web_document_url()
                .unwrap(),
            "http://localhost/.well-known/did.json"
        );
        for bad in [
            "did:web:localhost%3A",
            "did:web:localhost%3A99999",
            "did:web:localhost%3Aabc",
            "did:web:example.com%3A8443",
        ] {
            assert_eq!(
                SupportedDid::parse(bad, true),
                Err(DidError::WebHost),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_three_error_cases_stay_apart() {
        assert!(matches!(
            SupportedDid::parse("not a did", false),
            Err(DidError::Syntax(_))
        ));
        assert_eq!(
            SupportedDid::parse(
                "did:key:zQ3shqwJEJyMBsBXCWyCBpUBMqxcon9oHB7mCvx4sSpMdLJwc",
                false
            ),
            Err(DidError::UnsupportedMethod("key".to_owned()))
        );
        // The length limit of SPEC §9.1 is a syntax error.
        let long = format!("did:web:{}.com", "a".repeat(2048));
        assert!(matches!(
            SupportedDid::parse(&long, false),
            Err(DidError::Syntax(_))
        ));
    }
}
