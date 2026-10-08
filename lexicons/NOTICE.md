# Vendored lexicons

- `com/atproto/**` and `app/bsky/**` are copied verbatim from
  `bluesky-social/atproto` at commit `7ca16cc6989f8247637615aca17c5abb911b8fb1`
  (`@atproto/pds@0.5.34`), the reference pin in `../upstream.json`. Upstream
  licenses them under MIT or Apache-2.0; see `../docs/upstream-LICENSE.txt`.
  Do not edit them. An upstream bump replaces the directories and regenerates
  `crates/pds-types/src/generated`.
- `io/linkjar/account/**` are the LinkJar lexicons from
  `../patches/099b-signup-receipt.patch` at the patch pin.
- `io/linkjar/pds/**` are project lexicons defined by this repository under the
  NSID authority `io.linkjar.pds.*`.

Regenerate types with `cargo xtask codegen`. The generated file is checked in;
CI fails if it is stale.
