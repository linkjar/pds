//! OAuth 2.1 authorization server and resource-server verifier with server-rendered pages.
//!
//! Specified in `docs/SPEC.md` §11. The rules and the pages are here; the
//! routes, the rows and the fetching are in `pds-xrpc/src/oauth`.

pub mod agent;
pub mod client;
pub mod dpop;
pub mod pages;
pub mod pkce;
pub mod scope;
pub mod theme;
pub mod token;
