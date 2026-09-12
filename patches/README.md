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
