# Differences between targets

[Harness README](../README.md) · [Specification §2.2, C6](../../docs/SPEC.md#22-definition-of-compatible)

`pnpm parity compare <left> <right>` compares the transcripts of two targets.
The differences it may find are recorded here, one file per pair. A
difference that is not in the file fails the comparison, and so does a
recorded difference that no longer appears.

| Pair | File | Differences |
|---|---|---|
| `reference` against `repeat` (two boots of the reference image) | none | None is allowed. This is the check on the normaliser. |
| `reference` against `stock` (the production build against the `PATCH_PROFILE=none` build) | [reference-vs-stock.json](reference-vs-stock.json) | 33, in the four groups below. |
| `stock` against `official` (the `PATCH_PROFILE=none` build against the upstream distribution `ghcr.io/bluesky-social/pds`) | none | None is allowed: the two are different packagings of the same package version. |

Regenerate a file with `--update` after a change that is meant to move it,
and review the result like code.

## The production build against the stock build

Both targets run the same configuration: the production hostnames, the same
environment, the same fixtures. They differ in the eight patches of
[`legacy/patches/`](../../legacy/patches/README.md). Sixteen of the
twenty-five scenarios show no difference at all, and in three more the only
difference is the text of a mail or one page header. The patches do not
change the repository, the firehose, proxying, rate limits, legacy sessions,
handles and PLC operations of password accounts, account status, admin
methods, invite administration, or the OAuth protocol.

| Group | Patch | Scenarios | What differs |
|---|---|---|---|
| LinkJar behaviour that a stock build does not have | 093, 099, 099b, 100, 102, 103 | `default/30-linkjar-external`, `31-linkjar-handles`, `32-linkjar-signup-journey`, `33-linkjar-signin-methods`, `invites/02-linkjar-invite-handoff` | The scenario is skipped on the stock build, so each is one recorded difference. On the production build they pass: external sign-in, derived handles, the creation receipt, the hosted handle policy and handle hosts, the sign-up journey, sign-in methods with their notices, and the invitation hand-off. |
| Signup policy | 099c | `captcha/01-signup-policy` (22 differences) | With one of the three hCaptcha variables missing, the production build refuses to start and says "Partial hCaptcha config"; the stock build reads it as no hCaptcha and starts. With hCaptcha configured, the production build answers 400 "Create an account through the OAuth signup page" to a legacy `createAccount`; the stock build creates the account, which bypasses the CAPTCHA of the sign-up page. |
| Account mail | 101 | `default/10-email` (4), `default/11-identity` (1) | The text of the five token mails: confirm email, update email, reset password, delete account, PLC operation. Subjects, sender and the presence of a text part are the same. The sixth template, the sign-in notice, exists only in the production build. |
| Page security header | 102 | `default/21-oauth-negative` (1) | The `Content-Security-Policy` of the authorization page gains `font-src 'self'` for the bundled font. |

The pages themselves are outside the comparison (SPEC C6), so the visual
changes of patch 102 appear here only through that header and through the
steps the page driver took.

## What the comparison does not see

- **The data directory.** The patches add tables and migrations `007a` to
  `007d` to the account database (SPEC §8.2). The harness compares
  behaviour over HTTP and does not diff schemas.
- **Packaging.** The official distribution runs its server as root and lays
  the package out differently. Neither shows over HTTP: the `official`
  target compares equal to `stock` on every scenario. The image workflow's
  smoke test ([`legacy/scripts/smoke.py`](../../legacy/scripts/smoke.py))
  makes the same comparison on six requests.
