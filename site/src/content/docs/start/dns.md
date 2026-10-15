---
title: Point DNS at the host
description: The two records a PDS needs, the ports to open, and how to check both before you install.
sidebar:
  order: 2
---

Add two records at your DNS provider. Both point at the host.

| Name | Type | Value |
|---|---|---|
| `pds.example.com` | `A` | `203.0.113.10` |
| `*.pds.example.com` | `A` | `203.0.113.10` |

Add `AAAA` records with the same names when the host has a public IPv6
address.

The wildcard matters. An account's handle, `alice.pds.example.com`, is a
host name of its own: apps resolve it to find the account's DID, and Caddy
fetches a certificate for it the first time a client asks. Without the
wildcard, handles do not resolve.

:::note[DNS only, no proxy]
If your DNS provider can proxy traffic through its own edge, leave the
proxy off for both records. Caddy needs to answer the certificate
challenge itself, and the firehose is a WebSocket that stays open for
hours.
:::

## Check the records

Wait until both answers are right. A DNS change takes a few minutes to
spread.

```bash
dig +short pds.example.com
```

```bash
dig +short anything.pds.example.com
```

Each prints `203.0.113.10`.

## Open the ports

Open ports 80 and 443 in the host's firewall and in your provider's
firewall, if it has one. Port 80 serves the certificate challenge and the
redirect to HTTPS; 443 serves everything else. The server itself listens
on port 3000 inside the Compose network only. Nothing else on the host
needs to be reachable.

Next: [install with Docker Compose](/start/install/).
