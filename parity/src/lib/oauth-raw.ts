// A hand-written OAuth client for the cases the official client refuses to
// produce: redeeming a code twice, replaying a refresh token, signing a DPoP
// proof with the wrong key. It speaks PAR, PKCE S256 and DPoP (RFC 9449)
// directly, and records every exchange in the transcript.

import { createHash, randomBytes, webcrypto } from 'node:crypto'
import type { Result, Scenario } from '../scenario.ts'
import { HOSTS } from '../stack/targets.ts'

const b64url = (value: unknown) => Buffer.from(JSON.stringify(value)).toString('base64url')
const ISSUER = `https://${HOSTS.pds}`

export type DpopKey = { privateKey: webcrypto.CryptoKey; jwk: Record<string, unknown>; thumbprint: string }

export async function generateDpopKey(): Promise<DpopKey> {
  const pair = await webcrypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify'])
  const { kty, crv, x, y } = (await webcrypto.subtle.exportKey('jwk', pair.publicKey)) as Record<string, string>
  const jwk = { kty, crv, x, y }
  // RFC 7638: members in lexicographic order.
  const thumbprint = createHash('sha256').update(JSON.stringify({ crv, kty, x, y })).digest('base64url')
  return { privateKey: pair.privateKey, jwk, thumbprint }
}

async function proof(key: DpopKey, method: string, url: string, nonce?: string, accessToken?: string): Promise<string> {
  const claims: Record<string, unknown> = {
    jti: randomBytes(12).toString('hex'),
    htm: method,
    htu: url.split('?')[0],
    iat: Math.floor(Date.now() / 1000),
  }
  if (nonce) claims.nonce = nonce
  if (accessToken) claims.ath = createHash('sha256').update(accessToken).digest('base64url')
  const input = `${b64url({ typ: 'dpop+jwt', alg: 'ES256', jwk: key.jwk })}.${b64url(claims)}`
  const signature = await webcrypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, key.privateKey, Buffer.from(input))
  return `${input}.${Buffer.from(signature).toString('base64url')}`
}

export type RawClient = {
  clientId: string
  redirectUri: string
  key: DpopKey
  /** The DPoP nonces last seen, for the authorization server and for the resource server. */
  nonce: { as?: string; rs?: string }
}

export function pkce(): { verifier: string; challenge: string } {
  const verifier = randomBytes(32).toString('base64url')
  return { verifier, challenge: createHash('sha256').update(verifier).digest('base64url') }
}

/**
 * Sends a form to an authorization-server endpoint with a DPoP proof. A
 * `use_dpop_nonce` answer is retried once with the nonce the server gave,
 * which is the protocol's own first step and is kept out of the transcript.
 */
export async function tokenEndpoint(
  s: Scenario,
  step: string,
  client: RawClient,
  path: '/oauth/par' | '/oauth/token' | '/oauth/revoke',
  form: Record<string, string>,
  opts: { key?: DpopKey; dpop?: boolean } = {},
): Promise<Result> {
  const url = `${ISSUER}${path}`
  const send = async (silent: boolean) => {
    const headers: Record<string, string> = {}
    if (opts.dpop !== false) headers.dpop = await proof(opts.key ?? client.key, 'POST', url, client.nonce.as)
    const result = await s.http(step, {
      method: 'POST',
      path,
      body: new URLSearchParams({ client_id: client.clientId, ...form }).toString(),
      contentType: 'application/x-www-form-urlencoded',
      headers,
      silent,
    })
    const nonce = result.headers.get('dpop-nonce')
    if (nonce) client.nonce.as = nonce
    return result
  }
  const first = await send(true)
  if (first.json?.error === 'use_dpop_nonce') return send(false)
  // Record the exchange that counted. Replaying it would change server state, so record from the result.
  s.entries.push({
    step,
    request: `POST ${HOSTS.pds}${path}`,
    status: first.status,
    headers: first.headers.has('dpop-nonce') ? { 'dpop-nonce': '<nonce>' } : {},
    body: s.norm.value(first.json ?? first.text),
  })
  return first
}

/** Calls the resource server with a DPoP-bound access token. */
export async function resource(
  s: Scenario,
  step: string,
  client: RawClient,
  accessToken: string,
  request: { method?: string; path: string; query?: Record<string, string>; json?: unknown },
  opts: { key?: DpopKey; scheme?: 'DPoP' | 'Bearer' } = {},
): Promise<Result> {
  const method = request.method ?? (request.json === undefined ? 'GET' : 'POST')
  const url = `${ISSUER}${request.path}`
  const send = async (silent: boolean) => {
    const result = await s.http(step, {
      method,
      path: request.path,
      query: request.query,
      json: request.json,
      headers: {
        authorization: `${opts.scheme ?? 'DPoP'} ${accessToken}`,
        dpop: await proof(opts.key ?? client.key, method, url, client.nonce.rs, accessToken),
      },
      silent,
    })
    const nonce = result.headers.get('dpop-nonce')
    if (nonce) client.nonce.rs = nonce
    return result
  }
  const first = await send(true)
  if (first.status === 401 && /use_dpop_nonce/.test(first.headers.get('www-authenticate') ?? '')) return send(false)
  s.entries.push({
    step,
    request: `${method} ${HOSTS.pds}${request.path}`,
    status: first.status,
    headers: first.headers.has('www-authenticate') ? { 'www-authenticate': s.norm.text(first.headers.get('www-authenticate')!) } : {},
    body: s.norm.value(first.json ?? first.text),
  })
  return first
}
