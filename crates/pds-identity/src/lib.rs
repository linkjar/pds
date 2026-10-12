//! Identity: keys, DID documents, DID methods and handle rules
//! (`docs/SPEC.md` §9, §6.1).
//!
//! This crate holds the parts of identity that need no network:
//!
//! - [`keys`]: secp256k1 and P-256 keys, `did:key`, and signatures with the
//!   low-S rule on both curves. The interop crypto vectors are its tests.
//! - [`did_doc`]: the DID-document contract of §9.3.
//! - [`method`]: the supported DID methods and the `did:web` rules of §9.1.
//! - [`handle`]: the handle rules of §9.2 beyond syntax.
//!
//! The resolver with its cache, the PLC client, and DNS and HTTPS handle
//! verification arrive with the read-only server; see `docs/plan.md`.

pub mod did_doc;
pub mod handle;
pub mod keys;
pub mod method;

pub use did_doc::{DidDocError, DidDocument, ServiceEndpoint};
pub use handle::{HandlePolicyError, check_handle_policy};
pub use keys::{Algorithm, KeyError, Keypair, PublicKey, SignatureError};
pub use method::{DidError, SupportedDid};
