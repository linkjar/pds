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
| The Reference against the Candidate, each on a data directory the Reference started (`pnpm parity writes`) | [reference-vs-candidate.json](reference-vs-candidate.json) | 37, in the eight groups [below](#the-reference-against-the-candidate). |

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

`pnpm parity writes` runs the write and firehose scenarios on each server
([Harness README](../README.md#the-writes-run)). The differences are the
deliberate differences of SPEC §2.3 that these scenarios reach, and one
difference in the wording of a refusal.

| Difference | Scenario | Entries | What differs |
|---|---|---|---|
| DD-1 | `default/03-repo-writes`, "createRecord with a key that exists" | 3 | The Reference answers 500 `InternalServerError`; the Candidate answers 400 `InvalidRequest`, "Record already exists". |
| DD-2 | `default/03-repo-writes`, "applyWrites with one invalid operation" | 3 | The same, for a batch that creates a key that exists. |
| DD-15 | `default/15-import`, "importRepo of bytes that are not a CAR" | 3 | The Reference answers 500; the Candidate answers 400 `InvalidRequest`. |
| DD-7 | `writes/22-elsewhere`, three steps | 8 | A key at the wrong layer and keys out of order across nodes: the Reference answers 200 and replaces the repository; the Candidate answers 400 and keeps it. An empty node below the root: 500 against 400, and both keep the repository. |
| DD-8 | `writes/22-elsewhere`, three steps | 9 | Map keys out of canonical order, a float with a whole value, and `undefined` in a record block: the Reference answers 200 and replaces the repository; the Candidate answers 400. |
| DD-14 | `writes/22-elsewhere`, two steps | 6 | A commit that another key signed, and a commit for another account: the Reference answers 200 and replaces the repository; the Candidate answers 400. |
| DD-9 | `writes/22-elsewhere`, two steps | 4 | A record that references a blob with size zero, and one with an empty media type: the Reference answers 200; the Candidate answers 400 `InvalidRequest`. |
| Wording | `default/15-import`, "importRepo of a tree that shares nodes" | 1 | Both answer 400 `InvalidRequest`. The tree also hangs a subtree below layer zero; the Reference names the shared node and the Candidate the subtree. |

DD-3 to DD-5 need the endpoints of unit 5. DD-6 does not show here: the
production Reference carries upstream's fix and agrees with the Candidate.
DD-10 cannot be observed in a run that does not last an hour (SPEC §2.3).

The scenarios do not reach the account events of the firehose, rate
limits, OAuth and service tokens, or the proxy. The Candidate does not have
them yet ([unit record](../../docs/research/2026-10-13-writes-and-firehose.md)
§6), and no difference file can show what no scenario asks.
