//! OAuth 2.1 authorization server and resource-server verifier with server-rendered pages.
//!
//! Specified in `docs/SPEC.md` §11. Built so far: DPoP proofs, key
//! thumbprints, the rotating nonce and PKCE.

pub mod agent;
pub mod client;
pub mod dpop;
pub mod pages;
pub mod pkce;
pub mod scope;
pub mod theme;
pub mod token;
