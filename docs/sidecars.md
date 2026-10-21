# Sidecars: the operations console and the MCP server

Design, 2026-10-08. Extends [SPEC.md](SPEC.md). Two containers
run beside the PDS. Neither holds a database connection or a signing key;
both talk to the PDS over authenticated APIs, so the PDS stays the single
writer and the audit log (SPEC §20, from the research record) sees every
action they take.

```
                 public hostname                    private network
  clients ──────▶ pds.linkjar.social ──▶ linkjar-pds ◀── ops API, OTLP ── console sidecar
  AI clients ───▶ mcp.linkjar.social ──▶ mcp sidecar ──▶ xrpc + oauth ──▶ linkjar-pds
```

## 1. The operations console

### 1.1 Purpose

One place for an operator to see what the PDS is doing right now and to act
on it: who is connected, which relays are crawling, where the firehose lags,
which background jobs are stuck, and the moderation and cleanup controls
that the reference server exposes only through raw XRPC calls and SQL.

### 1.2 What it shows

- **Connections.** Every live firehose subscriber with remote address, user
  agent, cursor, lag in events and seconds, bytes sent, queue depth, and the
  time of the last frame. Relay crawl status per configured relay from
  `com.atproto.sync.getHostStatus` on the relay side. Proxy upstreams
  (AppView, PLC, SMTP, identity providers) with last success, last failure
  and circuit state. Active OAuth sessions and legacy sessions per account,
  by client id and device.
- **Topology.** A live diagram of the above: the PDS in the middle, relays
  and subscribers on one side, upstreams on the other, edges coloured by
  health and labelled with lag. This is the "see the connections" view.
- **Repositories.** Per-account cards with head `rev`, MST root, record
  count, blob count and bytes, last commit time, hosting status, and the
  result of the last `verify`.
- **Jobs.** Sequencer, crawler notifications, mail outbox depth and retry
  state, scheduled deletions, blob garbage collection, DID-cache refresh,
  audit checkpointing, each with last run, duration and errors.
- **Telemetry.** Traces, metrics and logs from the PDS over OpenTelemetry,
  rendered inside the console.
- **Audit.** The §20 audit log with search, inclusion proof display and
  checkpoint verification status.

### 1.3 What it can do

Every action is an authenticated call to the PDS. Nothing bypasses it.

| Area | Actions | Backed by |
|---|---|---|
| Accounts | Takedown, suspend with expiry, reinstate, deactivate, schedule deletion, force email or handle or password update, revoke all sessions, disable invites | `com.atproto.admin.updateSubjectStatus`, `updateAccountEmail`, `updateAccountHandle`, `updateAccountPassword`, `deleteAccount`, `disableAccountInvites`, plus an ops method to revoke sessions |
| Records and blobs | Takedown a record, quarantine or unquarantine a blob, purge a blob | `updateSubjectStatus` on record and blob subjects |
| Reports | Link out to the configured external moderation service for the subject. The PDS keeps forwarding `createReport` as the reference does; the console holds no report queue (decision of 2026-10-08) | `PDS_REPORT_SERVICE_*` |
| Abuse | Block an email domain, block an IP or range for signup, revoke an invite code, raise or lower a per-account rate-limit multiplier | Ops methods; the signup gate (§12.2) reads the lists |
| Cleanup | Run blob garbage collection now, purge expired tokens and requests, run `verify` on one repository or all, run `repair` from the ladder, compact the sequencer log to the retention window | Ops methods wrapping the CLI commands |
| Firehose | Disconnect a subscriber, set a per-subscriber rate cap, emit `#sync` for a repository | Ops methods |
| Invites and signup | Create invite codes, toggle invite requirement for a window, view signup funnel | `createInviteCodes`, ops config |

Dangerous actions (delete, purge, takedown of an active account) require a
typed confirmation and a reason. The reason goes into the audit entry.

### 1.4 How it is built

- **A Rust service** (axum) that serves a server-rendered interface with
  small islands of interactivity for the live views over a WebSocket. No
  database. State lives in the PDS.
