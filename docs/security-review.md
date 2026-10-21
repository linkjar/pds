# Security, import, and performance review

Initial review: 2026-10-19. Follow-up: 2026-10-21.
Baseline: `77bf09e2e4e94079c38b86e47fb2d84e6ed36bbc`.

Changes are on `codex/security-import-hardening` in the isolated `linkjar-pds-security` worktree. Nothing has been deployed. Production remains unchanged.

## Changes prepared for review

| Area | Result |
| --- | --- |
| Browser sessions | A rotated cookie accepts only its exact predecessor during a fixed five-second overlap. An unrelated cookie cannot take over or delete a remembered device. |
| Account recovery | Password reset and admin password update revoke legacy and OAuth credentials, app passwords, remembered sign-ins, pending authorizations, client grants, and email tokens. Shared account transactions prevent concurrent credential grants from surviving recovery. Members confirm current token status with their entryway. |
| Recovery races | Regression coverage includes concurrent login, app-password creation, email changes, account deletion, delayed PLC signing, one-time reset-token reuse, and stale consent proofs. Recovery markers survive restarts, signing-key rotation, and SQLite/PostgreSQL copies. |
| Login and recovery responses | Identifier case changes cannot bypass the identifier throttle. Password-reset requests return the same public result for known and unknown addresses. |
| Outbound requests | Untrusted destinations require public HTTPS on port 443. Address validation runs in the connector's DNS resolver. Redirects and environment proxies are disabled. Metadata and buffered service responses have byte limits and deadlines. Proxy authentication precedes DID resolution. |
| Resource exhaustion | Uploads, imports, exports, blocking jobs, audit workers, and firehose connections have separate budgets. Request bodies and response producers have idle and total deadlines. Export/firehose pages enforce byte budgets before copying large bodies. |
| Imports and reads | Imports preserve record blocks still used by other keys, support moved records, replace stale blob references, and remove newly unreferenced blob files after commit. Repository proof/export reads use stable snapshots. |
| MCP registration | Registration requests are rate limited. A full registration store refuses new clients instead of evicting existing clients. |
| Reverse proxy image | The Compose template and parity image pin use verified Caddy 2.11.7 image bytes. The configuration validates locally. |

