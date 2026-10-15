//! The parts of the LinkJar PDS binary: configuration, and the `serve`,
//! `verify` and `reconcile` commands (`docs/SPEC.md` §14). The command line
//! is in `main.rs`.

pub mod config;
pub mod reconcile;
pub mod serve;
pub mod verify;
