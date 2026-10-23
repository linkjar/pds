---
title: Connect an AI client with MCP
description: Run the optional MCP server and connect an AI client to your PDS account with OAuth and scoped permissions.
---

The MCP server lets an AI client read your account's records and, when you
allow it, write to selected collections. Your chosen AI application runs the
model. The PDS and MCP server do not need an LLM API key.

Use a client that supports **remote Streamable HTTP with OAuth**. This server
does not expose a local stdio transport. Client support and setup screens vary;
check your client's documentation for that combination.

## Connection at a glance

| Setting in your AI client | Value for the example deployment |
|---|---|
| Server URL | `https://mcp.example.com/mcp` |
| Transport | Streamable HTTP |
| Authentication | OAuth through the browser |

```text title="MCP server URL — replace the example hostname"
https://mcp.example.com/mcp
```

The AI client connects to the MCP sidecar. The sidecar connects to the PDS
through OAuth and authenticated repository APIs. It does not grant shell
access, install software, or administer the host.

## Prepare the server

First finish [PDS installation](/start/overview/) and
[create an account](/start/first-account/). Give MCP its own DNS name, such as
`mcp.example.com`, pointing to the host that will run it.

[Build `linkjar-pds-mcp`](/reference/binaries/#build-the-sidecars) from a reviewed
checkout, or build `Dockerfile.mcp`. The portable PDS download does not contain it.
The following addresses assume native services on one Linux host.

### Set the environment

Store these settings in a protected runtime environment file. Generate a fresh
P-256 private key with your key-management tool and store its 32-byte private
scalar as 64 hexadecimal digits. This is a separate MCP client key, not the
PDS's secp256k1 PLC rotation key. Do not paste either key into your AI client.

```dotenv title="MCP service environment"
MCP_ADDRESS=127.0.0.1:2586
MCP_PUBLIC_URL=https://mcp.example.com
MCP_PDS_PUBLIC_URL=https://pds.example.com
MCP_PDS_INTERNAL_URL=http://127.0.0.1:2583
MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX=REPLACE_WITH_MCP_P256_PRIVATE_KEY
MCP_WRITE_COLLECTIONS=
```

Leave `MCP_WRITE_COLLECTIONS` empty for the initial read-only connection.
`MCP_PDS_PUBLIC_URL` must match the PDS's public OAuth issuer even when the
internal connection uses loopback.

### Allow the client at the PDS

Add this metadata URL to the PDS's `PDS_MCP_CLIENT_IDS` allowlist. Preserve
any existing entries; this setting is a comma-separated list.

```dotenv title="PDS service environment"
PDS_MCP_CLIENT_IDS=https://mcp.example.com/oauth-client-metadata.json
```

Restart the PDS through its service manager after updating the setting. If you
use `PDS_OAUTH_KNOWN_CLIENTS_FILE`, keep its MCP metadata and public key aligned
with the running sidecar.

### Run the sidecar behind HTTPS

Run `linkjar-pds-mcp` without arguments, as a separate unprivileged service.
Have the service manager load its protected environment file. On a native
Linux host, adapt the [systemd example](/start/native/#linux-binary-with-systemd)
with a separate service user, the MCP executable path, and its environment file.
MCP does not need access to the PDS data directory.

Proxy the **whole MCP origin**, including OAuth discovery and callbacks:

```caddyfile title="Caddyfile — native services on the same host"
mcp.example.com {
    reverse_proxy 127.0.0.1:2586
}
```

Keep port `2586` private. For Docker, bind inside the container and proxy through
the container network instead of using the native loopback addresses.

Check the public health and discovery routes:

```bash title="Verify MCP discovery"
curl --fail --silent --show-error https://mcp.example.com/health
curl --fail --silent --show-error https://mcp.example.com/.well-known/oauth-protected-resource
curl --fail --silent --show-error https://mcp.example.com/.well-known/oauth-authorization-server
curl --fail --silent --show-error https://mcp.example.com/oauth-client-metadata.json
```

## Connect and try a read

1. Add `https://mcp.example.com/mcp` as a remote server in your AI client.
2. Start the OAuth sign-in flow. Sign in on your PDS's page and review consent.
3. Ask the client to use `whoami`, then `list_collections`.
4. Try `list_records` on one collection you recognize.

These are the available repository tools:

| Tools | Purpose |
|---|---|
| `whoami` | Identify the signed-in account |
| `describe_repo`, `list_collections` | Inspect the repository and its collections |
| `list_records`, `get_record` | Read records |
| `create_record`, `put_record`, `delete_record` | Write records when the session has permission for the collection |

The client may send tool results to its configured model provider. Choose
records and a provider appropriate for the data you intend to share.

## Enable selected writes

Set `MCP_WRITE_COLLECTIONS` to the comma-separated collection NSIDs you want
the client to be able to request. For example:

```dotenv title="Example allowlist — use your application's collection"
MCP_WRITE_COLLECTIONS=com.example.note
```

Restart the sidecar, refresh any pinned client metadata at the PDS, and sign in
again to request the new permissions. The allowlist alone does not authorize a
session: the user must grant access, and the PDS still validates each write.
Review `put_record` and `delete_record` calls carefully; they can replace or
remove existing records.

## Sessions and troubleshooting

Run one MCP instance. Its registrations, tokens, and sessions are held in memory;
a restart ends them. Reconnect and sign in again after a restart. For a
service-local check, run `linkjar-pds-mcp health` with the same environment.

| Symptom | Check |
|---|---|
| The client only accepts a command and arguments | Use a client with remote Streamable HTTP and OAuth support |
| Discovery succeeds but sign-in fails | Public URLs, the PDS client allowlist, pinned metadata, and the callback route |
| Connection uses a container name on a native host | Change the internal URL to an address reachable from the sidecar |
| Reads work but a write is denied | The collection allowlist and the permissions granted to this session |
| The client loses access after a restart | Reconnect; sessions are intentionally in memory |

For the protocol background, see the official
[MCP authorization specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization).
For deployment boundaries, see [binaries and ports](/reference/binaries/).
