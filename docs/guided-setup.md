# Guided setup proposal

Status: design proposal, 2026-10-23. No installer command is shipped by this change.

## Recommendation

Build a separate Rust executable, provisionally `linkjar-pds-setup`. Start with
new native Linux installations on AMD64 and ARM64, using systemd, local SQLite,
disk blobs, and a local Caddy proxy. Keep the current manual guides complete.
Add NixOS configuration generation and Docker only after the native path has
end-to-end tests. Existing installations should receive a read-only assessment;
migration and upgrades require a separate design.

The installer should have one typed planning and execution engine. Offer an
interactive terminal wizard and a machine-readable interface to that engine.
An optional assistant explains choices and failures through the latter. An LLM
must not be required to install or operate the server.

## Existing pieces to reuse

- `linkjar-pds check-config` validates configuration and serving secrets. It
  does not test DNS, networking, mail delivery, or recovery.
- `linkjar-pds health` checks the running service. Public metadata and HTTPS
  need separate probes from outside the host.
- The NixOS module and native systemd documentation define the service user,
  state directory, environment file, and restrictions.
- Native CI builds normal and static Linux packages and verifies copied
  executables. CI artifacts expire after 14 days; they are not a stable release
  distribution channel for an unattended installer.
- The existing MCP binary exposes account repository tools. It has no host
  provisioning capability and should not gain one.

## User journey

1. **Inspect.** Detect OS, architecture, systemd, available storage, existing
   services, configuration, and data. Report unsupported hosts without changes.
2. **Choose.** Collect a domain, install path, storage choice, email settings,
   and optional sidecars. Explain decisions with documented defaults.
3. **Check.** Inspect DNS A and AAAA records, conflicts on needed ports,
   file ownership, TLS prerequisites, outbound HTTPS, and disk space. Label
   checks as passed, failed, or not verified; local reachability is not proof
   of public reachability.
4. **Plan.** Show the exact package revision, files, service changes, required
   privileges, network exposure, and remaining manual steps. Secrets appear as
   references. Generate the same plan for manual and agent-assisted use.
5. **Apply.** Recheck preconditions and apply only the approved plan. Create
   protected secret files locally, install a verified artifact, validate
   configuration, then enable services. Stop at an unexpected state.
6. **Verify.** Check local health, public HTTPS and server metadata, and sidecar
   discovery where selected. Ask the operator to verify external reachability,
   email receipt, and a first backup/restore. Produce a redacted result report.

## Engine and interfaces

Model the workflow as `inspect -> plan -> apply -> verify`, not an arbitrary
command runner. Proposed subcommands could be `doctor`, `plan`, `apply`, and
`wizard`; settle their syntax during implementation. None are current PDS CLI
commands, and public installation guides must not present them as available.

Each step needs preconditions, bounded actions, observable postconditions, and
recovery behavior. Persist a journal without secrets. A second run should
resume or report the completed state. Never replace a key, configuration file,
or populated data directory because a previous run was interrupted.

`plan` should be unprivileged and produce a versioned JSON document plus a
human-readable diff. Bind approval to its content hash and observed host state.
`apply` must reject a changed plan or stale preconditions. Use argument arrays
and typed file templates, not interpolated shell scripts. Privileged actions
belong in a narrow executor after review; the interactive and model-facing
processes should not run as root.

## Optional agent assistance

Support a user-selected assistant through structured JSON first. A later,
local-only stdio MCP adapter could expose inspection, plan preparation, and
redacted diagnostics. Avoid embedding provider-specific chat SDKs in the
initial installer. A model API key stays with the user's chosen client.

The assistant may suggest a plan or explain a failed check. Approval to apply
comes from the local operator, not model text or a tool response. Treat tool
outputs and retrieved documents as data; they cannot approve new actions.
Do not offer a general shell tool through the installer interface.

Keep admin credentials, signing keys, OAuth tokens, and SMTP passwords out of
prompts, logs, reports, and plan JSON. Generate secrets with the OS random source
and the existing cryptographic types. Use restrictive permissions, no-follow
file creation, atomic writes, and explicit ownership. Do not rotate existing
keys. Show only redacted diagnostic fields to the assistant.

## Packaging and optional components

Before automatic downloads, establish durable release assets with pinned
versions and verifiable provenance. A checksum supplied alongside an archive
does not independently authenticate its publisher. As a first implementation,
the operator can provide a reviewed local binary and expected digest.

The PDS, console, and MCP binaries need separate installation plans. The console
also needs its asset bundle. MCP needs a separate P-256 client key, TLS origin,
and a PDS client allowlist entry. The console needs a private operations listener,
loopback client ID, and an operator account. Neither should open the operations
API publicly. User sign-in and role grants remain explicit steps.

DNS-provider credentials and automatic firewall edits should stay outside the
first version. Give exact instructions and check the result. Do not claim to
configure every hosting environment correctly from local observations alone.

## Implementation and acceptance tests

1. Implement read-only inspection and a versioned plan format in a separate
   crate. Test unsupported OS/CPU, occupied ports, malformed DNS results,
   existing directories, unknown configuration, and permission errors.
2. Add configuration rendering, local-binary verification, and the interactive
   wizard. Exercise a fresh install in disposable Linux VMs on both architectures.
3. Add the reviewed executor. Test interrupted writes, resume, repeated runs,
   stale plans, symlinks, changed ownership, checksum mismatch, offline hosts,
   invalid TLS, and failures before and after service startup.
4. Verify redaction with generated secret values and test that no diagnostic,
   journal, error, or JSON output contains them. Assert existing data and keys
   remain byte-for-byte unchanged when setup refuses a host.
5. Add optional agent integration over the same engine. Confirm it cannot
   approve its own changes or expand the execution plan.

Keep tests in separate test modules or integration-test files. Do not validate
an installer against the live PDS or claim successful backups without a restore.

## Open product decisions

The first implementation needs a choice of durable release distribution and
the exact supported Linux versions. Decide whether native sidecar packaging
ships with the installer or follows it. Keep manual setup available throughout.
