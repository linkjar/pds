// Service-auth tokens (SPEC 5.4): reading one and verifying its signature
// against the issuer's key in the PLC directory.

import assert from 'node:assert/strict'
import { verifySignature } from '@atproto/crypto'
import { decodeJwt } from '../lib/jwt.ts'
import type { Scenario } from '../scenario.ts'
import { signingKey } from './repo.ts'

export type ServiceToken = { header: Record<string, any>; claims: Record<string, any>; lifetime: number }

/** Decodes a service-auth JWT and requires a valid signature by `issuer`'s repository key. */
export async function verifyServiceToken(s: Scenario, token: string, issuer: string): Promise<ServiceToken> {
  const [header, payload, signature] = token.split('.') as [string, string, string]
  const decoded = decodeJwt(token)
  assert.equal(decoded.claims.iss, issuer, 'iss is the calling account')
  const didKey = await signingKey(s, issuer)
  const valid = await verifySignature(didKey, Buffer.from(`${header}.${payload}`), Buffer.from(signature, 'base64url'))
  assert.ok(valid, "the token is signed with the account's repository key")
  return { ...decoded, lifetime: decoded.claims.exp - decoded.claims.iat }
}

/** The bearer token of a request the fixture server recorded. */
export function bearer(headers: Record<string, string>): string {
  const value = headers.authorization ?? ''
  assert.match(value, /^Bearer /, 'the forwarded request carries a bearer token')
  return value.slice('Bearer '.length)
}
