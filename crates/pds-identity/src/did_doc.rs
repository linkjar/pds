//! The DID-document contract (SPEC §9.3).
//!
//! A document names three things the server uses: the claimed handle, the
//! signing key and the PDS endpoint. Everything else in it is ignored. The
//! document is untrusted: it comes from the PLC directory or from a web
//! host, and the reader takes the first entry that the contract names and
//! then holds that entry to the contract.

use pds_types::{Did, Handle};
use serde::Deserialize;
use serde_json::Value;

use crate::keys::{Algorithm, KeyError, PublicKey};

/// The fragment of the atproto signing key.
pub const SIGNING_KEY_FRAGMENT: &str = "#atproto";
/// The fragment of the PDS service entry.
pub const PDS_SERVICE_FRAGMENT: &str = "#atproto_pds";
/// The service type of a PDS.
pub const PDS_SERVICE_TYPE: &str = "AtprotoPersonalDataServer";

const MULTIKEY: &str = "Multikey";
const LEGACY_K256: &str = "EcdsaSecp256k1VerificationKey2019";
const LEGACY_P256: &str = "EcdsaSecp256r1VerificationKey2019";

/// Why a DID document cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DidDocError {
    /// The document is longer than the caller allows.
    #[error("DID document is longer than {0} bytes")]
    TooLong(usize),
    /// Not JSON, or not the shape of a DID document.
    #[error("DID document does not parse: {0}")]
    Parse(String),
    /// The document's `id` is not the DID that was resolved.
    #[error("DID document is for {found}, not {expected}")]
    WrongSubject {
        /// The DID that was resolved.
        expected: String,
        /// The `id` of the document.
        found: String,
    },
    /// No verification method with the id `#atproto`.
    #[error("DID document has no #atproto verification method")]
    NoSigningKey,
    /// The `#atproto` method is controlled by another DID.
    #[error("the #atproto verification method is not controlled by the DID")]
    Controller,
    /// The `#atproto` method has a type the server does not read.
    #[error("unsupported verification method type {0:?}")]
    KeyType(String),
    /// The `#atproto` method has no usable `publicKeyMultibase`.
    #[error("the #atproto verification method holds no valid key: {0}")]
    Key(#[from] KeyError),
    /// No service with the wanted id.
    #[error("DID document has no {0} service")]
    NoService(String),
    /// The service has another type than the contract requires.
    #[error("service {id} has type {found:?}")]
    ServiceType {
        /// The service fragment.
        id: String,
        /// The type found.
        found: String,
    },
    /// The service endpoint is not a URL of scheme, host and port only.
    #[error("service {0} has an endpoint that is not scheme, host and port")]
    Endpoint(String),
}

/// A verification method entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VerificationMethod {
    /// The id: `#atproto` or `<did>#atproto`.
    pub id: String,
    /// The key type.
    #[serde(rename = "type")]
    pub type_: String,
    /// The controlling DID.
    #[serde(default)]
    pub controller: String,
    /// The key.
    #[serde(default, rename = "publicKeyMultibase")]
    pub public_key_multibase: Option<String>,
}

/// A service entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Service {
    /// The id: `#atproto_pds` or `<did>#atproto_pds`.
    pub id: String,
    /// The type. DID core allows a list; atproto services use one string,
    /// and a list never matches.
    #[serde(default, rename = "type")]
    pub type_: Value,
    /// The endpoint. DID core allows maps and lists; atproto uses a string.
    #[serde(default, rename = "serviceEndpoint")]
    pub service_endpoint: Value,
}

/// A DID document, reduced to the fields the server reads.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DidDocument {
    /// The subject.
    pub id: Did,
    /// Aliases; the first `at://` entry is the claimed handle.
    #[serde(default, rename = "alsoKnownAs")]
    pub also_known_as: Vec<String>,
    /// Keys.
    #[serde(default, rename = "verificationMethod")]
    pub verification_method: Vec<VerificationMethod>,
    /// Services.
    #[serde(default)]
    pub service: Vec<Service>,
}

