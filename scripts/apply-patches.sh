#!/bin/sh
set -eu
export LC_ALL=C
case "$1" in
  production) directory=/patches ;;
  none) exit 0 ;;
  branding-smoke) directory=/examples/branding-smoke ;;
  *) echo "Unknown patch profile: $1" >&2; exit 1 ;;
esac
for patch in "$directory"/*.patch; do
  [ -f "$patch" ] || continue
  echo "Applying $(basename "$patch")"
  git apply --check "$patch"
  git apply "$patch"
done
