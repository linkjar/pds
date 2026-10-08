Production source patches go here, named `001-description.patch`, in bytewise order.
Production currently includes `093-handle-policy.patch` for #93; provider patch
`099-...` applies after it. See [the policy, tests and removal condition](../docs/handle-policy.md).

Generate patches against the exact `upstream.json` commit with `git diff --binary`.
Use `a/` and `b/` paths rooted in the atproto checkout. Include a rationale,
upstream issue, owning LinkJar issue, tests and removal condition with each patch.
The build runs `git apply --check` before each patch; conflicts fail CI.

The two OAuth packages are workspace source dependencies in this build. A pnpm
registry-package patch would not patch those workspace sources. These unified
patches apply before the pinned upstream pnpm install/build; any registry package
patch can instead change upstream's `pnpm.patchedDependencies` and lockfile in the
same source patch. Handle reserved-list additions from #93 use this same directory.

`examples/branding-smoke` is an isolated, reversible UI example. It is only applied
with `PATCH_PROFILE=branding-smoke` and is never included in published production
images. UI compilation and asset manifests are produced by upstream's full build.

`099-external-providers.patch` follows `093-handle-policy.patch`; see
[external provider configuration, invariants, tests, and removal plan](../docs/external-providers.md).
The image build runs the patched provider tests before pruning development
dependencies. Runtime tests exercise the compiled SQLite and UI/API boundary.

`099b-signup-receipt.patch` follows provider signup and binds the newly created
DID to its OAuth session. See [the receipt contract and lifecycle](../docs/signup-receipt.md).

`099c-signup-policy.patch` prevents fresh legacy signup from bypassing the
configured OAuth hCaptcha flow and rejects partial hCaptcha configuration.
See [#94's policy, compatibility tests and removal condition](../docs/provider-policy.md).

`100-signin-methods.patch` adds explicit device-bound provider linking,
transactional last-method protection, account-page controls and persistent
security notifications. See [scope, migration, tests and removal](../docs/signin-methods.md).

`101-mail-templates.patch` gives all six account emails consistent HTML layouts and
keeps a text alternative. See [scope, tests, runtime overlay and removal](../docs/mail-templates.md).

`102-signup-journey.patch` opens the OAuth sign-up page on the email form, puts
Apple and Google under it, and gives the authorization screens LinkJar's onboarding
look (dark stage, Geist, pill actions). The app hands new members to this page,
which looked like a separate product. It also adds `font-src 'self'` to the page
CSP so the bundled font loads. Upstream issue: none filed. Owner: the
`linkjar/linkjar.io` onboarding build (October 2026). Tests: the compiled
`tests/external-ui.mjs` and `tests/provider-branding.mjs` checks cover the order,
font and pill in light and dark. Remove it when upstream's OAuth UI supports a
configurable layout and typeface, or when LinkJar serves its own sign-up pages.
See [scope, tests, runtime and removal](../docs/signup-journey.md).