- **The PDS gains a private ops API** on its private listener
  (`PDS_OPS_ADDRESS`), JSON over HTTP plus one WebSocket stream of live
  state, authenticated by a bearer token the operator provisions. The
  methods above are its surface. Each call carries an operator identity
  that the PDS writes into the audit entry.
- **Operator sign-in** is OAuth against the PDS itself with an atproto
  account that holds an operator role, so the console never stores a
  password. The reference's basic-auth admin password remains for
  break-glass and the CLI.
- **Telemetry path.** The PDS exports OTLP over HTTP to the console's
  address. The console embeds an OTLP receiver and a small store, in the
  style of otel-desktop-viewer and otelop: in-process ingestion, embedded
  columnar storage with a size cap and retention, a trace waterfall, metric
  charts and log search, with optional forwarding of the same stream to an
  upstream collector. On the LinkJar host that upstream is the existing
  Prometheus and Grafana; the console does not replace them, it gives the
  operator the last hours at a glance beside the controls. Memory budget
  for the store is a configuration value with a default under 512 MiB.
- **Network position.** Private network only. The container publishes no
  public port. Access is through the operator's VPN or an SSH tunnel, as
  Grafana is reached today.

### 1.5 What it is not

Not a labeler and not Ozone. It moderates hosting status and content on
this PDS. If LinkJar later runs a labeling service, the console links to it.

### 1.6 State after unit 9

