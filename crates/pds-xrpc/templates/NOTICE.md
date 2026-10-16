# Mail templates

The five `.hbs` files are the token mails of `@atproto/pds`, copied from
`packages/pds/src/mailer/templates/` of `bluesky-social/atproto` at the
commit that [`upstream.json`](../../../upstream.json) pins. Upstream
licenses them under MIT or Apache-2.0, as this repository is.

`crates/pds-xrpc/src/mail.rs` fills them. It reads the variables and the
one condition they use; it is not a Handlebars engine. The templates of
the `linkjar` profile, which replace these when that profile runs, are in
`crates/pds-ext-linkjar/templates/`.
