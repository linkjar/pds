# Test the Candidate on a Host of Its Own

[Plan](../plan.md) · [Cutover runbook](cutover.md) · [SPEC §4.1](../SPEC.md#41-listeners-and-non-xrpc-routes)

This runbook puts `linkjar-pds` on a public test host with a name of its
own, so that a person can use it from the Bluesky app, from `linkjar.io`
and from any atproto client. It serves these steps of the plan:

- The Bluesky app signs in through a Candidate and posts.
- The browser gate and kit self-test of `linkjar.io` run against a
  Candidate (`E2E_PDS_URL`).

It is not the production deployment, which `linkjar/infra` owns, and it
does not touch production data. To check a copy of production data, follow
§1 of the [cutover runbook](cutover.md) instead.

The files are in [`deploy/test-host/`](../../deploy/test-host/): a compose
file with the server and Caddy, a Caddyfile, and two example environment
files.

## 1. What the host needs

- Linux with Docker Engine and Compose, on `amd64` or `arm64`.
- A name for the server, such as `pds.example.com`. DNS must point the name
  and the wildcard `*.pds.example.com` at the host: a handle such as
  `alice.pds.example.com` is a name of its own.
- Ports 80 and 443 open to the internet. Caddy obtains the certificates
  from Let's Encrypt over them.
- A login to `ghcr.io` that can read `linkjar/pds`, while the images are
  private.

A name under a domain that production uses, such as `linkjar.social`, is
not a test host: pick a name that no production record points at.

## 2. Set it up

1. Copy the directory to the host.

   ```bash
   scp -r deploy/test-host operator@test-host:linkjar-pds-test
   ```

2. Fill in `.env` from `example.env`. Take the image digest from the
   summary of the newest "Server images" run on `main`.

   ```bash
   gh run list --workflow "Server images" --branch main --limit 1
   ```

   ```bash
   cp example.env .env
   ```

3. Fill in `pds.env` from `example.pds.env`. Generate the three secrets
   with the commands above each one, and keep them: the accounts depend on
   them.

   ```bash
   cp example.pds.env pds.env
   ```

   ```bash
   chmod 600 pds.env
   ```

4. Create the data directory for the server's user, uid 1000.

   ```bash
   sudo install -d -o 1000 -g 1000 data
   ```

5. Check the configuration without starting the server.

   ```bash
   docker compose run --rm pds linkjar-pds check-config
   ```

   It prints `the configuration is valid`.

6. Start it.

   ```bash
   docker compose up -d
   ```

   The server logs JSON lines; `serving` means it is up.

   ```bash
   docker compose logs pds
   ```

## 3. Check it from outside

Run the smoke test from a workstation with the harness installed
([parity/README.md](../../parity/README.md#run-it)). It reads only: health,
discovery, the OAuth metadata, a pushed authorization request and its
sign-in page, and the firehose.

```bash
cd parity && pnpm parity smoke https://pds.example.com
```

Every line must start with `ok`. Then let it create an account, use it the
way the Bluesky app does, and delete it. The account's DID stays in
`plc.directory` after the deletion.

```bash
read -s PDS_ADMIN_PASSWORD && export PDS_ADMIN_PASSWORD
```

```bash
pnpm parity smoke https://pds.example.com --write
```

`--keep-account` keeps the account and prints its handle and password, to
sign in to it from an app.

## 4. Use it from an app

1. Create an invite code. The answer holds the code.

   ```bash
   curl -s -u "admin:$PDS_ADMIN_PASSWORD" -H 'content-type: application/json' -d '{"useCount": 1}' https://pds.example.com/xrpc/com.atproto.server.createInviteCode
   ```

2. In the Bluesky app, choose **Create account**, then the hosting provider
   **Custom**, and enter `https://pds.example.com`. Use the invite code.
3. Post, follow, upload an image, and sign out and in again. A post appears
   in other people's timelines once the relay at `bsky.network` crawls the
   host. The server asks it to with the first write after a start, and
   then at most once in twenty minutes.
4. Sign in to `linkjar.io` with the account, or point its browser gate at
   the host with `E2E_PDS_URL=https://pds.example.com`. The gate needs
   `PDS_PROFILE=linkjar` and the external sign-in providers in `pds.env`.

Record each result against its step in the [plan](../plan.md#owners-steps).

## 5. Update and remove

To move to a newer image, change `PDS_IMAGE` in `.env` and start again.

```bash
docker compose up -d
```

To remove the host, delete each test account first, so that no repository
stays reachable.

```bash
curl -s -u "admin:$PDS_ADMIN_PASSWORD" -H 'content-type: application/json' -d '{"did": "did:plc:..."}' https://pds.example.com/xrpc/com.atproto.admin.deleteAccount
```

```bash
docker compose down --volumes
```

The data directory stays on the host until you delete it.

## 6. How the certificates work

Caddy asks for a certificate the first time a client connects to a name.
Before it asks, it calls the server's `GET /tls-check?domain=<name>`, which
passes the server's name and the handle of each active account and fails
for every other name ([SPEC §4.1](../SPEC.md#41-listeners-and-non-xrpc-routes)).
A name that nobody hosts therefore gets no certificate, and a stranger who
points a name at the host cannot use up the rate limits of Let's Encrypt.
