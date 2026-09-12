Production source patches go here, named `001-description.patch`, in bytewise order.
There are deliberately no production patches until #93/#99/#100 land.

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
