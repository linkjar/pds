// The official OAuth client at the pin, pointed at a stack (SPEC 17.1,
// oracle O4). Client metadata is served by the fixture server under the
// client's own hostname, as a real client's would be.

import { JoseKey, NodeOAuthClient } from '@atproto/oauth-client-node'
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
  opts: { host?: string; path?: string; scope?: string; metadata?: Partial<OAuthClientMetadataInput> } = {},
): Promise<TestClient> {
  const host = opts.host ?? HOSTS.client
  const path = opts.path ?? `/${name}-${s.tag}/client-metadata.json`
  const clientId = `https://${host}${path}`
  const redirectUri = `https://${host}/${name}-${s.tag}/callback`
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
