---
title: Logs, health and metrics
description: What the server logs, the health route, the metrics listener, and what to alert on.
sidebar:
  order: 3
---

## Logs

The server logs JSON lines to standard error, one object per line with
the time, the level, the message and the fields of the event. Requests
carry a request id. The logs never contain a token, a password, an email
address or the body of a record.

For NixOS and the native systemd service:

```bash
sudo journalctl -u linkjar-pds -f -o cat
```

For Compose:

```bash
docker compose logs -f pds
```

A foreground binary writes the same JSON to its standard error. Capture it
through your service manager rather than discarding it.

`LOG_LEVEL` sets the filter: `info` by default, `debug` for everything,
or a filter by crate such as `info,pds_oauth=debug`. It takes effect at
the next start.

## Health

```bash
curl -s https://pds.example.com/xrpc/_health
```

Status 200 when the account database answers; 503 with
`"error":"Service Unavailable"` when it does not. The container's health
check runs `linkjar-pds health` every ten seconds against the same route,
and Caddy waits for the first pass before it starts.

```bash
docker compose ps
```

The `pds` service shows `(healthy)`. For native services:

```bash
sudo systemctl status linkjar-pds
curl --fail http://127.0.0.1:2583/xrpc/_health
```

A running process alone does not establish health. Monitor the HTTP response;
there is no container health scheduler in a native installation.

## Metrics

`PDS_METRICS_ADDRESS` starts a private listener in Prometheus text
format. Keep it private and outside the public proxy routes.

For a native service, bind directly to loopback in its runtime environment:

```ini
PDS_METRICS_ADDRESS=127.0.0.1:9464
```

For Compose only, bind inside the container and publish on host loopback.
In `pds.env`:

```ini
PDS_METRICS_ADDRESS=0.0.0.0:9464
```

In `compose.yml`, under the `pds` service:

```yaml
    ports:
      - "127.0.0.1:9464:9464"
```

Then:

```bash
curl -s http://127.0.0.1:9464/metrics
```

| Metric | What it is |
|---|---|
| `pds_build_info` | the version, as a label |
| `pds_http_requests_total{route,status}` | requests by method and status |
| `pds_http_request_duration_seconds{route}` | a histogram of latency by method |
| `pds_firehose_subscribers` | open firehose connections |
| `pds_audit_entries` | entries in the audit log |
| `pds_audit_unsigned_entries` | entries not yet under a signed checkpoint |
| `pds_audit_checkpoint_age_seconds` | time since the last checkpoint |
| `pds_notice_outbox_depth` | mail waiting to go out, in the `linkjar` profile |

A `route` label is a method the server handles, a path it knows,
`proxied` or `other`, so a client cannot make up a label.

## What to alert on

| Signal | Means |
|---|---|
| `pds_http_requests_total{status=~"5.."}` rising | the server fails requests; read the logs |
| the 99th percentile of `pds_http_request_duration_seconds` | the host is short of CPU or disk, or a client is heavy |
| `pds_audit_unsigned_entries` above 0 for more than ten minutes | checkpoints stopped; the key is missing or the clock is off |
| `pds_notice_outbox_depth` growing | SMTP fails |
| `pds_firehose_subscribers` at 0 for long | the relay is not connected; see [the first account](/start/first-account/#when-posts-reach-other-people) |

## Rate limits

The reference server's limits run by default: by client address for
requests overall and for sign-ups and sign-ins, and by account for writes.
Every answer carries `RateLimit-Limit`, `RateLimit-Remaining` and
`RateLimit-Reset`; a refused request is a 429 with `Retry-After`. The
server uses the connection address and forwarding from private proxy hops.
The supplied Caddy setups forward the client address. Keep the upstream HTTP
listener private so public clients cannot bypass the proxy.