/// A service endpoint that is a URL of scheme, host and optional port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceEndpoint {
    /// Whether the scheme is `https`.
    pub https: bool,
    /// The host, lowercase: a DNS name, an IPv4 address, or an IPv6 address
    /// in brackets.
    pub host: String,
    /// The port, when the URL names one.
    pub port: Option<u16>,
}

impl ServiceEndpoint {
    /// Parses `http://host[:port]` or `https://host[:port]`. A lone trailing
    /// slash is the empty path and is accepted. Userinfo, a path, a query
    /// and a fragment are refused.
    pub fn parse(s: &str) -> Option<Self> {
        let (https, rest) = match s.strip_prefix("https://") {
            Some(rest) => (true, rest),
            None => (false, s.strip_prefix("http://")?),
        };
        let authority = rest.strip_suffix('/').unwrap_or(rest);
        if authority.is_empty() || authority.len() > 253 + 6 {
            return None;
        }
        let (host, port) = if let Some(end) = authority.strip_prefix('[') {
            // An IPv6 literal.
            let (addr, after) = end.split_once(']')?;
            if addr.is_empty()
                || !addr
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
            {
                return None;
            }
            let port = match after {
                "" => None,
                p => Some(p.strip_prefix(':')?),
            };
            (format!("[{}]", addr.to_ascii_lowercase()), port)
        } else {
            let (host, port) = match authority.split_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (authority, None),
            };
            let valid = !host.is_empty()
                && host.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                });
            if !valid {
                return None;
            }
            (host.to_ascii_lowercase(), port)
        };
        let port = match port {
            None => None,
            Some(p) if !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) => {
                Some(p.parse::<u16>().ok()?)
            }
            Some(_) => return None,
        };
        Some(Self { https, host, port })
    }

    /// The URL without a trailing slash, as the server stores and compares
    /// it.
    pub fn origin(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        match self.port {
            Some(port) => format!("{scheme}://{}:{port}", self.host),
            None => format!("{scheme}://{}", self.host),
        }
    }
}

impl DidDocument {
    /// Parses a document of at most `max_len` bytes. The caller's fetch
    /// already bounds the body (SPEC §15.1); this bound holds for every
    /// other source.
    pub fn from_json(bytes: &[u8], max_len: usize) -> Result<Self, DidDocError> {
        if bytes.len() > max_len {
            return Err(DidDocError::TooLong(max_len));
        }
        serde_json::from_slice(bytes).map_err(|e| DidDocError::Parse(e.to_string()))
    }

    /// Parses a document and checks that it is about `did`.
    pub fn from_json_for(bytes: &[u8], max_len: usize, did: &Did) -> Result<Self, DidDocError> {
        let doc = Self::from_json(bytes, max_len)?;
        if &doc.id == did {
            Ok(doc)
        } else {
            Err(DidDocError::WrongSubject {
                expected: did.to_string(),
                found: doc.id.to_string(),
            })
        }
    }

    /// Whether `id` names `fragment` of this document: the relative form
    /// `#fragment` or the full form `<did>#fragment`.
    fn names(&self, id: &str, fragment: &str) -> bool {
        id == fragment
            || id
                .strip_prefix(self.id.as_str())
                .is_some_and(|rest| rest == fragment)
    }

    /// The claimed handle: the first `at://` entry of `alsoKnownAs`, when it
    /// is a handle. A first entry that is not a valid handle claims nothing;
    /// later entries are not consulted. The claim is not verified here.
    pub fn handle(&self) -> Option<Handle> {
        self.also_known_as
            .iter()
            .find_map(|aka| aka.strip_prefix("at://"))
            .and_then(|h| Handle::parse(h).ok())
            .map(|h| h.normalized())
    }

