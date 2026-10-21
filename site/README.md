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
pnpm build      # dist/
```

The workflow `.github/workflows/site.yml` builds every pull request that
touches `site/` or `docs/architecture/`. It checks site types and diagram freshness
before building, validates the Cloudflare configuration, and saves the static output
as the `pds-docs` artifact. GitHub Pages is not used.

Cloudflare Workers serves the static files using `wrangler.jsonc`. No Worker
runtime code, database, or server-side adapter is required. The custom domain is
`pds.linkjar.io`; `workers.dev` and preview URLs are disabled. To move the guide,
update both the Astro `site` URL and the custom domain in `wrangler.jsonc`.

For a local release from a reviewed `main` checkout, authenticate with
`pnpm exec wrangler login`, then run:

```sh
pnpm install --frozen-lockfile
pnpm check
pnpm --dir ../docs/architecture install --frozen-lockfile
pnpm --dir ../docs/architecture check
pnpm build
pnpm deploy:dry
pnpm run deploy
```

Automatic releases are optional. To enable them, connect the `linkjar-pds-docs` Worker to `linkjar/pds`
in Cloudflare Workers Builds. Use `main` as the production branch, `site` as the
root directory, and disable builds for other branches. Use this build command:

```sh
pnpm install --frozen-lockfile && pnpm check && pnpm --dir ../docs/architecture install --frozen-lockfile && pnpm --dir ../docs/architecture check && pnpm build && pnpm deploy:dry
```

Set the deploy command to `pnpm run deploy` and the watch paths to `site/**` and
`docs/architecture/**`. Cloudflare manages the build's deployment credentials;
no personal OAuth token belongs in the repository or GitHub secrets. Both the
Cloudflare build and GitHub Actions run the site checks before publication.

`pnpm-workspace.yaml` pins the parser used by `postcss-nested@6.2.0` to the
patched version for [GHSA-rj75-hqrm-r3gf](https://github.com/advisories/GHSA-rj75-hqrm-r3gf).
Remove this scoped override when the upstream dependency uses a patched parser.

What the guide documents is what the server does today. A page that
describes behaviour the server does not have is a bug: fix the page or
the server in the same change.
