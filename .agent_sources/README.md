# Source mirrors

Checkouts of the upstream sources this project is specified against, for
reading. Nothing builds from them and git ignores them.

```sh
scripts/vendor-agent-sources.sh            # fetch what is missing or stale
scripts/vendor-agent-sources.sh --refresh  # fetch every mirror again
scripts/vendor-agent-sources.sh --list     # print the pins
```

Layout: `.agent_sources/github.com/<owner>/<repo>/`, one shallow checkout of
one commit, with `.agent-source.json` beside it. `AGENT_SOURCES_DIR` moves
the mirrors, for example to share one copy between worktrees.

Each commit comes from a pin this repository already records. Move the pin,
run the script, and the mirror follows.

| Mirror | Pinned by | Read it for |
|---|---|---|
| `github.com/bluesky-social/atproto` | [`upstream.json`](../upstream.json) | The Reference. `packages/pds` is the server, `packages/repo` the repository library, `packages/oauth` the authorization server, `lexicons/` the schemas. An "at the pin" statement in the SPEC is checked here. |
| `github.com/bluesky-social/atproto-interop-tests` | [`interop/README.md`](../interop/README.md) | Upstream of the vectors in `interop/`. |
| `github.com/bluesky-social/indigo` | [`parity/images.json`](../parity/images.json), `relay.tag` | The Sync 1.1 relay of the parity stack, `cmd/relay`. |
| `github.com/did-method-plc/did-method-plc` | `parity/images.json`, `plc.commit` | The PLC directory of the parity stack. |
| `github.com/blacksky-algorithms/rsky` | [`bench/Cargo.toml`](../bench/Cargo.toml) | `rsky-repo` as compared in decision D2. |

Cite a mirror by repository, commit and path, as the research notes do, and
not by its local path.

To add a mirror, add a `mirror` line to the script with the file that pins
its commit, and a row here.
