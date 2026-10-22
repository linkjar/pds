# The self-hosting guide

The pages of [pds.linkjar.io](https://pds.linkjar.io): how to host a
LinkJar PDS, from the DNS records to the audit log. Built with
[Astro Starlight](https://starlight.astro.build).

The pages are Markdown under `src/content/docs/`, one directory per
sidebar group. The architecture diagrams come from
`../docs/architecture/svg` through `src/components/Diagram.astro`, so a
diagram rendered there is current here.

The header logo and favicons are copied unchanged from
[`linkjar/linkjar.io`](https://github.com/linkjar/linkjar.io), under
`apps/web/public/logo/` and `apps/web/public/favicon.ico`. Keep these assets
aligned with the main website instead of drawing a separate PDS mark.

```sh
pnpm install
pnpm dev        # http://localhost:4321
pnpm check      # type-check the site
pnpm test       # HTTP redirect regression tests
pnpm build      # dist/
```

The workflow `.github/workflows/site.yml` builds every pull request that
touches `site/` or `docs/architecture/`. It checks site types, redirect behavior,
and diagram freshness. It builds the site, validates both Cloudflare configurations,
and saves the static output as the `pds-docs` artifact. GitHub Pages is not used.

Cloudflare serves HTTPS pages and assets directly through the `linkjar-pds-docs`
Worker's static asset configuration in `wrangler.jsonc`. HTTPS requests do not run
application code. No database or server-side Astro adapter is required.

A separate `linkjar-pds-docs-https` Worker uses `redirect/wrangler.jsonc` and
`redirect/worker.js`. Its route matches only `http://pds.linkjar.io/*` and returns
`308` to `https://pds.linkjar.io`, preserving the path and query. The handler
rejects requests outside that HTTP origin. Cloudflare supports
[scheme-specific routes](https://developers.cloudflare.com/workers/configuration/routing/routes/#validity)
that run [before a Custom Domain](https://developers.cloudflare.com/workers/configuration/routing/custom-domains/#interaction-with-routes).
The redirect Worker does not proxy requests or fetch assets.

Both Workers disable `workers.dev` and preview URLs. To move the guide, update
the Astro `site` URL, the static Worker's custom domain, and the redirect route
and handler origin together.

For a local release from a reviewed `main` checkout, authenticate with
`pnpm exec wrangler login`, then run:

```sh
pnpm install --frozen-lockfile
pnpm check
pnpm test
pnpm --dir ../docs/architecture install --frozen-lockfile
pnpm --dir ../docs/architecture check
pnpm build
pnpm deploy:dry
pnpm run deploy
```

`deploy:dry` validates both Workers without publishing. `deploy` publishes the
static asset Worker first, then the HTTP redirect Worker. These are separate
deployments; if a step fails, fix the failure and rerun the command. The publishing
credential needs access to both Workers and the `linkjar.io` Worker route.

After release, check that HTTP redirects preserve the path and query:

```sh
curl --head 'http://pds.linkjar.io/start/install/?check=one&check=two'
curl --head https://pds.linkjar.io/start/install/
```

Expect `308` with the same HTTPS path and query, then `200` for the HTTPS page.
Also check the homepage, search, branding assets, and an unknown HTTPS path
(`404`). Local tests and dry runs cannot verify Cloudflare's live route binding.

Automatic publication is not configured. If adding it, treat the two Workers
as separate deployment targets and retain these checks. Keep deployment credentials
outside the repository. GitHub Actions only validates and saves the static build.

`pnpm-workspace.yaml` pins the parser used by `postcss-nested@6.2.0` to the
patched version for [GHSA-rj75-hqrm-r3gf](https://github.com/advisories/GHSA-rj75-hqrm-r3gf).
Remove this scoped override when the upstream dependency uses a patched parser.

What the guide documents is what the server does today. A page that
describes behaviour the server does not have is a bug: fix the page or
the server in the same change.
