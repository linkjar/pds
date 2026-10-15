# pds-lexicon

The lexicon schema as Rust data, a loader for the lexicons built into the
binary or for a directory, the document rules, and record validation:
strict on write, with relaxed blob rules on read. It also holds the
Reference server's wording for validation errors, so that the server
answers with the same messages. It does no I/O at run time.

## Modules

| Module | Holds |
|---|---|
| `model` | `LexiconDoc`, `LexUserType` and the `Lex*` types, permission sets included. |
| `validate` | `validate_record`, `validate_record_key`, `validate_data_model`, `ValidationError`, `BlobRules`. |
| `wording` | The Reference server's error messages. |
| `bundled` | `DOCS`: the lexicon files under `lexicons/`, included at compile time; written by `cargo xtask codegen`. |

`Corpus`, in `lib.rs`, is the entry point: `Corpus::bundled`,
`Corpus::load_dir`, `resolve_ref`, `unresolved_refs` and the validation
methods.

## Place in the workspace

Depends on pds-types. Used by pds-xrpc, by `xtask` (which validates the
lexicons before it generates code) and by `fuzz/`.

## What it needs

Nothing at run time. `lexicons/` is needed at build time, through
`bundled.rs`.

## Tests

`cargo test -p pds-lexicon`. The inline tests read `interop/lexicon/catalog`,
`interop/lexicon/record-data-valid.json`, `record-data-invalid.json` and
`interop/data-model`. No network, no Docker.

## Specification

SPEC §2.1, §5.5, §6.1 to §6.3 and §17.3. Permission sets are modelled in
`model` and not resolved: an `include:` scope is accepted and grants
nothing, as `docs/plan.md` records under "Not built".
