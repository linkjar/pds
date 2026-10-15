# xtask

Repository tasks, run as `cargo xtask <task>` through the alias in
`.cargo/config.toml`.

`cargo xtask codegen` loads `lexicons/`, validates every document and
resolves the references, then writes `crates/pds-types/src/generated/mod.rs`
(the API types) and `crates/pds-lexicon/src/bundled.rs` (the documents,
included at compile time), both formatted with `rustfmt`.
`cargo xtask codegen --check` fails when either file is stale; CI runs it.

It depends on pds-lexicon and needs `rustfmt` on the path. It is a
workspace member and not a default member, so `cargo build --workspace`
builds it and `cargo build` does not.
