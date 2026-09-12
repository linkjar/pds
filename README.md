# LinkJar PDS image

Source build of the reference AT Protocol PDS for LinkJar's hosted accounts.
Tracks [linkjar.io #98](https://github.com/linkjar/linkjar.io/issues/98).
External sign-in and account linking belong to #99/#100; this repository provides
the build, patch and staging-image boundary for that work.

[Local verification record](docs/local-verification.json): all three ARM64 image
profiles built; official-image parity, production startup, and real-PAR Chromium
checks passed. [Original UI](docs/unpatched.png) and [rebuilt branding UI](docs/branding-smoke.png)
are captured from the containers. Remote CI, GHCR publication, staging acceptance,
and filing the upstream proposal are separate outstanding steps.

## Build and verify

Requirements: Docker/BuildKit, Python 3, Node 24 and npm. The atproto build itself
runs with upstream's pinned Node/pnpm inside Docker.

```sh
python3 scripts/check.py
npm ci
export PLAYWRIGHT_BROWSERS_PATH="$PWD/.build/browsers"
npx playwright install chromium
docker build --build-arg PATCH_PROFILE=none -t linkjar-pds:unpatched .
docker build --build-arg PATCH_PROFILE=branding-smoke -t linkjar-pds:branding-smoke .
docker build -t linkjar-pds:production .
python3 scripts/smoke.py --browser
```

`upstream.json` pins a stable PDS tag, its **peeled commit**, local patch revision
and official distribution comparator digest. Fetch verifies both tag and commit;
a moved tag fails. `Dockerfile` is generated from the verbatim upstream service
Dockerfile in `docs/upstream.Dockerfile`, adding only source fetch and patch steps
and redirecting source copies to that stage. It retains upstream's build order,
runtime user, entrypoint, telemetry and environment defaults.
The verbatim upstream Dockerfile and branding patch context retain the pinned
[Bluesky copyright and license notice](docs/upstream-LICENSE.txt), with its
[MIT](docs/upstream-LICENSE-MIT.txt) and [Apache-2.0](docs/upstream-LICENSE-APACHE.txt)
texts alongside it. These notices describe vendored upstream material; they do not
assign a new license to LinkJar's own code.

The source service image and official `bluesky-social/pds` distribution have
different packaging and Node patch versions. The smoke gate compares their PDS
version, discovery/OAuth metadata, invalid authentication and record validation
responses under identical isolated configuration. The official distribution bakes
its independent `0.4.50xx` release label into `PDS_VERSION`; probes set that display
value explicitly to the source version and independently verify the installed
`@atproto/pds` package version in both containers. It also renders the
authorization sign-in page in Chromium after a real PAR request and verifies the branding patch changed the actual
compiled UI. Results and screenshots are in `.build/`. This is bounded local
runtime evidence, not proof of complete equivalence or a staging signup test.
No public identities are created by these probes; PLC points to an unused local
port. Containers use random local ports and are removed with their volumes.

## Patch lifecycle

See [patches/README.md](patches/README.md). `production` is the default profile;
`none` and `branding-smoke` exist only for verification. Production begins with no
patches. The branding example changes a sign-in title, is rebuilt through Lingui
and Vite, and is never published. Reserved handles (#93), provider (#99) and
linking (#100) patches can land independently, each with an upstream/removal plan.

## CI and releases

Pull requests and main pushes run pin checks, unpatched/branding/production builds
and runtime/browser checks. Only verified main builds publish AMD64 and ARM64
images to GHCR, with provenance and SBOMs. Tags normalize the upstream package tag
to Docker syntax: `ghcr.io/linkjar/pds:atproto-pds-0.5.34-1`.
Increment `revision` for each new image from the same upstream release. Existing
release tags are protected against replacement by the publish job; deploy digests.
There is no mutable `latest` tag and no automatic deployment.

First publication uses the same path as later releases: push the verified build
to a unique `build-<run_id>-<run_attempt>` candidate tag, then confirm the release
tag is absent and promote the exact candidate digest. This creates a new GHCR
package with this workflow's credentials before inspecting a versioned tag; GHCR
otherwise returns an ambiguous `DENIED` for a nonexistent package. Authorization,
DNS and registry failures never count as absence. Existing release tags are never
promoted over. Failed promotions can leave a uniquely named candidate for diagnosis.

Every Monday the updater checks stable upstream PDS tags, resolves annotated tags,
copies that release's service Dockerfile, and opens/updates one bump PR. Configure
`UPSTREAM_PR_TOKEN` with a narrowly scoped GitHub App token or fine-grained PAT
allowing contents and pull-request writes for this repository, so its PR triggers
normal CI (GitHub's default workflow token does not). A patch conflict or parity
version mismatch keeps the PR red. Update `officialImage` to the corresponding
official digest after checking its package version; do not weaken the parity gate.
Release commits cannot be silently selected by a lexicographic tag sort.

## Staging target

`staging/compose.yml` consumes an explicit verified image digest and an external
secrets file. It does not create DNS, TLS, servers, secrets, public accounts or an
R2 bucket. Those dependencies belong to [#90](https://github.com/linkjar/linkjar.io/issues/90)
and restore acceptance to #91. Keep staging data and credentials separate from
production. Prepare the mounted directory for upstream's `node` UID 1000, set
`PDS_DATA_DIRECTORY=/app/data`, and configure the #90 proxy/TLS and PDS environment.

```sh
docker compose --env-file staging/image.env -f staging/compose.yml config --quiet
docker compose --env-file staging/image.env -f staging/compose.yml pull
docker compose --env-file staging/image.env -f staging/compose.yml up -d --wait
```

Before accepting accounts, verify real PAR/OAuth signup, compiled UI, TLS, CAPTCHA,
email, R2 round-trip and lost-host restore. Record the digest and evidence in #98.
Rollbacks after schema changes require the version-specific restore plan; changing
an image tag alone is not a database rollback.

## Upstream proposal

[Prepared proposal](docs/upstream-proposal.md), target
[bluesky-social/atproto](https://github.com/bluesky-social/atproto/issues/new).
It is a draft until its actual issue URL is recorded here and in #98.

## Sources

- [Pinned source service Dockerfile](https://github.com/bluesky-social/atproto/blob/7ca16cc6989f8247637615aca17c5abb911b8fb1/services/pds/Dockerfile)
- [Pinned UI asset resolution](https://github.com/bluesky-social/atproto/blob/7ca16cc6989f8247637615aca17c5abb911b8fb1/packages/oauth/oauth-provider/src/router/assets/assets.ts)
- [Official self-hosted distribution](https://github.com/bluesky-social/pds)
- [LinkJar hosting decision](https://github.com/linkjar/linkjar.io/blob/main/docs/research/serverless-pds-2026-09/README.md)
