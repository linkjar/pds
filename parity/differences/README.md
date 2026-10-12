# Differences between targets

[Harness README](../README.md) · [Specification §2.2, C6](../../docs/SPEC.md#22-definition-of-compatible) · [§2.3, deliberate differences](../../docs/SPEC.md#23-deliberate-differences)

`pnpm parity compare <left> <right>` compares the transcripts of two targets.
The differences it may find are recorded here, one file per pair. A
difference that is not in the file fails the comparison, and so does a
recorded difference that no longer appears.

| Pair | File | Differences |
|---|---|---|
| `reference` against `repeat` (two boots of the reference image) | none | None is allowed. This is the check on the normaliser. |
| `reference` against `stock` (the production build against the `PATCH_PROFILE=none` build) | [reference-vs-stock.json](reference-vs-stock.json) | 47, in the five groups below. |
| `stock` against `official` (the `PATCH_PROFILE=none` build against the upstream distribution `ghcr.io/bluesky-social/pds`) | none | None is allowed: the two are different packagings of the same package version. |
| The Reference against the Candidate on one data directory (`pnpm parity restore`) | none | None is allowed: both servers read the same bytes, and bodies are also compared by hash. |
| `reference` against `candidate`, each on its own stack | `reference-vs-candidate.json`, written when the Candidate first answers the write scenarios (unit 4) | The deliberate differences [below](#the-reference-against-the-candidate), and no others. |

Regenerate a file with `--update` after a change that is meant to move it,
and review the result like code.

## The production build against the stock build

Both targets run the same configuration: the production hostnames, the same
environment, the same fixtures. They differ in the nine patches of
[`legacy/patches/`](../../legacy/patches/README.md). Sixteen of the
twenty-six scenarios show no difference at all, and in three more the only
difference is the text of a mail or one page header. The patches do not
change the repository, the firehose, proxying, rate limits, legacy sessions,
handles and PLC operations of password accounts, account status, admin
methods, invite administration, or the OAuth protocol.

| Group | Patch | Scenarios | What differs |
|---|---|---|---|
| LinkJar behaviour that a stock build does not have | 093, 099, 099b, 100, 102, 103 | `default/30-linkjar-external`, `31-linkjar-handles`, `32-linkjar-signup-journey`, `33-linkjar-signin-methods`, `invites/02-linkjar-invite-handoff` | The scenario is skipped on the stock build, so each is one recorded difference. On the production build they pass: external sign-in, derived handles, the creation receipt, the hosted handle policy and handle hosts, the sign-up journey, sign-in methods with their notices, and the invitation hand-off. |
| Signup policy | 099c | `captcha/01-signup-policy` (22 differences) | With one of the three hCaptcha variables missing, the production build refuses to start and says "Partial hCaptcha config"; the stock build reads it as no hCaptcha and starts. With hCaptcha configured, the production build answers 400 "Create an account through the OAuth signup page" to a legacy `createAccount`; the stock build creates the account, which bypasses the CAPTCHA of the sign-up page. |
| Account mail | 101 | `default/10-email` (4), `default/11-identity` (1) | The text of the five token mails: confirm email, update email, reset password, delete account, PLC operation. Subjects, sender and the presence of a text part are the same. The sixth template, the sign-in notice, exists only in the production build. |
| Shared-node import | 104 | `default/15-import` (14) | A repository whose tree references one node more than once. The production build answers 400 `InvalidRequest`, "invalid MST: node referenced more than once", and keeps the repository. The stock build answers 200 and replaces the repository with the imported tree: the record written before is gone and the head has moved. This is SPEC §2.3, DD-6, observed on 2026-10-13 with a tree four levels deep. |
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

## The Reference against the Candidate

The Candidate does not answer the write scenarios yet, so it has no
transcript of its own stack (unit 4). When it does, its comparison with the
Reference on separate stacks may show the deliberate differences of SPEC
§2.3 and nothing else. Each one is one step of one scenario.

| Id | Scenario | Step | Reference | Candidate |
|---|---|---|---|---|
| DD-1 | `default/03-repo-writes` | createRecord with a key that exists | 500 `InternalServerError` | 400 `InvalidRequest` |
| DD-2 | `default/03-repo-writes` | applyWrites with one invalid operation | 500 `InternalServerError` | 400 `InvalidRequest` |
| DD-3 | `default/02-sessions` | createAppPassword with a used name | 500 `InternalServerError` | 400 `InvalidRequest` |
| DD-4 | `default/08-proxy` | proxy target that is not a DID | 500 `InternalServerError` | 400 `InvalidRequest` |
| DD-5 | `default/08-proxy` | uploadBlob with the same service token again | 200 | 401 `BadJwt` |

SPEC §2.3 also lists DD-6 and DD-7 to DD-13. DD-6 shows against the stock
build only: the production Reference, with patch 104, answers as the
Candidate. It is recorded in [reference-vs-stock.json](reference-vs-stock.json).
The scenario steps of DD-7 to DD-13 are named here when units 4, 5 and 8 add
them.

The owner decided DD-1 to DD-5 on 2026-10-09 ([plan.md](../../docs/plan.md#decisions-taken-2026-10-09)).
