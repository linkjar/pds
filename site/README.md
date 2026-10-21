# The self-hosting guide

The pages of [pds.linkjar.io](https://pds.linkjar.io): how to host a
LinkJar PDS, from the DNS records to the audit log. Built with
[Astro Starlight](https://starlight.astro.build).

The pages are Markdown under `src/content/docs/`, one directory per
sidebar group. The architecture diagrams come from
`../docs/architecture/svg` through `src/components/Diagram.astro`, so a
diagram rendered there is current here.

```sh
pnpm install
pnpm dev        # http://localhost:4321
pnpm check      # type-check the site
pnpm build      # dist/
```

The workflow `.github/workflows/site.yml` builds every pull request that
touches `site/` or `docs/architecture/`. It checks site types and diagram freshness
before building. A push to `main` publishes the result to GitHub Pages. The site
address is `site` in `astro.config.mjs`; change it there and in the
repository's Pages settings to move the guide.

`pnpm-workspace.yaml` pins the parser used by `postcss-nested@6.2.0` to the
patched version for [GHSA-rj75-hqrm-r3gf](https://github.com/advisories/GHSA-rj75-hqrm-r3gf).
Remove this scoped override when the upstream dependency uses a patched parser.

What the guide documents is what the server does today. A page that
describes behaviour the server does not have is a bug: fix the page or
the server in the same change.
