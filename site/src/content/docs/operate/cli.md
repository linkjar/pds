---
title: Command line
description: The commands of the linkjar-pds binary and how to run them in the Compose deployment.
sidebar:
  order: 5
---

The image has one program, `linkjar-pds`. The container runs `serve`; the
other commands run from a shell on the host.

Against the running server, a command that reads only:

```bash
docker compose exec pds linkjar-pds verify
```

With the server stopped, or in a container of its own:

```bash
docker compose run --rm pds linkjar-pds check-config
```

Every command reads its configuration from the environment, which both
forms take from `pds.env`.

## Commands

| Command | Does |
|---|---|
| `serve` | Serves the data directory. The container's command. |
| `check-config` | Validates the configuration and exits. Prints `the configuration is valid`. |
| `verify [--json]` | Checks the data directory: each repository against its records and blobs. Reads only; prints a summary with the count of differences, lists each, and exits 1 when there is one. |
| `export-car <did>` | Writes one repository to standard output as a CAR file. |
| `audit seal` | Begins the audit log with a signed snapshot of the accounts, for a directory with history before the log. Refused on a log with entries. |
| `audit export [<from> [<to>]]` | Writes the audit log, or a range of entries, to standard output as a bundle. |
| `audit verify <file>` | Checks a bundle offline. Needs no configuration, so it runs on any machine with the binary. Exits 1 on a problem. |
| `migrate-storage --to postgres\|sqlite` | Copies the shared databases between the files of the data directory and Postgres, and compares the two. Runs only while no server does. |
| `reconcile [--remove-orphans]` | At an entryway, compares the accounts with the repositories of the member host. |
| `health` | Asks the server on this host for its health route. The container's health check. |
| `version` | Prints the version. |

Standard output belongs to the command's result, so `export-car` and
`audit export` redirect cleanly; the logs go to standard error. Use
`docker compose exec -T` when you redirect, so that Docker allocates no
terminal:

```bash
docker compose exec -T pds linkjar-pds audit export > audit-bundle.json
```
