---
title: Command line
description: Run operator commands with a native binary, the NixOS service, or Docker Compose.
sidebar:
  order: 5
---

Every installation uses the same `linkjar-pds` executable and commands.
Except for `version`, `help`, and `audit verify`, commands read configuration
from the environment. The binary does not automatically load `pds.env`.
Use the same data paths, secrets, and service user as the server.

## Native services

The [native installation](/start/native/) uses the `linkjar-pds` user,
`/var/lib/linkjar-pds`, port `2583`, and `/etc/linkjar-pds/pds.env`.
Systemd can load that root-owned file before switching to the service user.
Define this shell helper on the host:

```bash
PDS_BIN=/usr/local/bin/linkjar-pds
pds() {
  sudo systemd-run --quiet --wait --pipe --collect \
    --property=Type=exec \
    --property=User=linkjar-pds \
    --property=Group=linkjar-pds \
    --property=WorkingDirectory=/var/lib/linkjar-pds \
    --property=EnvironmentFile=/etc/linkjar-pds/pds.env \
    --property=UMask=0077 \
    --setenv=PDS_DATA_DIRECTORY=/var/lib/linkjar-pds \
    --setenv=PDS_PORT=2583 \
    "$PDS_BIN" "$@"
}
```

For the NixOS example, set `PDS_BIN=/run/current-system/sw/bin/linkjar-pds`
instead. It installs the same package used by the service on that path.
If you changed the runtime file, data path, or port, change the helper too.
If public settings live in `services.linkjar-pds.environment`, also pass them
with `--setenv=NAME=value`. Use an explicit `SSL_CERT_FILE` when the host's CA
bundle is in a nonstandard location.

This helper uses a temporary service for each command. It waits for completion,
returns the command's exit status, and passes its output to your terminal.
It does not start or stop the main PDS service. See the
[systemd-run manual](https://www.freedesktop.org/software/systemd/man/latest/systemd-run.html)
for these options.

```bash
pds check-config
pds verify --json
```

Do not run a second `serve` through this helper. Stop the main service before
`audit seal` or `migrate-storage`. Run commands that write state as the service
user so they do not leave root-owned database files.

For a foreground installation, use `linkjar-pds <command>` directly from the
shell that holds its configuration and has access to its data.

## Docker Compose

Compose supplies the environment from `pds.env`. Against the running container:

```bash
docker compose exec pds linkjar-pds verify
```

With the main server stopped, run a command in a separate container:

```bash
docker compose run --rm pds linkjar-pds check-config
```

The configuration check can also run while the server is up. Storage migration
and the initial audit seal require the main server to be stopped.

## Commands

| Command | Does |
|---|---|
| `serve` | Serves the data directory. Run under your service manager. |
| `check-config` | Validates the configuration and exits. Prints `the configuration is valid`. |
| `verify [--json]` | Checks the data directory: each repository against its records and blobs. Reads only; prints a summary with the count of differences, lists each, and exits 1 when there is one. |
| `export-car <did>` | Writes one repository to standard output as a CAR file. |
| `audit seal` | Begins the audit log with a signed snapshot of the accounts, for a directory with history before the log. Refused on a log with entries. |
| `audit export [<from> [<to>]]` | Writes the audit log, or a range of entries, to standard output as a bundle. |
| `audit verify <file>` | Checks a bundle offline. Needs no configuration, so it runs on any machine with the binary. Exits 1 on a problem. |
| `migrate-storage --to postgres\|sqlite` | Copies the shared databases between the files of the data directory and Postgres, and compares the two. Runs only while no server does. |
| `reconcile [--remove-orphans]` | At an entryway, compares the accounts with the repositories of the member host. |
| `health` | Asks the server on this host for its health route. Also used by the container health check. |
| `version` | Prints the version. |

Standard output belongs to the result; logs go to standard error.
For native services, redirect the helper's output directly:

```bash
umask 077
pds audit export > audit-bundle.json
pds export-car did:plc:REPLACE_WITH_ACCOUNT_DID > account.car
```

For Docker, use `exec -T` so it allocates no terminal:

```bash
umask 077
docker compose exec -T pds linkjar-pds audit export > audit-bundle.json
```
