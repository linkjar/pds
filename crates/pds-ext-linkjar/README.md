# pds-ext-linkjar

The `linkjar` profile: `Linkjar` implements `Profile` with LinkJar's
account behaviour, which the patched Reference build carried as source
patches. It covers handles under `linkjar.social` (generation,
normalisation, one free rename within thirty days), the reserved names and
their look-alikes, sign-in with Apple, Google and GitHub, and the redirect
from a handle host to the person's page on linkjar.io. It does no I/O;
HTTP goes through the server's `Fetch`.

## Modules

| Module | Holds |
|---|---|
| `handle` | The labels and the rename rule: `rename`, `DOMAIN`. |
| `providers` | `Provider`, `ProviderId`: the authorization URLs, the token form and the ID-token checks of the three providers. |
| `reserved` | `is_reserved_label`. |

## Place in the workspace

Depends on pds-ext, pds-identity and pds-types. Used by linkjar-pds only,
which selects it when `PDS_PROFILE=linkjar`.

## What it needs

The server reads `PDS_EXTERNAL_APPLE_CLIENT_ID` and
`PDS_EXTERNAL_APPLE_CLIENT_SECRET`, and the same pair for `GOOGLE` and
`GITHUB`. Through `Fetch` the profile reaches the three providers'
endpoints. The link target `https://linkjar.io/jar/` is fixed.

## Tests

`cargo test -p pds-ext-linkjar`: inline tests, with the RSA key
`tests/data/test-rsa-key.pk8` for the ID-token checks. The LinkJar
scenarios of the parity harness hold the profile to the patched Reference.

## Specification

SPEC §13 (the patches the profile reproduces), §12.3 and §12.4.
