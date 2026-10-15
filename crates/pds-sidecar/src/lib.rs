//! What the sidecars of LinkJar PDS share (`docs/SPEC.md` §22): the OAuth
//! client of the PDS with DPoP, what their pages read from a request, the
//! checks on their configuration, and the logging and the stop signal of a
//! process, which the server uses too.
//!
//! The crate holds the HTTP client, so that `pds-oauth` stays free of it.
//! It holds no HTTP server: the console serves with Topcoat and the MCP
//! server with axum, so each keeps the shape of its own responses.

pub mod client;
pub mod config;
pub mod process;
pub mod request;
