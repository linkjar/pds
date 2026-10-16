# pds-ext

The boundary between the server core and the behaviour of one operator. It
holds the `Profile` trait, whose every method has the stock behaviour as
its default; the `Fetch` trait, which the server implements so that a
profile can call out without an HTTP client of its own; the value types
the two sides exchange; and `Stock`, the profile that keeps every default.
It has no I/O and no operator logic.

## Contents

Everything is in `lib.rs`: `Profile`, `Stock`, `Fetch`, `Boxed`,
`ProviderInfo`, `VerifiedIdentity`, `ProviderAnswer`, `Rename`,
`Suggested`, `HandlePolicyRow`, `MailKind`. The futures are boxed by hand,
without `async-trait`. The brand of §12.7 is two methods: `mark`, the
path of the mark the pages show, and `mail_template`, the profile's own
template for a mail where it has one.

## Place in the workspace

Depends on no workspace crate. Used by pds-xrpc (which calls the profile),
pds-ext-linkjar (which implements it) and linkjar-pds (which picks the
profile from `PDS_PROFILE`).

## What it needs

Nothing.

## Tests

None of its own. The profiles are tested where they are implemented and
where they are called.

## Specification

SPEC §12 (§12.1 to §12.11).
