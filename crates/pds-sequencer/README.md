# pds-sequencer

The event log and the firehose: the `repo_seq` rows, the frames of the
`#commit`, `#sync`, `#identity` and `#account` events, subscriptions with a
cursor and backfill, and the `requestCrawl` notices to relays. Frames are
built from stored rows only. The WebSocket route is pds-xrpc's, the schema
is pds-storage's, and the shape of a commit is pds-repo's.

## Modules

| Module | Holds |
|---|---|
| `sequencer` | `Sequencer`, the single writer of the log with a shared frame cache; `Subscription`, which pages one subscriber; `Refusal`, `SequencerError`, `Frame`; `iso_millis`, the timestamp format the server uses throughout. |
| `events` | `commit_event`, `sync_event`, `identity_event`, `account_event`, `CommitOp`, `event_rev`; the info and error frames; the 5 MB frame cap. |
| `crawlers` | `Crawlers`: `requestCrawl` to the configured relays, at most once per twenty minutes. |

## Place in the workspace

Depends on pds-types, pds-repo and pds-storage. Used by pds-xrpc,
linkjar-pds and pds-sim.

## What it needs

`sequencer.sqlite` through `DataLayout`. The caller passes the values of
`PDS_HOSTNAME`, `PDS_CRAWLERS`, `PDS_REPO_BACKFILL_LIMIT_MS` and
`PDS_MAX_SUBSCRIPTION_BUFFER`. The crawler notices are HTTP calls with a
thirty-second timeout.

## Tests

`cargo test -p pds-sequencer`. `tests/subscription.rs` covers cursor
replay, an outdated cursor, a subscriber that falls too far behind, and
backfill pages bounded by bytes, on a temporary SQLite file with no
crawlers.

## Specification

SPEC §7 (§7.1 to §7.6) and §3.2.
