// JWT helpers. Scenarios read claims to assert lifetimes, and mint legacy
// session tokens with the stack's own PDS_JWT_SECRET to reach states that
// would otherwise need a clock: an expired token is a token whose `exp` the
// harness wrote in the past. The Reference derives its HS256 key from the
// UTF-8 bytes of the secret, and so does this.

import { createHmac } from 'node:crypto'

const part = (value: unknown) => Buffer.from(JSON.stringify(value)).toString('base64url')

export function decodeJwt(token: string): { header: Record<string, any>; claims: Record<string, any> } {
  const [header, claims] = token.split('.') as [string, string]
  return {
    header: JSON.parse(Buffer.from(header, 'base64url').toString('utf8')),
    claims: JSON.parse(Buffer.from(claims, 'base64url').toString('utf8')),
  }
}

export function signHs256(secret: string, header: Record<string, unknown>, claims: Record<string, unknown>): string {
  const input = `${part({ alg: 'HS256', ...header })}.${part(claims)}`
  return `${input}.${createHmac('sha256', Buffer.from(secret, 'utf8')).update(input).digest('base64url')}`
}

/** Re-signs a legacy session token with some claims changed. */
export function reissue(secret: string, token: string, changes: Record<string, unknown>): string {
  const { header, claims } = decodeJwt(token)
  return signHs256(secret, header, { ...claims, ...changes })
}

/** Mints a legacy access token as the Reference would, for a scope, account and audience of the caller's choice. */
export function mintLegacyToken(secret: string, claims: { scope: string; sub: string; aud: string; expiresInSeconds?: number; typ?: string }): string {
  const now = Math.floor(Date.now() / 1000)
  const { expiresInSeconds = 3600, typ = 'at+jwt', ...rest } = claims
  return signHs256(secret, { typ }, { ...rest, iat: now - 120, exp: now + expiresInSeconds })
}