The Caddy pin addresses the affected version range in [GHSA-8qx7-j2v2-x6q5](https://github.com/caddyserver/caddy/security/advisories/GHSA-8qx7-j2v2-x6q5). The selected version is documented in the [2.11.7 release](https://github.com/caddyserver/caddy/releases/tag/v2.11.7).

## Performance

Added `record_blob_record_uri_idx(recordUri)`. New stores create it; existing actor stores add it on their first writable open. Reference migration entries are unchanged.

Three local release measurements with 100,000 blob references reduced the deletion query's median from **6.777–7.158 ms to 6.458–6.542 µs**. The index added about **7.0 MiB**, took **65.535–72.945 ms** to create, and increased the median cost of inserting 100 references by **25–28%**.

These are synthetic SQL measurements, not an estimate of production throughput. See the [method, raw results, and limits](performance/record-blob-index.md).

The original three paired HTTP runs showed comparable anonymous read performance. The follow-up review found that their copied fixture schema omitted existing production indexes. Those absolute timings, especially `describeRepo`, do not represent the production schema. The [HTTP report](performance/http-reads.md) records the correction and links the replacement measurements.

## Initial validation, 2026-10-19

- `cargo test --workspace --locked`: **483 passed**, no failures, 3 ignored manual measurements. PostgreSQL, the official MCP client, and MCP Inspector were enabled. The HTTP timing measurement was run separately in release mode.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `cargo check --manifest-path fuzz/Cargo.toml --locked`: passed.
- `cargo xtask codegen --check`: passed; 263 lexicons remain current.
- `cargo audit --deny warnings`: no reported vulnerabilities or warnings among 404 locked dependencies. Advisory database commit `999ec89d5d3f10f20be14750ec8c2d8b44e0ac9b`, updated 2026-10-17.
- `cargo deny --all-features --locked check` and `cargo vet --locked`: passed under the repository's existing policies and exemptions. Existing duplicate-version notices remain.
- Documentation site: 24 pages built. Existing i18n/404 warnings remain. Changed architecture diagrams were rendered in both themes.
- The shipped Caddy configuration validated against the pinned 2.11.7 image in a temporary local container.

The full external differential parity matrix was not run. Local regression tests and the existing workspace protocol fixtures passed.

## Performance and code organization follow-up, 2026-10-21

- Blob cleanup uses indexed existence checks and deduplicates candidate CIDs. It no longer loads every surviving association. Repeated fixed SQL uses cached statements.
- Repository writes wait in a bounded asynchronous queue before taking a blocking worker. One busy account can admit eight active or waiting operations, within a process-wide limit of 64. Cancellation retains the permit until a running operation finishes. The synchronous repository lock remains for maintenance and simulation coordination.
- Record decoding and JSON encoding run inside the admitted blocking job. Database connections return to the pool before conversion. Response bytes and content types remain covered by regression tests.
- Legacy authentication reuses verified claims only inside the current request, for the same token, configuration, and verification second. Revocation checks remain live. The storage cache records only confirmed schema presence, never a missing table or a recovery-marker value.
- Pure OAuth reads use pooled connections. Related reads share a consistent SQLite or PostgreSQL snapshot. Final credential checks remain inside write transactions.
- Legacy session reads reuse the account connection pool instead of opening a SQLite connection per request. Each request still reads the current account and recovery marker.
- Two recovery races now use deterministic synchronization. Both tests failed when their final guards were deliberately omitted, then passed after exact source restoration.
- Large auth, configuration, address-policy, and lexicon test modules moved into separate files. Account and OAuth integration scenarios share HTTP fixtures and use focused child modules. Small private unit tests remain under Rust's `#[cfg(test)]`; they do not ship in the production binary.
- Site type checking now has pinned tools. CI checks site types and verifies every architecture SVG against its source, including missing and obsolete files.
- The documentation build pins the affected CSS parser to `7.1.6` for [GHSA-rj75-hqrm-r3gf](https://github.com/advisories/GHSA-rj75-hqrm-r3gf). The site audit reports no known vulnerabilities. All four generated CSS files are byte-identical after the update. This dependency is outside the PDS runtime.
- A fresh Rust advisory scan found three TLS advisories published after the initial scan: `RUSTSEC-2026-0381`, `RUSTSEC-2026-0382`, and `RUSTSEC-2026-0383`. Workspace and fuzz lockfiles now use `rustls-webpki 0.103.16` and `webpki-root-certs 1.0.10`; the workspace also uses `webpki-roots 1.0.10`. These updates [reject distrusted intermediate public keys](https://github.com/rustls/webpki/pull/546) and [remove the affected root certificate](https://github.com/rustls/webpki-roots/pull/133). The refreshed audit reports no remaining advisories.

The HTTP measurement fixture now opens the production schema and verifies its indexes and WAL mode. The old fixture remains available for independent Reference compatibility tests.

Across three runs with identical mixed-workload fixtures, small-read p99 latency during large-page traffic fell from **2.725–2.814 ms to 0.382–0.410 ms**. Small-read throughput increased from **13.07–13.47 thousand to 20.83–21.63 thousand requests/s**. This comparison includes the full security and performance patch; it does not isolate one change. Separately, changing only the session account connection to use the pool reduced median `getSession` latency from **2.040–2.130 ms to 0.155 ms**, with live recovery checks retained. The [mixed-workload report](performance/mixed-reads.md) records the baseline, intermediate control, final candidate, and measurement limits.

Three isolated SQL runs reduced the median shared-blob check from **9.955–10.043 ms to 0.542 µs**, returning one CID instead of 100,000 duplicates. Checking 500 distinct referenced blobs fell from **329–335 µs to 190–196 µs**. Two hundred record-CID lookups fell from **233–246 µs to 62–64 µs**. The [SQL report](performance/blob-references.md) includes raw results, process isolation, and the superseded exploratory comparison.

The early anonymous HTTP comparison recorded **2.4–8.0% lower** concurrent 50-record list throughput in three short paired runs. A [longer follow-up](performance/small-page-followup.md) did not reproduce a consistent slowdown: 21 paired five-second measurements had a **0.89% higher median throughput**, with a conservative interval for the paired median of **−0.42% to +3.66%** under the stated independence assumption. The current secured version passed the predeclared local comparison gate without production-code changes. The original short samples do not establish a recurring 2–8% cost, and neither comparison establishes a general HTTP speedup or production capacity.

## Follow-up validation, 2026-10-21

- `cargo test --workspace --locked`: **492 passed**, no failures, 4 ignored manual/demo tests. PostgreSQL, the official MCP client, and MCP Inspector were enabled. This run includes the patched TLS dependencies. After the final session connection-pool change, all **50 account, OAuth, and session integration tests** passed again with PostgreSQL enabled. Release measurements run separately.
- Full workspace Clippy, including all targets and features with warnings denied: passed.
- Formatting, diff checks, fuzz-target compilation, and generated-code checks: passed. All 263 generated lexicons remain current.
- `cargo audit --deny warnings`: passed for 404 locked dependencies against advisory database commit `4b587f00996ebe5c05a73eaef105ef241e876414`.
- `cargo deny --all-features --locked check`: passed. Existing duplicate-version notices remain.
- `cargo vet --locked`: passed with three reviewed patch deltas and 383 existing exemptions. [The delta records](../supply-chain/audits.toml) identify their AI review, upstream evidence, and limits. No new exemptions were added; this is not a full audit of every dependency.
- Site type check: no errors, warnings, or hints. The 24-page build and dependency audit passed. Existing build-time i18n/404 warnings remain.
- All 38 architecture SVGs match their sources. Deliberately stale, missing, and obsolete SVGs failed the read-only check in a disposable copy.
- Small-page follow-up: all 42 confirmatory benchmark processes passed with identical response digests and valid throughput metadata. The new standalone benchmark passed focused Clippy and formatting checks. Runtime source and dependency files remain identical to the previously validated version; the security suite was not rerun for benchmark-only changes.

All tests used local fixtures and temporary services. The full external differential parity matrix and production load tests were not run.

## PR integration validation, 2026-10-21

The branch was rebased onto `e5acc8a2d8ab5a3d6b559e9e041b109f9a380281`, retaining the latest rate-limit fix that preserves OAuth nonce exposure headers. Its browser regression moved unchanged into the OAuth authorization test module. The production rate-limit file matches that upstream commit.

- The full workspace suite passed again: **494 passed**, no failures, **5 ignored** manual/demo measurements. PostgreSQL, the official MCP client, and MCP Inspector were enabled. Both upstream rate-limit regression tests passed.
- Full workspace Clippy, including all targets and features with warnings denied, passed again. Formatting and staged diff checks passed.
- Dependency lockfiles and the documentation build inputs are unchanged from the earlier validated patch. Benchmark reports retain their recorded pre-rebase source snapshots; measurements were not rerun for this integration.

Nothing has been deployed. The temporary PostgreSQL test container was removed after validation.

## Review before rollout

- Imports default to a finite 64 MiB limit. Larger accepted imports require an explicit positive `PDS_MAX_REPO_IMPORT_SIZE`.
- Request bodies and export producers have a 15-second idle limit and a five-minute total limit. Refused bodies drain for at most five seconds. Capacity exhaustion returns `503 ServerBusy`; slow request bodies return `408 RequestTimeout`.
- Resource budgets are per process: 8 uploads, 2 imports, 8 exports, 32 firehose connections, 64 ordinary blocking jobs, and 4 audit workers. Audit writes queue asynchronously.
- Repository operations additionally admit eight active or waiting jobs per DID and 64 overall. Waiters use no blocking worker. Queue exhaustion returns `503 ServerBusy`; running jobs retain admission through cancellation.
- Production untrusted fetches reject private destinations, HTTP, custom ports, and redirects. Explicitly configured services remain trusted destinations. Development exceptions require development mode.
- A member requires an upgraded entryway and `PDS_ENTRYWAY_ADMIN_TOKEN`. Every legacy/OAuth authenticated member request adds an uncached status round trip. Failure to confirm status returns 503. Review this availability dependency and rollout order.
- The additive `credential_epoch` account table is security state. Preserve it in backups and database copies. An older server or Reference binary does not enforce it; restoring older credentials can restore older access. See the [rollback guidance](../site/src/content/docs/operate/update.md).
- Index creation takes a SQLite write lock and needs disk space. Measure first-open cost on representative large stores before rollout.
- Import fixes apply to future imports. They do not scan or repair historical stale references. Physical blob deletion remains best effort with logged failures; there is no new durable deletion retry queue.

This work validates the identified code paths. It is not a guarantee that the service has no remaining vulnerabilities. No production load test, deployment, or historical-data repair was performed.
