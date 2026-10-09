// The official OAuth client at the pin, pointed at a stack (SPEC 17.1,
// oracle O4). Client metadata is served by the fixture server under the
// client's own hostname, as a real client's would be.

import { JoseKey, NodeOAuthClient, atprotoLoopbackClientMetadata } from '@atproto/oauth-client-node'
import type { NodeSavedSession, NodeSavedState, OAuthClientMetadataInput } from '@atproto/oauth-client-node'
import type { Scenario } from '../scenario.ts'
import { HOSTS } from '../stack/targets.ts'

export type ClientKind = 'public' | 'confidential'

export type TestClient = {
  client: NodeOAuthClient
  clientId: string
  redirectUri: string
  metadata: OAuthClientMetadataInput
  /** The private key of a confidential client, to sign with or to take away. */
  key?: JoseKey
  /** Sessions the client holds, by DID. Scenarios read tokens from here for the negative cases. */
  sessions: Map<string, NodeSavedSession>
}

function memoryStore<V>(map = new Map<string, V>()) {
  return {
    map,
    async set(key: string, value: V) {
      map.set(key, value)
    },
    async get(key: string) {
      return map.get(key)
    },
    async del(key: string) {
      map.delete(key)
    },
  }
}

/**
 * Registers a client by publishing its metadata and returns the SDK client.
 * `host` defaults to the untrusted client host; `app.linkjar.io` is in the
 * server's trusted list.
 */
export async function makeClient(
  s: Scenario,
  name: string,
  kind: ClientKind,
  opts: { host?: string; path?: string; redirectPath?: string; scope?: string; metadata?: Partial<OAuthClientMetadataInput> } = {},
): Promise<TestClient> {
  const host = opts.host ?? HOSTS.client
  const path = opts.path ?? `/${name}-${s.tag}/client-metadata.json`
  const clientId = `https://${host}${path}`
  const redirectUri = `https://${host}${opts.redirectPath ?? `/${name}-${s.tag}/callback`}`
  const key = kind === 'confidential' ? await JoseKey.generate(['ES256'], `${name}-key-1`) : undefined
  const metadata: OAuthClientMetadataInput = {
    client_id: clientId,
    client_name: `Parity ${name}`,
    client_uri: `https://${host}`,
    redirect_uris: [redirectUri],
    grant_types: ['authorization_code', 'refresh_token'],
    response_types: ['code'],
    scope: opts.scope ?? 'atproto transition:generic',
    application_type: 'web',
    dpop_bound_access_tokens: true,
    ...(key
      ? { token_endpoint_auth_method: 'private_key_jwt', token_endpoint_auth_signing_alg: 'ES256', jwks: { keys: [key.publicJwk as never] } }
      : { token_endpoint_auth_method: 'none' }),
    ...opts.metadata,
  }
  await s.fixtures.serve({ host, path, body: JSON.stringify(metadata) })
  const sessions = memoryStore<NodeSavedSession>()
  const client = new NodeOAuthClient({
    clientMetadata: metadata,
    keyset: key ? [key] : undefined,
    stateStore: memoryStore<NodeSavedState>(),
    sessionStore: sessions,
    fetch: s.target.fetch({ ip: s.ip }),
    handleResolver: `https://${HOSTS.pds}`,
    plcDirectoryUrl: `https://${HOSTS.plc}`,
  })
  return { client, clientId, redirectUri, metadata, key, sessions: sessions.map }
}

/**
 * The LinkJar web app, which PDS_OAUTH_TRUSTED_CLIENTS lists. The server
 * caches client metadata by client_id, so every scenario publishes the same
 * document for it.
 */
export function makeAppClient(s: Scenario): Promise<TestClient> {
  return makeClient(s, 'app', 'public', { host: HOSTS.app, path: '/client-metadata.json', redirectPath: '/callback', metadata: { client_name: 'LinkJar' } })
}

/**
 * The LinkJar browser extension, also on the trusted list. It asks for the
 * permission to change the handle, which the web app does not.
 */
export function makeExtensionClient(s: Scenario): Promise<TestClient> {
  return makeClient(s, 'ext', 'public', {
    host: HOSTS.web,
    path: '/ext-client-metadata.json',
    redirectPath: '/ext/callback',
    scope: 'atproto transition:generic identity:handle',
    metadata: { client_name: 'LinkJar extension' },
  })
}

/**
 * A loopback client (SPEC 11.2): the client_id is `http://localhost` with the
 * redirect URI and the scope in its query, and the server synthesizes the
 * metadata. Nothing is published.
 */
export function makeLoopbackClient(s: Scenario, scope = 'atproto transition:generic'): TestClient {
  const redirectUri = 'http://127.0.0.1/callback'
  const clientId = `http://localhost?redirect_uri=${encodeURIComponent(redirectUri)}&scope=${encodeURIComponent(scope)}`
  const metadata = atprotoLoopbackClientMetadata(clientId)
  const sessions = memoryStore<NodeSavedSession>()
  const client = new NodeOAuthClient({
    clientMetadata: metadata,
    stateStore: memoryStore<NodeSavedState>(),
    sessionStore: sessions,
    fetch: s.target.fetch({ ip: s.ip }),
    handleResolver: `https://${HOSTS.pds}`,
    plcDirectoryUrl: `https://${HOSTS.plc}`,
  })
  return { client, clientId, redirectUri, metadata: { ...metadata, client_name: 'Loopback client' }, sessions: sessions.map }
}
