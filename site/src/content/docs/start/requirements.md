---
title: What you need
description: Prepare a Linux host, choose Docker or a native installation, and configure the domain and network access for LinkJar PDS.
sidebar:
  order: 1
---

A PDS holds the accounts of a few people and talks to the rest of the
network for them. One small host is enough.

## A host

| | |
|---|---|
| Operating system | Linux, on `amd64` or `arm64` |
| Software | Docker Engine with Compose, or [Nix / a native executable](/start/native/) |
| Memory | 1 GiB is plenty. The server starts in a few MiB; Caddy takes a few tens |
| Disk | 20 GB to start. Records live in SQLite files, images and videos as files beside them |
| Network | A public IP address, with ports 80 and 443 open to the internet |

If you choose the Compose deployment, check Docker on the host:

```bash
docker compose version
```

It prints `Docker Compose version v2…`. The older `docker-compose` with a
hyphen, version 1, does not read the Compose file of this guide.

## A domain name

The server needs a name of its own, such as `pds.example.com`, at a DNS
provider where you can add a wildcard record. Each account gets a handle
under that name, `alice.pds.example.com`, and each handle is a name the
server answers for. [Point DNS at the host](/start/dns/) has the records.

Keep the PDS on a domain of its own, not on a subdomain of a site that
serves a web app. The server's authorization pages and the blobs it serves
should not share a parent domain with an app's cookies.

## HTTPS and CA certificates

The Compose deployment includes Caddy. A native installation needs its own TLS
proxy; the [native guide](/start/native/#public-https) includes a Caddy example.
Caddy uses an operator email for certificate notices. Native hosts also need
a CA certificate bundle for the PDS's outbound HTTPS requests.

## Optional: an SMTP account

The server sends mail to confirm an address and to reset a password.
Without SMTP it sends none, which is fine for a server of one.
[Email](/configure/email/) has the setup.

## Placeholders in this guide

| Placeholder | Replace with |
|---|---|
| `pds.example.com` | the name of your server |
| `203.0.113.10` | the public IP address of your host |
| `operator@example.com` | your email address |
| `alice` | the first account's handle |

Run commands on the host unless a page says otherwise. Docker commands run
from the Compose directory. Native service commands use the service user
and runtime configuration, as explained in [the CLI guide](/operate/cli/).
