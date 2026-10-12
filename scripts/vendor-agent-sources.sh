#!/usr/bin/env bash
# Source mirrors for reading, under .agent_sources/github.com/<owner>/<repo>.
#
# Each mirror is a shallow checkout of one commit, and the commit comes from
# a pin this repository already records. Move a pin, run the script, and the
# mirror follows. The mirrors are reference material: nothing builds from
# them, and git ignores them.
#
#   scripts/vendor-agent-sources.sh            # fetch what is missing or stale
#   scripts/vendor-agent-sources.sh --refresh  # fetch every mirror again
#   scripts/vendor-agent-sources.sh --list     # print the pins and stop
#
# AGENT_SOURCES_DIR moves the mirrors, for example to share one copy between
# worktrees. The default is .agent_sources/github.com in this checkout.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${AGENT_SOURCES_DIR:-${ROOT}/.agent_sources/github.com}"
MODE=fetch

for arg in "$@"; do
  case "$arg" in
    --refresh) MODE=refresh ;;
    --list) MODE=list ;;
    -h | --help)
      sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

command -v jq >/dev/null || { echo "jq is required (devenv provides it)" >&2; exit 1; }

mirror() {
  local owner="$1" repo="$2" commit="$3" pin="$4" note="$5"
  local remote="https://github.com/${owner}/${repo}.git"
  local dest="${PREFIX}/${owner}/${repo}"
  local metadata="${dest}/.agent-source.json"

  if [[ ! "$commit" =~ ^[0-9a-f]{40}$ ]]; then
    echo "${owner}/${repo}: ${pin} does not hold a full commit id (got '${commit}')" >&2
    exit 1
  fi
  if [[ "$MODE" == list ]]; then
    printf '%s/%s %s (%s)\n' "$owner" "$repo" "$commit" "$pin"
    return 0
  fi
  if [[ "$MODE" == fetch && -d "${dest}/.git" ]] &&
    [[ "$(git -C "$dest" rev-parse HEAD 2>/dev/null)" == "$commit" ]]; then
    echo "ok    ${owner}/${repo} @ ${commit:0:12}"
    return 0
  fi

  mkdir -p "$dest"
  if [[ ! -d "${dest}/.git" ]]; then
    git -C "$dest" init --quiet
    git -C "$dest" remote add origin "$remote"
  fi
  echo "fetch ${owner}/${repo} @ ${commit:0:12}"
  git -C "$dest" fetch --quiet --depth 1 origin "$commit"
  git -C "$dest" checkout --quiet --detach --force FETCH_HEAD

  jq -n \
    --arg owner "$owner" --arg repo "$repo" --arg remote "$remote" \
    --arg commit "$commit" --arg pin "$pin" --arg note "$note" \
    --arg fetchedAt "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{type: "github-repo-source", owner: $owner, repo: $repo, remote: $remote,
      commit: $commit, pinnedBy: $pin, fetchedAt: $fetchedAt, note: $note}' >"$metadata"
}

# The pins. Each value is read from the file that owns it.
ATPROTO="$(jq -r .commit "${ROOT}/upstream.json")"
INTEROP="$(grep -oE '[0-9a-f]{40}' "${ROOT}/interop/README.md" | head -n 1)"
RELAY="$(jq -r .relay.tag "${ROOT}/parity/images.json" | sed 's/^relay-//')"
PLC="$(jq -r .plc.commit "${ROOT}/parity/images.json")"
RSKY="$(grep -oE 'rsky", rev = "[0-9a-f]{40}' "${ROOT}/bench/Cargo.toml" | head -n 1 | grep -oE '[0-9a-f]{40}')"

mirror bluesky-social atproto "$ATPROTO" "upstream.json" \
  "The Reference: packages/pds, packages/repo, packages/oauth, lexicons. The oracle for SPEC.md."
mirror bluesky-social atproto-interop-tests "$INTEROP" "interop/README.md" \
  "Upstream of the vectors vendored in interop/."
mirror bluesky-social indigo "$RELAY" "parity/images.json relay.tag" \
  "The Sync 1.1 relay of the parity stack: cmd/relay."
mirror did-method-plc did-method-plc "$PLC" "parity/images.json plc.commit" \
  "The PLC directory of the parity stack."
mirror blacksky-algorithms rsky "$RSKY" "bench/Cargo.toml" \
  "rsky-repo as compared in decision D2 (main branch form)."

[[ "$MODE" == list ]] || echo "mirrors are under ${PREFIX}"
