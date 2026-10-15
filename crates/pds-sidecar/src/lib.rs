//! What the sidecars of LinkJar PDS share (`docs/SPEC.md` §22): the OAuth
//! client of the PDS with DPoP, the helpers of their pages, the checks on
//! their configuration, and the logging and the stop signal of a process,
//! which the server uses too.
//!
//! The crate holds the HTTP client, so that `pds-oauth` stays free of it.

pub mod client;
pub mod config;
pub mod http;
pub mod process;