    /// The signing key: the first verification method with the id
    /// `#atproto`. It must be controlled by the DID and be a `Multikey` or
    /// one of the two legacy types.
    pub fn signing_key(&self) -> Result<PublicKey, DidDocError> {
        let method = self
            .verification_method
            .iter()
            .find(|m| self.names(&m.id, SIGNING_KEY_FRAGMENT))
            .ok_or(DidDocError::NoSigningKey)?;
        if method.controller != self.id.as_str() {
            return Err(DidDocError::Controller);
        }
        let key = method
            .public_key_multibase
            .as_deref()
            .ok_or(DidDocError::Key(KeyError::Multibase))?;
        match method.type_.as_str() {
            MULTIKEY => Ok(PublicKey::from_multikey(key)?),
            LEGACY_K256 => Ok(PublicKey::from_legacy_multibase(Algorithm::Secp256k1, key)?),
            LEGACY_P256 => Ok(PublicKey::from_legacy_multibase(Algorithm::P256, key)?),
            other => Err(DidDocError::KeyType(other.to_owned())),
        }
    }

    /// The endpoint of the first service with the id `fragment` (for example
    /// `#atproto_pds`). When `type_` is given the service must have it.
    pub fn service_endpoint(
        &self,
        fragment: &str,
        type_: Option<&str>,
    ) -> Result<ServiceEndpoint, DidDocError> {
        let service = self
            .service
            .iter()
            .find(|s| self.names(&s.id, fragment))
            .ok_or_else(|| DidDocError::NoService(fragment.to_owned()))?;
        if let Some(wanted) = type_
            && service.type_.as_str() != Some(wanted)
        {
            return Err(DidDocError::ServiceType {
                id: fragment.to_owned(),
                found: service.type_.to_string(),
            });
        }
        service
            .service_endpoint
            .as_str()
            .and_then(ServiceEndpoint::parse)
            .ok_or_else(|| DidDocError::Endpoint(fragment.to_owned()))
    }

    /// The PDS endpoint: the first `#atproto_pds` service, of type
    /// `AtprotoPersonalDataServer`.
    pub fn pds_endpoint(&self) -> Result<ServiceEndpoint, DidDocError> {
        self.service_endpoint(PDS_SERVICE_FRAGMENT, Some(PDS_SERVICE_TYPE))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const MAX: usize = 64 * 1024;
    const DID: &str = "did:plc:ewvi7nxzyoun6zhxrhs64oiz";
    const KEY: &str = "zQ3shqwJEJyMBsBXCWyCBpUBMqxcon9oHB7mCvx4sSpMdLJwc";

    fn doc(v: &Value) -> DidDocument {
        DidDocument::from_json(v.to_string().as_bytes(), MAX).unwrap()
    }

    fn plc_document() -> Value {
        // The shape plc.directory serves.
        json!({
            "@context": [
                "https://www.w3.org/ns/did/v1",
                "https://w3id.org/security/multikey/v1",
                "https://w3id.org/security/suites/secp256k1-2019/v1"
            ],
            "id": DID,
            "alsoKnownAs": ["at://Alice.Example.com"],
            "verificationMethod": [{
                "id": format!("{DID}#atproto"),
                "type": "Multikey",
                "controller": DID,
                "publicKeyMultibase": KEY
            }],
            "service": [{
                "id": "#atproto_pds",
                "type": "AtprotoPersonalDataServer",
                "serviceEndpoint": "https://pds.example.com"
            }]
        })
    }

    #[test]
    fn reads_the_three_fields_of_a_plc_document() {
        let d = doc(&plc_document());
        assert_eq!(d.handle().unwrap().as_str(), "alice.example.com");
        assert_eq!(
            d.signing_key().unwrap().to_did_key(),
            format!("did:key:{KEY}")
        );
        assert_eq!(
            d.pds_endpoint().unwrap().origin(),
            "https://pds.example.com"
        );
        let did = Did::parse(DID).unwrap();
        assert!(
            DidDocument::from_json_for(plc_document().to_string().as_bytes(), MAX, &did).is_ok()
        );
        let other = Did::parse("did:plc:aaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        assert!(matches!(
            DidDocument::from_json_for(plc_document().to_string().as_bytes(), MAX, &other),
            Err(DidDocError::WrongSubject { .. })
        ));
    }

