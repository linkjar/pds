// The official MCP TypeScript client against the MCP server of LinkJar
// (sidecars.md §2), for the gate of unit 10. The Rust test
// `the_official_mcp_client_signs_in_through_the_pds` in
// crates/linkjar-pds/tests/oauth.rs starts the PDS and the MCP server and
// runs this script with the public address of the MCP endpoint and the
// local address of the server. The fetch of the script sends a request for
// the public origin to the local one, as DNS would.
//
// The test is the browser of the person. The script writes one JSON line
// for each step: `{"authorize": <url>}` when the SDK sends the person to
// sign in, and `{"done": <results>}` at the end. The test answers an
// authorize line with the URL that the browser came back to, on one line
// of standard input.
//
// The script does discovery, registration, PKCE, the code exchange and a
// refresh through the SDK, and the MCP calls through its client. The SDK
// has no revocation, so that one is a plain request to the endpoint that
// the metadata names. The Inspector, when the test gives its binary in
// MCP_INSPECTOR, lists the tools with the access token.

import { execFileSync } from 'node:child_process'
import { createInterface } from 'node:readline'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { UnauthorizedError, auth, discoverAuthorizationServerMetadata } from '@modelcontextprotocol/sdk/client/auth.js'
import type { OAuthClientProvider } from '@modelcontextprotocol/sdk/client/auth.js'
import type { OAuthClientInformationMixed, OAuthClientMetadata, OAuthTokens } from '@modelcontextprotocol/sdk/shared/auth.js'

const endpoint = new URL(process.argv[2]!)
const local = new URL(process.argv[3]!)
const fetchFn = (input: string | URL | Request, init?: RequestInit) => {
  const url = new URL(input instanceof Request ? input.url : input)
  if (url.origin === endpoint.origin) {
    url.protocol = local.protocol
    url.host = local.host
  }
  return input instanceof Request ? fetch(new Request(url, input), init) : fetch(url, init)
}
const REDIRECT = 'http://127.0.0.1/official-callback'
const SCOPE = 'account:read repo:read repo:write:com.example.note'
const NOTE = 'com.example.note'

const lines = createInterface({ input: process.stdin })[Symbol.asyncIterator]()
const say = (message: unknown) => process.stdout.write(JSON.stringify(message) + '\n')

/** The memory of one AI client: what a real one keeps on disk. */
class Memory implements OAuthClientProvider {
  client?: OAuthClientInformationMixed
  saved?: OAuthTokens
  verifier = ''
  sent?: URL
  get redirectUrl() {
    return REDIRECT
  }
  get clientMetadata(): OAuthClientMetadata {
    return {
      client_name: 'Official SDK',
      redirect_uris: [REDIRECT],
      grant_types: ['authorization_code', 'refresh_token'],
      response_types: ['code'],
      token_endpoint_auth_method: 'none',
      scope: SCOPE,
    }
  }
  clientInformation() {
    return this.client
  }
  saveClientInformation(client: OAuthClientInformationMixed) {
    this.client = client
  }
  tokens() {
    return this.saved
  }
  saveTokens(tokens: OAuthTokens) {
    this.saved = tokens
  }
  redirectToAuthorization(url: URL) {
    this.sent = url
  }
  saveCodeVerifier(verifier: string) {
    this.verifier = verifier
  }
  codeVerifier() {
    return this.verifier
  }
}

const memory = new Memory()
const results: Record<string, unknown> = {}

async function connect(): Promise<Client> {
  const client = new Client({ name: 'linkjar-official-client', version: '1.0.0' })
  await client.connect(new StreamableHTTPClientTransport(endpoint, { authProvider: memory, fetch: fetchFn }))
  return client
}

// Discovery and registration, then the browser.
try {
  await connect()
  throw new Error('the server took a client without a token')
} catch (error) {
  if (!(error instanceof UnauthorizedError)) throw error
}
results.registered = memory.client?.client_id !== undefined
results.authorizeScope = memory.sent?.searchParams.get('scope')
results.authorizeResource = memory.sent?.searchParams.get('resource')
say({ authorize: memory.sent!.toString() })
const back = new URL((await lines.next()).value as string)
results.state = back.searchParams.get('state') === memory.sent!.searchParams.get('state')
const transport = new StreamableHTTPClientTransport(endpoint, { authProvider: memory, fetch: fetchFn })
await transport.finishAuth(back.searchParams.get('code')!)
results.scope = memory.saved?.scope

// The protocol through the official client.
const client = await connect()
results.server = client.getServerVersion()?.name
const tools = await client.listTools()
results.tools = tools.tools.map((tool) => tool.name).sort()
const who = await client.callTool({ name: 'whoami', arguments: {} })
results.did = (who.structuredContent as { did?: string } | undefined)?.did
const made = await client.callTool({ name: 'create_record', arguments: { collection: NOTE, rkey: 'official', record: { $type: NOTE, text: 'from the official client' } } })
results.created = (made.structuredContent as { uri?: string } | undefined)?.uri
const templates = await client.listResourceTemplates()
results.templates = templates.resourceTemplates.map((template) => template.uriTemplate)
const read = await client.readResource({ uri: results.created as string })
results.read = JSON.stringify(read.contents).includes('from the official client')
const prompts = await client.listPrompts()
results.prompts = prompts.prompts.length

// The Inspector, with the token the SDK holds.
if (process.env.MCP_INSPECTOR) {
  const listed = execFileSync(
    process.env.MCP_INSPECTOR,
    ['--cli', new URL(endpoint.pathname, local).toString(), '--transport', 'http', '--header', `Authorization: Bearer ${memory.saved!.access_token}`, '--method', 'tools/list'],
    { encoding: 'utf8', env: { ...process.env, NO_COLOR: '1' } },
  )
  results.inspectorTools = (JSON.parse(listed) as { tools: { name: string }[] }).tools.length
}

// A refresh through the SDK: with a refresh token it does not ask the person.
const first = memory.saved!
results.refresh = await auth(memory, { serverUrl: endpoint, fetchFn })
results.rotated = memory.saved!.access_token !== first.access_token && memory.saved!.refresh_token !== first.refresh_token
const status = async (token: string) =>
  (
    await fetchFn(endpoint, {
      method: 'POST',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json', accept: 'application/json, text/event-stream' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'ping' }),
    })
  ).status
results.oldAccess = await status(first.access_token)
results.newAccess = await status(memory.saved!.access_token)

// Revocation, at the endpoint of the metadata.
const metadata = await discoverAuthorizationServerMetadata(new URL('/', endpoint), { fetchFn })
const revocation = (metadata as { revocation_endpoint?: string } | undefined)?.revocation_endpoint
const revoked = await fetchFn(revocation!, {
  method: 'POST',
  headers: { 'content-type': 'application/x-www-form-urlencoded' },
  body: new URLSearchParams({ token: memory.saved!.access_token, client_id: memory.client!.client_id }),
})
results.revoked = revoked.status
results.afterRevocation = await status(memory.saved!.access_token)
await client.close()

say({ done: results })
process.exit(0)
