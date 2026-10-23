---
title: Operations console
description: Run the console beside a native PDS, grant an operator role, and connect privately through an SSH tunnel.
---

The operations console shows accounts, sessions, audit events, and live server
state. Operators can take supported actions with a recorded reason. It calls
the PDS's private operations API; it does not open the server's database.

Use an SSH tunnel to reach it. The console currently requires an HTTP loopback
URL and does not support a public console hostname.

## Build the console

Follow [the sidecar build commands](/reference/binaries/#build-the-sidecars).
Keep the generated `assets/` directory next to `linkjar-pds-console` when you
install it. The PDS Nix package and portable archives do not include the console.

For containers, use the [Compose console guide](/scale/sidecars/#the-operations-console).
The examples below assume native services on one Linux host.

## Connect it to the PDS

Enable the private operations listener in the PDS service environment:

```dotenv title="PDS service environment"
PDS_OPS_ADDRESS=127.0.0.1:2584
```

Configure a separate unprivileged console service:

```dotenv title="Console service environment"
CONSOLE_ADDRESS=127.0.0.1:2585
CONSOLE_PUBLIC_URL=http://127.0.0.1:2585
CONSOLE_PDS_PUBLIC_URL=https://pds.example.com
CONSOLE_PDS_INTERNAL_URL=http://127.0.0.1:2583
CONSOLE_OPS_URL=http://127.0.0.1:2584
```

Run `linkjar-pds-console` without arguments, with these settings loaded by your
service manager. Adapt the [native systemd guide](/start/native/#linux-binary-with-systemd)
for its executable, environment file, and service user. The console does not
need the PDS's secrets or data directory.

Read the client ID from the console's startup log. Add it to `PDS_OPS_CLIENT_IDS`
in the PDS environment, preserving any existing comma-separated entries. For
the exact loopback URL above, the ID is:

```dotenv title="PDS service environment — console client allowlist"
PDS_OPS_CLIENT_IDS=http://localhost?redirect_uri=http%3A%2F%2F127.0.0.1%3A2585%2Foauth%2Fcallback&scope=atproto
```

Restart the PDS after updating its settings. If you change the console's
browser port, use the new client ID from its log.

## Grant an operator role

Create the operator's PDS account first. Only the PDS admin password can grant
a role. Use the private operations listener on the server host, and supply the
account's real DID:

```bash title="Run on the PDS host — curl prompts for the admin password"
curl --fail-with-body --user admin \
  --header 'X-Ops-Reason: first operator' \
  --header 'Content-Type: application/json' \
  --data '{"did":"REPLACE_WITH_ACCOUNT_DID","role":"operator"}' \
  http://127.0.0.1:2584/xrpc/internal.ops.grantRole
```

`operator` permits operations other than granting roles. Use `moderator` for
the more restricted moderation role. Keep the operations port off the public
network.

## Open the console

From your own computer, leave this SSH session open:

```bash title="Forward the console to your computer"
ssh -N -L 2585:127.0.0.1:2585 operator@pds.example.com
```

Open `http://127.0.0.1:2585` in your browser. Sign in on the PDS with the account
that holds the operator role. Do not use the admin password as a console login.

For a local health check, run `linkjar-pds-console health` with the console's
environment. Restarting the console signs operators out because sessions are
stored in memory.

## If sign-in fails

Check that the browser URL matches `CONSOLE_PUBLIC_URL`, the PDS allows the
logged client ID, and the account has a role. Verify that the console service
can reach `CONSOLE_OPS_URL`. When using containers, use the Compose network
addresses instead of these native loopback examples.

Next: [monitor the server](/operate/monitoring/) and [review the audit log](/operate/audit-log/).