    #[test]
    fn the_first_at_uri_is_the_claim() {
        let mut v = plc_document();
        v["alsoKnownAs"] = json!([
            "https://example.com",
            "at://one.example",
            "at://two.example"
        ]);
        assert_eq!(doc(&v).handle().unwrap().as_str(), "one.example");
        // A first entry that is no handle claims nothing.
        v["alsoKnownAs"] = json!(["at://not a handle", "at://two.example"]);
        assert_eq!(doc(&v).handle(), None);
        v["alsoKnownAs"] = json!([]);
        assert_eq!(doc(&v).handle(), None);
        v.as_object_mut().unwrap().remove("alsoKnownAs");
        assert_eq!(doc(&v).handle(), None);
    }

    #[test]
    fn signing_key_rules() {
        // Relative id.
        let mut v = plc_document();
        v["verificationMethod"][0]["id"] = json!("#atproto");
        assert!(doc(&v).signing_key().is_ok());
        // The first match decides, even when a later one would pass.
        let mut v = plc_document();
        v["verificationMethod"] = json!([
            { "id": "#atproto", "type": "Multikey", "controller": "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa", "publicKeyMultibase": KEY },
            { "id": "#atproto", "type": "Multikey", "controller": DID, "publicKeyMultibase": KEY }
        ]);
        assert_eq!(doc(&v).signing_key(), Err(DidDocError::Controller));
        // Another DID's fragment does not match.
        let mut v = plc_document();
        v["verificationMethod"][0]["id"] = json!("did:plc:aaaaaaaaaaaaaaaaaaaaaaaa#atproto");
        assert_eq!(doc(&v).signing_key(), Err(DidDocError::NoSigningKey));
        // Other fragments do not match.
        let mut v = plc_document();
        v["verificationMethod"][0]["id"] = json!("#atproto_label");
        assert_eq!(doc(&v).signing_key(), Err(DidDocError::NoSigningKey));
        // Legacy types carry the key without the multicodec prefix.
        let mut v = plc_document();
        v["verificationMethod"][0]["type"] = json!("EcdsaSecp256k1VerificationKey2019");
        v["verificationMethod"][0]["publicKeyMultibase"] =
            json!("z25z9DTpsiYYJKGsWmSPJK2NFN8PcJtZig12K59UgW7q5t");
        assert_eq!(
            doc(&v).signing_key().unwrap().to_did_key(),
            format!("did:key:{KEY}")
        );
        let mut v = plc_document();
        v["verificationMethod"][0]["type"] = json!("EcdsaSecp256r1VerificationKey2019");
        v["verificationMethod"][0]["publicKeyMultibase"] =
            json!("zxdM8dSstjrpZaRUwBmDvjGXweKuEMVN95A9oJBFjkWMh");
        assert_eq!(
            doc(&v).signing_key().unwrap().to_did_key(),
            "did:key:zDnaembgSGUhZULN2Caob4HLJPaxBh92N7rtH21TErzqf8HQo"
        );
        // An unknown type, a missing key and a broken key.
        let mut v = plc_document();
        v["verificationMethod"][0]["type"] = json!("Ed25519VerificationKey2020");
        assert!(matches!(
            doc(&v).signing_key(),
            Err(DidDocError::KeyType(_))
        ));
        let mut v = plc_document();
        v["verificationMethod"][0]
            .as_object_mut()
            .unwrap()
            .remove("publicKeyMultibase");
        assert!(matches!(doc(&v).signing_key(), Err(DidDocError::Key(_))));
        let mut v = plc_document();
        v["verificationMethod"][0]["publicKeyMultibase"] = json!("zzzz");
        assert!(matches!(doc(&v).signing_key(), Err(DidDocError::Key(_))));
    }

