# Differences between targets

[Harness README](../README.md) · [Specification §2.2, stock equivalence](../../docs/SPEC.md#22-definition-of-compatible) · [§2.3, deliberate differences](../../docs/SPEC.md#23-deliberate-differences)

`pnpm parity compare <left> <right>` compares the transcripts of two targets.
The differences it may find are recorded here, one file per pair. A
difference that is not in the file fails the comparison, and so does a
recorded difference that no longer appears.

| Pair | File | Differences |
|---|---|---|
| `reference` against `repeat` (two boots of the reference image) | none | None is allowed. This is the check on the normaliser. |
| `reference` against `official` (the Reference against the upstream distribution `ghcr.io/bluesky-social/pds`, the same package without the patches) | [reference-vs-official.json](reference-vs-official.json) | 47, in the five groups below. |
| The Reference against the Candidate on one data directory (`pnpm parity restore`) | none | None is allowed: both servers read the same bytes, and bodies are also compared by hash. |
| The Reference against the Candidate, each on a data directory the Reference started (`pnpm parity writes`) | [reference-vs-candidate.json](reference-vs-candidate.json) | 40, in the nine groups [below](#the-reference-against-the-candidate). |
| The same, with blobs in the S3-compatible store (`pnpm parity writes --only=s3`) | [reference-s3-vs-candidate-s3.json](reference-s3-vs-candidate-s3.json) | 6, one answer in two scenarios: `getBlob` of an upload that no record references. The Reference answers 500 on this store and the Candidate 400 `InvalidRequest`, "Blob not found", as both do on the disk store. This is a deliberate difference (SPEC §2.3). |

Regenerate a file with `--update` after a change that is meant to move it,
and review the result like code.

## The Reference against the official distribution

Both targets run the same configuration: the production hostnames, the same
environment, the same fixtures. They differ in the nine patches of the
legacy build, whose rules are the table of SPEC §13 (the build itself was
removed on 2026-10-15), and in packaging, which does not show over HTTP.
Sixteen of the
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
| Shared-node import | 104 | `default/15-import` (14) | A repository whose tree references one node more than once. The production build answers 400 `InvalidRequest`, "invalid MST: node referenced more than once", and keeps the repository. The stock build answers 200 and replaces the repository with the imported tree: the record written before is gone and the head has moved. This is a deliberate difference (SPEC §2.3), observed on 2026-10-13 with a tree four levels deep. |
| Page security header | 102 | `default/21-oauth-negative` (1) | The `Content-Security-Policy` of the authorization page gains `font-src 'self'` for the bundled font. |

The pages themselves are outside stock equivalence (SPEC §2.2), so the visual
changes of patch 102 appear here only through that header and through the
steps the page driver took.

## What the comparison does not see

- **The data directory.** The patches add tables and migrations `007a` to
  `007d` to the account database (SPEC §8.2). The harness compares
  behaviour over HTTP and does not diff schemas.
- **Packaging.** The official distribution runs its server as root and lays
  the package out differently. Neither shows over HTTP: while the legacy
  build existed, its `PATCH_PROFILE=none` build compared equal to `official`
  on every scenario, which is why the recorded differences are the patches
  alone.

## The Reference against the Candidate

`pnpm parity writes` runs the write and firehose scenarios on each server
([Harness README](../README.md#the-writes-run)). The differences are the
deliberate differences of SPEC §2.3 that these scenarios reach, and one
difference in the wording of a refusal.

| Difference | Scenario | Entries | What differs |
|---|---|---|---|
| Duplicate app-password name | `default/02-sessions`, "createAppPassword with a used name" | 3 | The Reference answers 500 `InternalServerError`; the Candidate answers 400 `InvalidRequest` and names the app password. |
| `createRecord` with a record key that exists | `default/03-repo-writes`, "createRecord with a key that exists" | 3 | The Reference answers 500 `InternalServerError`; the Candidate answers 400 `InvalidRequest`, "Record already exists". |
| `applyWrites` with a create whose record key exists | `default/03-repo-writes`, "applyWrites with one invalid operation" | 3 | The same, for a batch that creates a key that exists. |
| `importRepo` with bytes that are not a CAR | `default/15-import`, "importRepo of bytes that are not a CAR" | 3 | The Reference answers 500; the Candidate answers 400 `InvalidRequest`. |
| `importRepo` with a malformed tree | `writes/22-elsewhere`, three steps | 8 | A key at the wrong layer and keys out of order across nodes: the Reference answers 200 and replaces the repository; the Candidate answers 400 and keeps it. An empty node below the root: 500 against 400, and both keep the repository. |
| `importRepo` with record blocks that are not canonical DAG-CBOR | `writes/22-elsewhere`, three steps | 9 | Map keys out of canonical order, a float with a whole value, and `undefined` in a record block: the Reference answers 200 and replaces the repository; the Candidate answers 400. |
| `importRepo` with a commit that another key signed or that names another account | `writes/22-elsewhere`, two steps | 6 | A commit that another key signed, and a commit for another account: the Reference answers 200 and replaces the repository; the Candidate answers 400. |
| A blob reference with size zero or an empty MIME type | `writes/22-elsewhere`, two steps | 4 | A record that references a blob with size zero, and one with an empty media type: the Reference answers 200; the Candidate answers 400 `InvalidRequest`. |
| Wording | `default/15-import`, "importRepo of a tree that shares nodes" | 1 | Both answer 400 `InvalidRequest`. The tree also hangs a subtree below layer zero; the Reference names the shared node and the Candidate the subtree. |

Three deliberate differences need the endpoints of unit 5: the duplicate
app-password name, an `atproto-proxy` value that does not start with a DID,
and a replayed service token. The import of a tree that references a node
more than once does not show here: the production Reference carries
upstream's fix and agrees with the Candidate. The collection of uploads that
no record references cannot be observed in a run that does not last an hour
(SPEC §2.3).

The scenarios do not reach the account events of the firehose, rate
limits, OAuth and service tokens, or the proxy. The Candidate does not have
them yet ([unit record](../../docs/research/2026-10-13-writes-and-firehose.md)
§6), and no difference file can show what no scenario asks.
