---
title: Console and MCP server
description: Optional console and MCP sidecars, with Compose deployment examples and native build scope.
sidebar:
  order: 3
---

For a first setup, use the dedicated [MCP guide](/connect/mcp/) or
[operations console guide](/connect/console/). This page keeps the Compose
examples and existing links together. The [binary and port map](/reference/binaries/)
explains the native equivalents.

Both sidecars talk to the server over its APIs. Neither holds a database
connection, a signing key or a secret of the server, so each can be added
and removed without touching the data directory, and every action they
take is in the [audit log](/operate/audit-log/).

The examples on this page use Docker Compose. The native PDS archives and Nix
packages include only `linkjar-pds`, not these sidecars. A native PDS can use
separately built sidecars through the same APIs. Replace Compose DNS names
with reachable host addresses and keep operations listeners private. Native
listeners on the same host should use loopback, not the container-only
`0.0.0.0` examples below.

## The operations console

A page with the live numbers of the server and the firehose subscribers,
the accounts and the sessions of each, the audit log, the blocked email
domains and the operator roles. It takes an account down and lifts the
takedown, ends an account's sessions, disconnects a subscriber, blocks an
email domain, signs a checkpoint, and removes expired sessions. Each form
has a field for the reason, which goes into the audit entry.

An operator signs in with their own account on the server, through OAuth,
and the account needs a role. The console is a loopback client: reach it
through an SSH tunnel, never under a public name.

### Run it

On the server's side, in `pds.env`:

```dotenv
PDS_OPS_ADDRESS=0.0.0.0:2584
```

In `compose.yml`, a service beside `pds`:

```yaml
  console:
    image: ghcr.io/linkjar/pds-console@sha256:…
    restart: unless-stopped
    environment:
      CONSOLE_ADDRESS: 0.0.0.0:2585
      CONSOLE_PUBLIC_URL: http://127.0.0.1:2585
      CONSOLE_PDS_PUBLIC_URL: https://pds.example.com
      CONSOLE_PDS_INTERNAL_URL: http://pds:3000
      CONSOLE_OPS_URL: http://pds:2584
    ports:
      - "127.0.0.1:2585:2585"
```

Start it and read its client id from its first log lines:

```bash
docker compose up -d console && docker compose logs console
```

Put the id into `pds.env` as `PDS_OPS_CLIENT_IDS` and restart the server.
The ops listener takes a token only from a client on that list.

### Grant a role

Roles are `operator`, which takes every action but a role grant, and
`moderator`, which reads, changes the status of a subject, ends sessions
and disables invites. Only the admin password grants a role. Publish the
ops listener on the host's loopback, beside the console's port:

```yaml
    ports:
      - "127.0.0.1:2584:2584"
```

Then, with `PDS_ADMIN_PASSWORD` in the environment and a reason, which
every action on the ops listener requires:

```bash
curl -s -u "admin:$PDS_ADMIN_PASSWORD" -H 'X-Ops-Reason: first operator' -H 'content-type: application/json' -d '{"did": "did:plc:…", "role": "operator"}' http://127.0.0.1:2584/xrpc/internal.ops.grantRole
```

### Open it

From your workstation, a tunnel to the host:

```bash
ssh -L 2585:127.0.0.1:2585 operator@pds.example.com
```

Open `http://127.0.0.1:2585`, and sign in with the account that holds the
role. A restart of the console signs every operator out: it keeps the
sessions in memory.

## The MCP server

An MCP server that AI clients connect to with OAuth, in order to read and
write an account's records with the account's consent. It is a public
service under a name of its own, such as `mcp.example.com`, behind Caddy.
Toward the PDS it is an OAuth client with a key of its own; its own tokens
never reach the PDS, and each write names the AI client in the audit entry.

Its configuration is `MCP_ADDRESS`, `MCP_PUBLIC_URL`,
`MCP_PDS_PUBLIC_URL`, `MCP_PDS_INTERNAL_URL`,
`MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX` and `MCP_WRITE_COLLECTIONS`, the
collections an AI client may ask to write. Its client id is
`MCP_PUBLIC_URL` followed by `/oauth-client-metadata.json`, and it logs
it at start. On the server's side, `PDS_MCP_CLIENT_IDS` names it, and `PDS_OAUTH_KNOWN_CLIENTS_FILE` can hold
its client metadata so that the server fetches nothing. Sessions live in
the process's memory, so one instance runs and a restart ends every
session.

The design and the state of both sidecars are in
[docs/sidecars.md](https://github.com/linkjar/pds/blob/main/docs/sidecars.md)
in the repository.
