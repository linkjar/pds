//! AT Protocol syntax types, the data model, and lexicon-generated API types.
//!
//! The syntax types (`Did`, `Handle`, `Nsid`, `Tid`, `RecordKey`, `AtUri`,
//! `Datetime`, `Language`, `Uri`, `CidString`, `AtIdentifier`) validate on
//! construction with the rules of the AT Protocol syntax specifications and
//! the limits of `docs/SPEC.md` §6.1, §6.2 and §9. The interop vectors under
//! `interop/syntax` are their tests.
//!
//! [`Data`] is the atproto data model with the JSON representation (`$link`,
//! `$bytes`) and the DAG-CBOR representation (tag 42, byte strings) chosen by
//! the serializer's `is_human_readable` flag.
//!
//! `generated` holds the API types produced by `cargo xtask codegen` from the
//! vendored lexicons. It is checked in and CI fails when it is stale.

pub mod data;
pub mod syntax;

#[allow(missing_docs, clippy::all, clippy::pedantic, clippy::nursery)]
pub mod generated;

pub use data::{BlobRef, Bytes, CidLink, Data};
pub use syntax::{
    AtIdentifier, AtUri, CidString, Datetime, Did, Handle, Language, Nsid, RecordKey, SyntaxError,
    Tid, TidGenerator, Uri,
};
