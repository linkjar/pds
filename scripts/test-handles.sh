#!/bin/sh
set -eu
# Use - for a fully built production image. A source path is a local-only overlay
# with Node's TypeScript transform; it does not substitute for the CI typecheck.
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
source_dir=${1:--}
image=${2:-linkjar-pds:production}
if [ "$source_dir" = - ]; then
  docker run --rm --user root --entrypoint node -e LINKJAR_USE_COMPILED=1 \
    -v "$root/tests:/linkjar-tests:ro" "$image" /linkjar-tests/handles.mjs
else
  docker run --rm --user root --entrypoint node \
    -v "$source_dir:/patch-src:ro" -v "$root/tests:/linkjar-tests:ro" \
    "$image" /linkjar-tests/handles.mjs
fi
