# Unit 10: the MCP server

Date: 2026-10-13 · [Plan](../plan.md) · [SPEC §22.2](../SPEC.md#222-mcp-bridge) · [sidecars.md §2](../sidecars.md#2-the-mcp-server)

An AI client that speaks the Model Context Protocol can now read and
write the repository of a person, after the person signs in on the pages
of the PDS.

## 1. Result

| Part | State |
|---|---|
| OAuth 2.1 authorization server for AI clients | Built: metadata, registration, metadata documents, PKCE, refresh rotation, revocation. |
| Consent for the AI client | Built: a page of the MCP server, after the sign-in at the PDS. |
| Confidential client of the PDS | Built: `private_key_jwt` and DPoP. |
| MCP over Streamable HTTP | Built: JSON answers, eight tools, one resource template, two prompts. |
| Audit of writes | Built in the PDS: the MCP server and the AI client. |
| Known clients of the PDS | Built: `PDS_OAUTH_KNOWN_CLIENTS_FILE`. |
| Official client and Inspector in the Harness | Not done. |

The gate of the unit names the official MCP client and the Inspector.
They did not run. The test of this unit is a client written for it, so
the unit proves the server against its own reading of the specification.

## 2. How it is built

- **Two faces.** A token of an AI client is a random bearer token that
  the server keeps by its hash. A token of the PDS is DPoP-bound and
  stays in the server. Neither reaches the other side.
- **Why the server asks again.** The PDS shows "LinkJar MCP" on its
  consent page, and it skips the page for a client that the account
  authorized before. Without a second question, a link to `/authorize`
  with the client id of an attacker gives the attacker a code. The server
  asks on a page of its own each time. The page names the AI client, the
  account and the host of the redirect URI. The form needs the cookie of
  the browser that started the request and a token of the request.
- **Scopes.** Reads need the identity only, because the PDS takes any
  OAuth token for a read of a repository. A write scope maps to a `repo:`
  scope. The PDS takes a scope only when the client metadata declares it,
  so `MCP_WRITE_COLLECTIONS` lists the collections that can be asked one
  at a time.
- **Clients without a fetch.** The PDS reads a file of client metadata at
  start. This serves the MCP server on a private network, and it serves
  the tests, which have no HTTPS host.
- **The client half of OAuth** (the DPoP proof and the client assertion)
  moved to `pds-oauth::agent`. The tests there check each against the
  server half.
- **JSON-RPC by hand.** The endpoint takes one message for each POST and
  answers with JSON, which the Streamable HTTP transport allows. The
  workspace gets no MCP dependency.

## 3. How it is checked

- `crates/linkjar-pds/tests/oauth.rs`: one AI client from discovery to
  revocation, against the PDS and the MCP server over HTTP. It covers the
  challenge, registration, a denied and an allowed consent, a forged
  form, a second browser, the code, the tools, a write outside the
  scope, a resource of another account, the audit entry, refresh
  rotation with a replay, and revocation on both sides.
- `crates/pds-mcp`: the configuration and the client metadata, the
  scope map, the sessions and their budget, redirect URIs, the address
  check, the tool schemas and the prompts.
- `crates/pds-oauth`: the proof and the assertion against the checks of
  the server.

## 4. What is not built

See [sidecars.md](../sidecars.md) §2.8. The three that matter most:

1. **A store that outlives the process.** A restart ends every session,
   and two instances do not share one.
2. **The official client and the Inspector.** Until they run, a
   difference from what real clients send is possible.
3. **Schemas from the lexicons.** A model sees `record` as an object and
   learns its shape from a refusal of the PDS.

## 5. Dependencies

None is new. `cargo vet` needs no new exemption.