Built, as the crate `pds-console` and the binary `linkjar-pds-console`
(image: `Dockerfile.console`). Since 2026-10-16 the pages are rendered
with [Topcoat](https://github.com/tokio-rs/topcoat) 0.10 and have the look
of LinkJar, on the design tokens, the typeface and the mark the pages of
the PDS use (`pds_oauth::theme`); the research record of that date has
the details:

- **Sign-in.** The console is a loopback OAuth client of the PDS. The
  operator signs in on the pages of the PDS; the console keeps the session
  and its DPoP key in memory, and a restart signs every operator out. An
  account without a role gets no session.
- **Pages.** The overview with the numbers of the server, the diagram of
  the firehose subscribers and the housekeeping; the accounts with a
  search; the card of one account with its sessions and actions; the
  audit log; the blocked email domains; the roles. A sidebar names the
  server and the operator.
- **Live view.** The overview opens the stream
  `internal.ops.subscribeState` of the ops listener with the operator's
  session, and Topcoat sends each new rendering of the numbers over the
  browser's WebSocket to the console. The browser never reaches the
  listener, and the page is not reloaded.
- **Actions.** Take an account down and lift the takedown, end the
  sessions of an account, send `#sync`, disconnect a subscriber, block
  and unblock an email domain, sign a checkpoint, remove expired sessions.
  Each action opens a dialog with a field for the reason. A takedown needs
  the DID typed again.
- **Rules.** The console holds no database connection, no signing key and
  no secret of the PDS. Every form has the token of its session, and the
  console checks the origin of a form. Every page has a content security
  policy without inline script: `default-src 'none'`, scripts and styles
  from the console only, and the browser's WebSocket to the console. The
  policy allows `'unsafe-eval'` for scripts: Topcoat's browser runtime
  compiles the expressions of a page, which the server translated to
  JavaScript, with `Function`. No other script runs, and no inline script
  is allowed.

Configuration: `CONSOLE_PUBLIC_URL` (a loopback address such as
`http://127.0.0.1:2585`), `CONSOLE_PDS_PUBLIC_URL`,
`CONSOLE_PDS_INTERNAL_URL`, `CONSOLE_OPS_URL` and `CONSOLE_ADDRESS`. The
console logs its client id at start; `PDS_OPS_CLIENT_IDS` of the PDS must
hold it.

Not built:

1. The telemetry pane and the OTLP receiver. The PDS exports no OTLP yet.
2. A console under a host name. The console is a loopback client, so an
   operator reaches it through an SSH tunnel or a VPN that ends on the
   loopback address. A host name needs client metadata that the PDS can
   fetch, or a client that the PDS knows without a fetch.
3. Relay crawl status, upstream health, job state, the result of the last
   `verify` on a card, and the inclusion proof of an audit entry.
4. Record and blob takedowns, forced email, handle and password changes,
   scheduled deletion, invite codes, rate multipliers, blocked IP ranges,
   and the cleanup triggers. The ops listener serves the admin methods
   for the first five; the console has no form for them.
5. Account search by handle or DID reads the listing of the ops listener,
   which has no search; a search by email address goes to
   `com.atproto.admin.searchAccounts`, which this server does not serve
   yet.

## 2. The MCP server

### 2.1 Purpose

Let a person reach the data they own on the PDS from any AI client that
speaks the Model Context Protocol, after signing in the way they sign in
anywhere else: through the PDS's own authorization server, including the
Apple, Google and GitHub sign-in the `linkjar` profile adds. The MCP server
never sees a provider credential and never holds a password.

### 2.2 Why the PDS's OAuth server is almost already an MCP authorization server

The current MCP authorization specification (revision 2025-11-25) makes the
MCP server an OAuth 2.1 resource server, requires Protected Resource
Metadata (RFC 9728) for discovery, requires PKCE, recommends Client ID
Metadata Documents for client identification and allows dynamic client
registration, binds tokens to the resource with the RFC 8707 `resource`
parameter, and lets the resource server declare whether it requires
DPoP-bound tokens [MCP-AUTH]. AT Protocol OAuth already requires PKCE and
PAR, identifies clients by metadata document URL, and uses DPoP. The two
designs were written by people reading each other's work.

The one real gap is DPoP. The PDS issues only DPoP-bound tokens; most MCP
clients cannot produce DPoP proofs yet. So the MCP sidecar is a bridge with
two faces.

### 2.3 Two faces of the bridge

**Toward AI clients** the sidecar is an MCP server over Streamable HTTP and
an OAuth 2.1 resource server:

- `/.well-known/oauth-protected-resource` names the sidecar's own
  authorization endpoint set and declares `dpop_bound_access_tokens_required: false`,
  with DPoP accepted when a client offers it.
- The sidecar's authorization server accepts Client ID Metadata Documents
  first and dynamic client registration as a fallback, requires PKCE and
  the `resource` parameter, issues short-lived bearer access tokens scoped
  to MCP tool groups, rotates refresh tokens, and answers unauthorized
  requests with a `WWW-Authenticate` challenge that names the scopes.
- The consent step is delegated: the sidecar redirects the person to the
  PDS authorization server as an atproto OAuth client.

**Toward the PDS** the sidecar is a confidential atproto OAuth client:

- Registered by its own client metadata document, authenticating with
  `private_key_jwt`, holding a DPoP key per session.
- It requests the atproto scopes that the MCP scopes map to, using the
  permission-set and granular scope grammar the PDS supports (SPEC §5.5).
  The PDS consent page shows "LinkJar MCP" and the requested permissions
  exactly as it would for any app.
- It stores the resulting atproto session (access token, refresh token,
  DPoP key) encrypted at rest with a key from the host's secret store,
  keyed to the MCP session. Refresh is automatic; revocation on either side
  ends both.

A person therefore signs in once, on the PDS's page, with a password, a
passkey if the profile offers one, or Apple, Google or GitHub. The MCP
client receives a token it can use. The PDS account page lists the grant
under the sidecar's client name, and revoking it there revokes MCP access.

### 2.4 What the MCP server exposes

Scope groups and the atproto scopes they map to:

| MCP scope | Grants | atproto scopes |
|---|---|---|
| `account:read` | `whoami`, handle, DID document, hosting status, session list | `atproto` |
| `repo:read` | list collections, list and get records, resolve `at://` URIs, fetch blobs the person's records reference | `repo:*?action=read` equivalents per the PDS grammar |
| `repo:write:<collection>` | create, update, delete records in one collection, upload blobs for it | `repo:<collection>` |
| `sync:subscribe` | live updates for the person's own repository | `atproto` plus the firehose filtered to one DID |
| `preferences:read`, `preferences:write` | Bluesky-style preferences | the matching scope |

Tools, resources and prompts:

- **Resources.** `at://<did>/<collection>/<rkey>` as resource URIs, with
  resource templates per collection discovered from the repository. Blobs
  as `at://<did>/blob/<cid>` with the stored content type. Subscriptions
  deliver resource-updated notifications from the firehose.
- **Tools.** `list_collections`, `list_records` (cursor, limit, reverse),
  `get_record`, `search_records` (a per-session index built from the
  person's own public records, held in memory for the session only),
  `create_record`, `put_record`, `delete_record`, `apply_writes`,
  `upload_blob`, `describe_repo`, `whoami`, `export_car`.
- **Prompts.** A small set the operator can edit: summarize recent public
  bookmarks, draft a collection record from a URL, explain a record.
- **Schema-aware input.** Tool input schemas are generated from the
  vendored lexicons, so an AI client sees the real record shapes and the
  PDS validates on write as it does for any client.

### 2.5 Private data

LinkJar's private bookmarks, boards and annotations are encrypted on the
client. The hosted sidecar sees ciphertext and MUST NOT attempt to decrypt
it; the seed never leaves the person's devices. The hosted server therefore
exposes public records and the account. A second, **local** mode is the way
to reach private data: a TypeScript process in the `linkjar.io` repository
that embeds the LinkJar kit, unlocks with the person's seed on the device,
and decrypts locally. It shares only the MCP scope names with this
repository (decision of 2026-10-08). It needs its own threat review and is
named here so the hosted mode is not mistaken for the whole story.

### 2.6 Safety rules

- Every write tool is off by default per session and enabled by scope and,
  for clients the person has not marked trusted, by a per-call confirmation
  prompt that the MCP client renders.
- Rate limits per session and per account, below the PDS's own, so an AI
  client in a loop cannot exhaust the account's write budget.
- No content retention: records and blobs pass through; the session index
  dies with the session; logs carry identifiers and sizes, never record
  bodies.
- Audit: each tool call that writes is sent to the PDS with the sidecar's
  client id and the MCP client id, so the audit log attributes it.
- Safe fetch (SPEC §15.1) for untrusted MCP client metadata: public HTTPS
  on port 443, connector address validation, no redirects or environment
  proxies, a 10-second deadline and a 64 KiB body cap. Configured PDS URLs
  remain trusted destinations and may use a private network.
- The sidecar is public, so it carries the same CSP, body limits and
  fuzzed parsers as the PDS.

### 2.7 How it is built

- A Rust service on axum with an MCP implementation (the official Rust SDK
  if its Streamable HTTP server is stable at build time, otherwise a thin
  in-house JSON-RPC layer), the `pds-oauth` client half shared with the PDS
  crate, and the vendored lexicons for schema generation.
- Public hostname `mcp.<pds domain>`, TLS at the proxy, horizontal scaling
  is trivial because state is per session in an encrypted store that can
  be SQLite or Postgres through the same `AccountStore`-style seam.
- Conformance: the harness drives the official MCP TypeScript client and
  the MCP Inspector through sign-in, discovery, scope challenge, refresh and
  revocation, against both profiles.

### 2.8 State after unit 10

Built, as the crate `pds-mcp` and the binary `linkjar-pds-mcp` (image:
`Dockerfile.mcp`):

- **Toward AI clients.** Protected resource metadata and authorization
  server metadata; registration of public clients (`POST /register`) and
  Client ID Metadata Documents; the authorization code grant with PKCE
  (S256) and the `resource` parameter; bearer access tokens of one hour;
  refresh tokens that rotate, where a second use ends the session;
  revocation; a 401 challenge that names the metadata, and a 403
  challenge that names the missing scope.
- **A consent page of its own.** The PDS shows the name of the MCP
  server, not the name of the AI client, and it asks once for a client it
  authorized. The MCP server therefore asks each time: it names the AI
  client, the account and the host the browser goes to. The form is tied
  to the browser that started the request.
- **Toward the PDS.** A confidential client with `private_key_jwt` and a
  DPoP key for each session. The PDS knows it from
  `PDS_OAUTH_KNOWN_CLIENTS_FILE` or fetches its metadata.
- **MCP.** One JSON-RPC message for each POST to `/mcp`, with a JSON
  answer. Protocol revisions `2025-11-25`, `2025-06-18` and `2025-03-26`.
  Eight tools: `whoami`, `describe_repo`, `list_collections`,
  `list_records`, `get_record`, `create_record`, `put_record`,
  `delete_record`. One resource template, `at://<did>/{collection}/{rkey}`.
  Two prompts.
- **Scopes.** `account:read`, `repo:read`, `repo:write`, and
  `repo:write:<collection>` for each collection of
  `MCP_WRITE_COLLECTIONS`.
- **Limits.** 120 calls and 20 writes in a minute for each session.
  Public registration admits 30 attempts per minute across the process,
  including invalid requests; excess returns 429 `temporarily_unavailable`
  with `Retry-After: 60`. At 4,096 registered clients, new registrations
  return 503 `temporarily_unavailable`. Existing registrations are retained
  until restart; filling the table does not evict them.
- **Audit.** Each write goes to the PDS with the id of the AI client,
  and the PDS writes the entry (SPEC §22.2).

Configuration: `MCP_PUBLIC_URL`, `MCP_PDS_PUBLIC_URL`,
`MCP_PDS_INTERNAL_URL`, `MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX`,
`MCP_WRITE_COLLECTIONS` and `MCP_ADDRESS`.

Not built:

1. A store that outlives the process. Clients, sessions and the sessions
   at the PDS are in memory: a restart ends every session, and the server
   runs as one instance.
2. `search_records`, `apply_writes`, `upload_blob` and `export_car`; blob
   resources; `sync:subscribe` and resource subscriptions; the
   preferences scopes.
3. Input schemas from the lexicons. A record is an object, and the PDS
   validates it.
4. The confirmation prompt for each write of a client that is not
   trusted. A write tool says in its annotations that it writes.
5. DPoP for an AI client that offers it. Tokens are bearer tokens.
6. A scope step-up for the official client. On `insufficient_scope` the
   SDK 1.30.1 refreshes with the scope it has, so a client that needs a
   wider scope signs in again. The official client and the Inspector run
   against the server in CI (`the_official_mcp_client_signs_in_through_the_pds`).
8. The local mode for private data, which belongs to `linkjar.io`.

## 3. What this adds to the SPEC and the plan

| SPEC | Change |
|---|---|
| §4.1 | The private ops listener and its WebSocket; `PDS_OPS_ADDRESS` and `PDS_OTLP_ENDPOINT` in Appendix C. |
| §12 | Operator roles as an extension point: who may call ops methods, with the stock implementation being the admin password and the `linkjar` profile adding role-bearing accounts. |
| §14.4 | OTLP export of traces, metrics and logs in addition to the Prometheus listener. |
| §20 | Operator identity, reason and the acting sidecar recorded in every audit entry. |
| New §22 | Sidecars: the ops API contract, the MCP bridge contract, the scope map, and the rule that sidecars hold no keys and no database access. |

Units: the console lands as unit 9, after the audit log of unit 7 and the
cutover of unit 8, because it needs the ops API and the audit log; the MCP
server lands as unit 10, because it needs the
OAuth server and the `linkjar` profile's sign-in; the local MCP mode lives in `linkjar.io` as a
separate later unit. Allowances: three weeks each for the console and the
hosted MCP server, low confidence.

## Sources

- [MCP-AUTH] [MCP authorization specification, 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization); the 2026-07-28 tutorial on authorization confirms the same model.
- [otel-desktop-viewer](https://github.com/CtrlSpice/otel-desktop-viewer/) and [otelop](https://github.com/mashiro/otelop/): single-binary OTLP receivers with an embedded DuckDB store and a web UI; the pattern the console follows for its telemetry pane.
- [OpenObserve](https://github.com/openobserve/openobserve/) and [SigNoz](https://github.com/SigNoz/signoz): full platforms, kept as optional upstreams; a 2026 measurement puts OpenObserve near 310 MiB idle in one container and SigNoz near 1.6 GiB across four [VPS-GUIDE].
- [VPS-GUIDE] [Self-host SigNoz and OpenObserve on a VPS](https://www.virtua.cloud/learn/en/tutorials/self-host-signoz-openobserve-vps)
- Reference PDS admin surface at the pin: `packages/pds/src/api/com/atproto/admin/` in the scratch checkout.