    #[test]
    fn pds_endpoint_rules() {
        let with = |service: Value| {
            let mut v = plc_document();
            v["service"] = service;
            doc(&v).pds_endpoint()
        };
        let entry = |id: &str, type_: &str, endpoint: Value| json!([{ "id": id, "type": type_, "serviceEndpoint": endpoint }]);
        let pds = "AtprotoPersonalDataServer";
        assert!(
            with(entry(
                &format!("{DID}#atproto_pds"),
                pds,
                json!("https://pds.example.com")
            ))
            .is_ok()
        );
        assert_eq!(
            with(entry("#atproto_pds", pds, json!("http://localhost:2583/")))
                .unwrap()
                .origin(),
            "http://localhost:2583"
        );
        assert_eq!(
            with(entry(
                "#atproto_pds",
                pds,
                json!("https://[2001:DB8::1]:8443")
            ))
            .unwrap()
            .origin(),
            "https://[2001:db8::1]:8443"
        );
        assert!(matches!(
            with(entry(
                "#atproto_pds",
                "AtprotoLabeler",
                json!("https://pds.example.com")
            )),
            Err(DidDocError::ServiceType { .. })
        ));
        assert!(matches!(
            with(entry(
                "#bsky_appview",
                pds,
                json!("https://pds.example.com")
            )),
            Err(DidDocError::NoService(_))
        ));
        for bad in [
            json!("https://pds.example.com/xrpc"),
            json!("https://pds.example.com?x=1"),
            json!("https://pds.example.com#frag"),
            json!("https://user@pds.example.com"),
            json!("https://pds.example.com:99999"),
            json!("https://pds.example.com:"),
            json!("https://"),
            json!("ftp://pds.example.com"),
            json!("pds.example.com"),
            json!("https://pds..example.com"),
            json!("https://-pds.example.com"),
            json!({ "uri": "https://pds.example.com" }),
            json!(["https://pds.example.com"]),
            Value::Null,
        ] {
            assert!(
                matches!(
                    with(entry("#atproto_pds", pds, bad.clone())),
                    Err(DidDocError::Endpoint(_))
                ),
                "{bad}"
            );
        }
        // The first matching service decides.
        let two = json!([
            { "id": "#atproto_pds", "type": pds, "serviceEndpoint": "https://first.example.com" },
            { "id": "#atproto_pds", "type": pds, "serviceEndpoint": "https://second.example.com" }
        ]);
        assert_eq!(with(two).unwrap().origin(), "https://first.example.com");
    }

    #[test]
    fn hostile_documents_fail_cleanly() {
        assert!(matches!(
            DidDocument::from_json(&vec![b' '; MAX + 1], MAX),
            Err(DidDocError::TooLong(_))
        ));
        for bad in [
            "",
            "[]",
            "{}",
            r#"{"id":"not a did"}"#,
            r#"{"id":"did:plc:x","verificationMethod":{}}"#,
            r#"{"id":"did:plc:x","service":[{"type":"x"}]}"#,
        ] {
            assert!(
                matches!(
                    DidDocument::from_json(bad.as_bytes(), MAX),
                    Err(DidDocError::Parse(_))
                ),
                "{bad}"
            );
        }
        // A document with nothing but an id is valid and names nothing.
        let empty = DidDocument::from_json(br#"{"id":"did:web:example.com"}"#, MAX).unwrap();
        assert_eq!(empty.handle(), None);
        assert_eq!(empty.signing_key(), Err(DidDocError::NoSigningKey));
        assert!(matches!(
            empty.pds_endpoint(),
            Err(DidDocError::NoService(_))
        ));
    }
}
