# The self-hosting guide

The pages of [pds.linkjar.io](https://pds.linkjar.io): how to host a
LinkJar PDS, from the DNS records to the audit log. Built with
[Astro Starlight](https://starlight.astro.build) and
[Starlight Black](https://starlight-theme-black.vercel.app/).

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
pnpm test       # clipboard and HTTP redirect regression tests
pnpm build      # dist/
pnpm test:built # generated pages and Markdown export checks
```

## Link previews

`public/linkjar-pds-og-v1.jpg` is the shared 1200 × 630 social card. It was
generated with the built-in image tool using the real LinkJar jar as a reference,
then exported as an optimized JPEG. The generation prompt is saved in
`src/assets/linkjar-pds-og-prompt.md`. Use a new versioned filename when replacing
the image so sharing services can fetch a fresh asset.

Starlight supplies each page's canonical URL, title, and description. The global
head config adds the image, its dimensions and alt text, and the Open Graph locale.
The homepage declares itself a website; documentation pages remain articles.
`src/route-data.ts` mirrors the final Open Graph values into explicit Twitter card
tags, preserving page overrides. All tags are in the built HTML, so crawlers do
not need JavaScript. `pnpm test:built` checks the tags and the image itself.

The workflow `.github/workflows/site.yml` builds every pull request that
touches `site/` or `docs/architecture/`. It checks site types, redirect behavior,
and diagram freshness. It builds the site, checks its generated pages and Markdown,
validates both Cloudflare configurations,
and saves the static output as the `pds-docs` artifact. GitHub Pages is disabled;
there is no Pages deployment job, Pages environment, or Pages publishing permission.
The live site is served only by the Cloudflare Workers described below.

The separate `.github/workflows/nix.yml` workflow builds the native Rust server,
not this Astro site. Its `linkjar-pds-x86_64-linux` and
`linkjar-pds-aarch64-linux` artifacts contain portable executable archives and
checksums. It does not deploy a NixOS host. See the public
[build guide](https://pds.linkjar.io/start/builds/) and
[native installation guide](https://pds.linkjar.io/start/native/).

## Code examples

Expressive Code highlights examples with GitHub's dark and light palettes while
keeping the site's neutral styling. Use `bash` for shell commands, `dotenv` for
environment files, `ini` for systemd units, and `nix` for Nix expressions. Use
`text` for output or prose that does not need highlighting. Add `title="filename"`
to a fence when the reader must save the example in a particular file.

`src/integrations/caddyfile.mjs` supplies the Caddyfile highlighting used by the
native and MCP guides. It covers the syntax in those examples; it does not
validate proxy configuration. Code-copy controls preserve the original text.
Built-site tests check token colours in both themes and the copy payload.

## Markdown for AI assistants

[starlight-llm-actions](https://holdenhewett.github.io/starlight-llm-actions/)
generates `/<page>.md.txt`, `/llms.txt`, and `/llms-full.txt` during the build.
The homepage's Markdown is `/index.md.txt`. The `.txt` extension lets static hosts
serve the pages as readable text. `llms.txt` links to individual pages, the
complete documentation, and three focused bundles: `/llms-setup.txt`,
`/llms-operations.txt`, and `/llms-clients.txt`. Their membership and reading order
are configured in `src/integrations/llm-docs.mjs`. Every bundle uses the same
page renderer. Update the subset paths when adding a new documentation section.

Every HTML page links to the index with `rel="describedby"`. Each guide also
exposes its Markdown alternative with `rel="alternate"`. The **Use with AI** menu
provides visible links, copy controls, and a guide to choosing the right context.

The renderer resolves MDX components before converting their HTML to Markdown.
It keeps installation tab labels, code blocks, and diagram descriptions. A small
adapter adds each page's canonical source URL and makes link and image destinations
absolute, including their anchors. Draft pages are excluded. Use
`data-mdast="ignore"` for controls or duplicated visual elements that should not
appear in the exported text; do not use it to hide confidential material.

Black's built-in action copies raw MDX, so it is disabled. `DocsPageTitle.astro`
keeps Black's title layout and adds `DocsCopy.astro` plus a Markdown link. The same
copy component serves the homepage's complete-documentation action. It fetches
only public text from this origin when clicked, omits cookies, rejects redirects,
and stops a stalled download after ten seconds. Clipboard or network failures
show a message directing the reader to the Markdown link. No AI provider receives
content automatically, and no API key or AI service is required.

Run `pnpm test:built` after building. These tests inspect the published files,
including MDX conversion, canonical links, bundle coverage, discovery links,
diagram descriptions, and the page controls. The upstream renderer can warn and
fall back to raw source if a component
fails to render; review build warnings and keep these output checks passing when
adding components. Preview the site in a browser to check clipboard permission
handling and mobile layout. Static output tests do not exercise browser permissions.

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
pnpm test:built
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
