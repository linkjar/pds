# Architecture

[Documentation](../README.md) · [Specification](../SPEC.md) · [Plan](../plan.md)

One diagram per part of the system. Each diagram names the crate or file
that implements it and the SPEC section that governs it. A change to the
code updates the diagram in the same commit; a diagram that drifts is a
bug. Diagrams are Mermaid and render on GitHub.

Legend used throughout: blue is outside the host, amber is the PDS process,
green is a sidecar, grey is storage, red is an extension point.

```mermaid
flowchart LR
  classDef ext fill:#eef3ff,stroke:#4a6cf7,color:#1b2b6b
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef side fill:#eafaf1,stroke:#2e9e5b,color:#0f4d2a
  classDef store fill:#f4f4f5,stroke:#71717a,color:#27272a
  classDef hook fill:#fdeaea,stroke:#d64545,color:#6b1515
  a["outside the host"]:::ext --- b["PDS process"]:::pds --- c["sidecar"]:::side --- d[("storage")]:::store --- e["extension point"]:::hook
```

## Contents

- [System context](#system-context)
- [Crates](#crates)
- [Process and listeners](#process-and-listeners)
- [Write path](#write-path)
- [Sequencer and firehose](#sequencer-and-firehose)
- [Storage layout and seams](#storage-layout-and-seams)
- [Identity](#identity)
- [OAuth: stock sign-in](#oauth-stock-sign-in)
- [OAuth: LinkJar external sign-in](#oauth-linkjar-external-sign-in)
- [Extension model](#extension-model)
- [Audit log](#audit-log)
- [Simulation harness](#simulation-harness)
- [Parity harness](#parity-harness)
- [Deployment topologies](#deployment-topologies)
- [Sidecars](#sidecars)
- [Cutover and rollback](#cutover-and-rollback)
- [Repository and build](#repository-and-build)

## System context

SPEC §1, §21.1. Who talks to the PDS and over what.

```mermaid
flowchart TB
  classDef ext fill:#eef3ff,stroke:#4a6cf7,color:#1b2b6b
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef side fill:#eafaf1,stroke:#2e9e5b,color:#0f4d2a
  classDef store fill:#f4f4f5,stroke:#71717a,color:#27272a

  subgraph clients["Clients"]
    app["LinkJar web app"]:::ext
    ios["iOS and Mac apps"]:::ext
    ext["Browser extension"]:::ext
    bsky["Bluesky app, any atproto client"]:::ext
    ai["AI clients (MCP)"]:::ext
  end
  subgraph network["Network services"]
    relay["Relays"]:::ext
    appview["AppView"]:::ext
    plc["PLC directory"]:::ext
    dns["DNS"]:::ext
    idp["Apple · Google · GitHub"]:::ext
    smtp["SMTP"]:::ext
  end
  subgraph host["Host: pds.linkjar.social"]
    proxy["Reverse proxy (TLS)"]:::ext
    pds["linkjar-pds"]:::pds
    data[("data directory")]:::store
    blobs[("blobs: disk or S3")]:::store
    console["ops console"]:::side
    mcp["MCP server"]:::side
    otel["Prometheus · Grafana"]:::ext
  end

  app & ios & ext & bsky -- "XRPC, OAuth, WebSocket" --> proxy --> pds
  ai -- "MCP over HTTP" --> mcp -- "OAuth client, XRPC" --> pds
  relay -- "subscribeRepos, getRepo" --> proxy
  pds -- "requestCrawl" --> relay
  pds -- "proxied app.bsky.*" --> appview
  pds -- "resolve, submit ops" --> plc
  pds -- "handle TXT lookups" --> dns
  pds -- "sign-in (linkjar profile)" --> idp
  pds -- "token and notice mail" --> smtp
  pds --- data & blobs
  console -- "ops listener, OTLP" --> pds
  pds -- "metrics" --> otel
```

## Crates

SPEC §3.1. Arrows point from a crate to the crates it depends on. The
binary wires everything; extension crates depend only on the trait crate.

```mermaid
flowchart BT
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef hook fill:#fdeaea,stroke:#d64545,color:#6b1515
  classDef tool fill:#f4f4f5,stroke:#71717a,color:#27272a

  types["pds-types<br/>syntax, data model, generated API types"]:::pds
  lexicon["pds-lexicon<br/>lexicon model, validation, permission sets"]:::pds
  repo["pds-repo<br/>MST, commits, CAR, diffs"]:::pds
  storage["pds-storage<br/>seams + SQLite impls, StorageIo"]:::pds
  blob["pds-blob"]:::pds
  identity["pds-identity<br/>DID, handle, PLC, keys"]:::pds
  sequencer["pds-sequencer<br/>event log, firehose"]:::pds
  account["pds-account<br/>accounts, sessions, mail"]:::pds
  oauth["pds-oauth<br/>authorization server, pages"]:::pds
  audit["pds-audit<br/>chain, checkpoints, verifier"]:::pds
  xrpc["pds-xrpc<br/>routing, limits, proxy"]:::pds
  ext["pds-ext<br/>extension traits, stock profile"]:::hook
  linkjar["pds-ext-linkjar<br/>linkjar profile"]:::hook
  sim["pds-sim<br/>fault VFS, fail points, turmoil"]:::tool
  bin["linkjar-pds<br/>binary, CLI, ops listener"]:::pds
  xtask["xtask<br/>codegen"]:::tool

  lexicon --> types
  repo --> types
  storage --> types & repo
  blob --> storage
  identity --> types & storage
  sequencer --> repo & storage
  account --> storage & identity & ext & audit
  oauth --> account & lexicon & ext
  audit --> storage & repo
  xrpc --> types & lexicon & repo & sequencer & account & oauth & blob
  ext --> types & storage
  linkjar --> ext
  sim --> storage & sequencer
  bin --> xrpc & linkjar & sim
  xtask --> lexicon
```

## Process and listeners

SPEC §3.2, §4.1, §14.3. One process, three listeners, supervised background
tasks, injectable time and randomness.

```mermaid
flowchart LR
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef ext fill:#eef3ff,stroke:#4a6cf7,color:#1b2b6b
  classDef store fill:#f4f4f5,stroke:#71717a,color:#27272a

  subgraph proc["linkjar-pds process (tokio)"]
    pub["public listener<br/>PDS_PORT<br/>/xrpc, /oauth, /account, /.well-known"]:::pds
    met["metrics listener<br/>PDS_METRICS_ADDRESS"]:::pds
    ops["ops listener<br/>PDS_OPS_ADDRESS<br/>internal.* + live WebSocket"]:::pds
    subgraph bg["supervised background tasks"]
      t1["crawler notify"]:::pds
      t2["scheduled deletion"]:::pds
      t3["blob GC"]:::pds
      t4["mail outbox"]:::pds
      t5["DID cache refresh"]:::pds
      t6["audit checkpoint"]:::pds
      t7["permission-set cache"]:::pds
    end
    prov["providers: clock · random · StorageIo"]:::store
    pool["blocking pool: SQLite"]:::store
  end
  proxy["reverse proxy"]:::ext --> pub
  console["console"]:::ext --> ops & met
  bg --- prov
  pub --- pool
```

## Write path

SPEC §6.3, §6.4, §7.4. What happens on `createRecord` and why the sequencer
row is durable with the commit.

```mermaid
sequenceDiagram
  autonumber
  participant C as Client
  participant X as pds-xrpc
  participant A as auth (session / DPoP)
  participant L as pds-lexicon
  participant S as ActorStore (SQLite per actor)
  participant R as pds-repo
  participant Q as SequencerStore
  participant F as firehose fan-out

  C->>X: POST /xrpc/com.atproto.repo.createRecord
  X->>A: verify token, scope, active account, rate limit
  A-->>X: DID, permissions
  X->>L: validate record (mode: true / false / unset)
  L-->>X: ok + validation flag
  X->>S: begin write transaction (one writer per actor)
  S->>R: load MST root, apply op incrementally
  R-->>S: new blocks, new root, diff CAR slice
  S->>S: insert blocks, record row, blob refs
  S->>Q: append #commit (seq, ops, blocks, since, rev, prevData)
  Q-->>S: durable in the same step
  S-->>X: commit (cid, rev)
  X->>F: wakeup (no frame on the bus)
  F->>Q: read frames in seq order
  X-->>C: 200 { uri, cid, commit, validationStatus }
```

## Sequencer and firehose

SPEC §7. Frames come only from the durable log; the broadcast is a wakeup.

```mermaid
flowchart LR
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef ext fill:#eef3ff,stroke:#4a6cf7,color:#1b2b6b
  classDef store fill:#f4f4f5,stroke:#71717a,color:#27272a

  commit["commit path"]:::pds -- "append" --> log[("repo_seq<br/>sequencer.sqlite")]:::store
  commit -- "wakeup" --> bus["broadcast (signal only)"]:::pds
  bus --> sub1 & sub2
  subgraph subs["per-subscriber tasks"]
    sub1["subscriber A<br/>bounded queue"]:::pds
    sub2["subscriber B<br/>bounded queue"]:::pds
  end
  log -- "page reads in seq order<br/>backfill then live" --> sub1 & sub2
  sub1 -- "frames ≤ 5 MB" --> relayA["relay"]:::ext
  sub2 --> relayB["indexer"]:::ext
  startup["startup repair<br/>head ahead of last seq → #sync"]:::pds --> log
```

Cursor rules (SPEC §7.3): no cursor → live; `0` → whole window; in window →
backfill then live; above head → `FutureCursor`; older than the window →
`#info OutdatedCursor`; slow consumer → `ConsumerTooSlow`.

## Storage layout and seams

SPEC §8. The reference layout on disk, the Candidate-only files beside it,
and the traits every subsystem goes through.

```mermaid
flowchart TB
  classDef store fill:#f4f4f5,stroke:#71717a,color:#27272a
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef new fill:#fdeaea,stroke:#d64545,color:#6b1515

  subgraph seams["pds-storage seams (traits)"]
    as["ActorStore"]:::pds
    acc["AccountStore"]:::pds
    seq["SequencerStore"]:::pds
    bs["BlobStore"]:::pds
    dc["DidCache"]:::pds
    au["AuditStore"]:::pds
    io["StorageIo<br/>(OS files, or fault VFS in tests)"]:::pds
  end
  subgraph dir["PDS_DATA_DIRECTORY (reference layout)"]
    a[("account.sqlite")]:::store
    s[("sequencer.sqlite")]:::store
    d[("did_cache.sqlite")]:::store
    audit[("audit.sqlite · Candidate-only")]:::new
    opsdb[("ops.sqlite · Candidate-only")]:::new
    subgraph actors["actors/&lt;2 hex&gt;/&lt;did&gt;/"]
      st[("store.sqlite")]:::store
      key["key (signing key)"]:::store
    end
    rk["actors/reserved_keys/"]:::store
  end
  subgraph blobs["PDS_BLOBSTORE_DISK_LOCATION or S3"]
    b[("&lt;did&gt;/&lt;cid&gt;")]:::store
    tmp["tempt/&lt;did&gt;/"]:::store
    q["quarantine/&lt;did&gt;/"]:::store
  end
  as --- st & key
  acc --- a
  seq --- s
  dc --- d
  au --- audit
  bs --- b & tmp & q
  as & acc & seq & dc & au --- io
```

Schema version 1 equals the reference schema in the shared files, so the
reference image can serve the same directory during the rollback window.

## Identity

SPEC §9. Resolution paths and what the server trusts.

```mermaid
flowchart LR
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef ext fill:#eef3ff,stroke:#4a6cf7,color:#1b2b6b
  classDef store fill:#f4f4f5,stroke:#71717a,color:#27272a

  req["resolve DID / handle"]:::pds --> cache[("did_cache.sqlite<br/>stale TTL · max TTL")]:::store
  cache -- miss or stale --> fetch["safe fetch<br/>(unicast only, redirect cap, size cap)"]:::pds
  fetch --> plc["plc.directory<br/>did:plc"]:::ext
  fetch --> web["https://host/.well-known/did.json<br/>did:web"]:::ext
  handle["verify handle"]:::pds --> txt["DNS TXT _atproto.&lt;handle&gt;<br/>did=…"]:::ext
  handle --> wk["https://&lt;handle&gt;/.well-known/atproto-did"]:::ext
  txt -. "DNS wins on conflict" .- wk
  doc["DID document contract<br/>alsoKnownAs[0] at:// · #atproto Multikey · #atproto_pds service"]:::pds
  plc & web --> doc
```

## OAuth: stock sign-in

SPEC §11. The authorization-code flow with PAR, PKCE and DPoP against the
server's own pages.

```mermaid
sequenceDiagram
  autonumber
  participant App as Client app
  participant AS as pds-oauth (authorization server)
  participant UI as server-rendered pages
  participant Acc as pds-account
  participant RS as resource server (XRPC)

  App->>AS: GET /.well-known/oauth-authorization-server
  App->>AS: POST /oauth/par (client_id metadata URL, PKCE, DPoP)
  AS->>AS: fetch + validate client metadata (safe fetch, cache)
  AS-->>App: request_uri
  App->>UI: GET /oauth/authorize?request_uri
  UI->>Acc: sign-in (password) or remembered device account
  Acc-->>UI: account, consent decision (trusted client may skip)
  UI-->>App: redirect with code (+ iss)
  App->>AS: POST /oauth/token (code, PKCE verifier, DPoP proof)
  AS-->>App: access token (HS256, cnf.jkt), refresh token, scope, sub
  App->>RS: XRPC with DPoP-bound token
  RS->>RS: verify signature, jkt, nonce, scope/permissions
```

## OAuth: LinkJar external sign-in

SPEC §12.3, §13. The same flow with the `linkjar` profile's identity
providers inserted at the sign-in step; the receipt and handle policy hooks
run inside the account transaction.

```mermaid
sequenceDiagram
  autonumber
  participant App as LinkJar app
  participant UI as authorize page
  participant IdP as IdentityProviders (ext)
  participant Apple as Apple / Google / GitHub
  participant Gate as SignupGate (ext)
  participant Acc as account transaction
  participant HP as HandlePolicy (ext)
  participant Rc as CreationReceipt (ext)

  App->>UI: /oauth/authorize (after PAR)
  UI->>IdP: start(provider, device, pending request)
  IdP-->>UI: redirect (state, nonce, PKCE bound to device)
  UI->>Apple: provider authorization
  Apple-->>IdP: callback (code / id_token)
  IdP->>IdP: verify issuer, signature, nonce, audience, verified email
  IdP->>Gate: evaluate(signup request + verified identity)
  Gate-->>IdP: Allow
  IdP->>Acc: begin
  Acc->>HP: suggest handle · check reserved · record rename allowance
  Acc->>Acc: create account, external_identity row, sentinel password
  Acc->>Rc: record receipt (request, client, device, did)
  Acc-->>IdP: commit (all or nothing)
  IdP-->>UI: resume original request with session
  UI-->>App: code → token (receipt carried into the session row)
  App->>App: io.linkjar.account.getSignupReceipt → show handle, then acknowledge
```

## Extension model

SPEC §12. The core calls traits; a profile composes one implementation of
each; selection is `PDS_PROFILE`.

```mermaid
flowchart LR
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef hook fill:#fdeaea,stroke:#d64545,color:#6b1515

  core["server core<br/>(no operator branches)"]:::pds --> P["Profile"]:::hook
  P --> g["SignupGate"]:::hook & i["IdentityProviders"]:::hook & h["HandlePolicy"]:::hook & m["SignInMethods"]:::hook & n["Notices"]:::hook & t["MailTemplates + Branding"]:::hook & r["CreationReceipt"]:::hook & k["AccountHooks + migrations"]:::hook & o["OperatorRoles"]:::hook & s["StorageBackends"]:::hook
  subgraph stock["stock profile (pds-ext)"]
    s1["reference behaviour"]
  end
  subgraph lj["linkjar profile (pds-ext-linkjar)"]
    l1["patch 093 handle policy"]
    l2["patch 099 external providers"]
    l3["patch 099b receipt"]
    l4["patch 099c signup policy"]
    l5["patch 100 sign-in methods + notices"]
    l6["patch 101 mail templates"]
    l7["operator roles"]
  end
  P -. "PDS_PROFILE=stock" .-> stock
  P -. "PDS_PROFILE=linkjar" .-> lj
```

Rules: no request field sets an internal flag; hook writes commit in the
account transaction; hooks fail closed; every hook that changes account
state produces an audit entry.

## Audit log

SPEC §20. Insert-only entries, a hash chain, signed checkpoints, and the
firehose as the witness.

```mermaid
flowchart LR
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef store fill:#f4f4f5,stroke:#71717a,color:#27272a
  classDef ext fill:#eef3ff,stroke:#4a6cf7,color:#1b2b6b

  act["account / operator / MCP action"]:::pds --> e1["entry n<br/>DAG-CBOR, prev hash"]:::store --> e2["entry n+1"]:::store --> e3["…"]:::store
  e3 --> ck["checkpoint every 5 min<br/>Merkle root · key id · Ed25519 signature"]:::pds
  ck --> rec["record io.linkjar.pds.audit.checkpoint<br/>in the operator's own repo"]:::pds
  rec --> fh["firehose"]:::pds --> relays["relays, indexers, mirrors<br/>(witnesses)"]:::ext
  ck --> db[("audit.sqlite")]:::store
  ver["audit verify (standalone binary)<br/>recomputes chain, roots, signatures offline"]:::ext -.-> db
```

## Simulation harness

SPEC §19. One seed drives faults in storage, process and network; oracles
check verify, repair and firehose delivery.

```mermaid
flowchart TB
  classDef tool fill:#f4f4f5,stroke:#71717a,color:#27272a
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00

  seed["seed + commit"]:::tool --> plan["interaction plan<br/>writes, subscribers, imports, lifecycle, OAuth, mail"]:::tool
  plan --> run["pds-sim run (single thread)"]:::tool
  run --> vfs["fault VFS via sqlite-plugin<br/>I/O error after N · fsync failure · torn write · bit flip · disk full · power loss"]:::tool
  run --> fp["fail points<br/>commit · sequence · outbox · PLC · checkpoint"]:::tool
  run --> net["turmoil + mad-turmoil<br/>partitions, delays, crashes, seeded time and randomness"]:::tool
  vfs & fp & net --> sut["linkjar-pds (same code as production)"]:::pds
  sut --> o1["verify passes or repair restores recorded MST root"]:::tool
  sut --> o2["no acknowledged write lost"]:::tool
  sut --> o3["every seq delivered once, in order"]:::tool
  sut --> o4["TRACE logs identical on rerun (determinism meta-test)"]:::tool
```

## Parity harness

SPEC §17. Same scenario, two servers, eight oracles. Fault scenarios run
against the Candidate alone.

```mermaid
flowchart LR
  classDef tool fill:#f4f4f5,stroke:#71717a,color:#27272a
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef ext fill:#eef3ff,stroke:#4a6cf7,color:#1b2b6b

  runner["parity/ (TypeScript, official SDK)"]:::tool
  runner --> ref["Reference image<br/>pinned digest"]:::ext
  runner --> cand["Candidate"]:::pds
  plc["local PLC"]:::ext --- ref & cand
  relay["local Sync 1.1 relay (strict)"]:::ext --- ref & cand
  ref & cand --> o["oracles<br/>O1 responses · O2 CIDs · O3 firehose · O4 OAuth · O5 migration · O6 stock equivalence · O7 LinkJar journeys · O8 Bluesky app · O9 verify at pin"]:::tool
```

## Deployment topologies

SPEC §21. Version 1 is one node. Scale follows the protocol: an entryway
over member hosts. The shared-database cluster is documented, not
recommended.

```mermaid
flowchart TB
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef ext fill:#eef3ff,stroke:#4a6cf7,color:#1b2b6b
  classDef store fill:#f4f4f5,stroke:#71717a,color:#27272a

  subgraph v1["Version 1: single node"]
    n1["linkjar-pds (embedded account + OAuth)"]:::pds --- d1[("data dir")]:::store
    d1 -- "Litestream" --> r2[("object storage")]:::ext
  end
  subgraph v2["Scale: entryway + member hosts"]
    ew["entryway service<br/>accounts · sessions · OAuth · handles · PLC rotation keys · host map"]:::pds --- edb[("entryway db")]:::store
    ew --> m1["member host A<br/>repos · blobs · signing keys · firehose"]:::pds
    ew --> m2["member host B"]:::pds
    ew --> m3["member host C"]:::pds
    relays["relays crawl each member"]:::ext --- m1 & m2 & m3
  end
```

## Sidecars

[sidecars.md](../sidecars.md), SPEC §22. Neither sidecar holds a database
connection or a signing key.

```mermaid
flowchart LR
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef side fill:#eafaf1,stroke:#2e9e5b,color:#0f4d2a
  classDef ext fill:#eef3ff,stroke:#4a6cf7,color:#1b2b6b
  classDef store fill:#f4f4f5,stroke:#71717a,color:#27272a

  subgraph console["ops console (private network)"]
    cui["live views: connections, topology, repos, jobs, audit"]:::side
    cact["actions: takedown, suspend, quarantine, GC, verify, repair, #sync"]:::side
    cotel["OTLP receiver + capped store<br/>traces · metrics · logs"]:::side
  end
  subgraph mcp["MCP server (mcp.&lt;domain&gt;)"]
    rs["MCP over Streamable HTTP<br/>OAuth 2.1 resource server (PRM, CIMD, PKCE)"]:::side
    br["bridge: confidential atproto OAuth client<br/>DPoP key per session, encrypted session store"]:::side
    tools["tools + resources generated from lexicons<br/>public records and account only"]:::side
  end
  op["operator (OAuth session with role)"]:::ext --> cui & cact
  cact -- "internal.* ops API + reason" --> pds["linkjar-pds"]:::pds
  pds -- "OTLP/HTTP" --> cotel
  cotel -- "forward" --> graf["Prometheus · Grafana"]:::ext
  ai["AI client"]:::ext --> rs --> br -- "PAR, consent on PDS page, XRPC" --> pds
  pds --> audit[("audit log: both client ids")]:::store
```

## Cutover and rollback

SPEC §18. Same hostname, same directory, same secrets; the window stays
open until the first Candidate-only migration.

```mermaid
stateDiagram-v2
  [*] --> Reference: production today
  Reference --> Verify: stop container · run verify
  Verify --> Candidate: start linkjar-pds on same dir + secrets
  Candidate --> Smoke: health · describeServer · OAuth metadata · relay resumes cursor
  Smoke --> Watch: web, iOS, extension sign-in · write · firehose · mail
  Watch --> Reference: rollback (start Reference image; Candidate-only files ignored)
  Watch --> Closed: migrate --allow-candidate-only (window closes)
  Closed --> [*]
```

## Repository and build

How the repository is organised and what the gates run.

```mermaid
flowchart LR
  classDef tool fill:#f4f4f5,stroke:#71717a,color:#27272a
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef ext fill:#eef3ff,stroke:#4a6cf7,color:#1b2b6b

  lex["lexicons/<br/>vendored at the pin + io.linkjar.*"]:::tool -- "cargo xtask codegen" --> gen["crates/pds-types/src/generated"]:::pds
  interop["interop/ (CC0 vectors)"]:::tool --> tests["crate tests"]:::pds
  src["crates/"]:::pds --> ci["rust.yml: fmt · clippy · test · codegen --check · deny · vet"]:::tool
  src --> fuzz["fuzz.yml nightly: parsers"]:::tool
  src --> sim["nightly seeds (pds-sim)"]:::tool
  legacy["legacy/ (reference image build)"]:::ext --> build["build.yml (paths: legacy/**)"]:::tool
  devenv["devenv.nix: toolchain, tools, hooks"]:::tool --- src
```
