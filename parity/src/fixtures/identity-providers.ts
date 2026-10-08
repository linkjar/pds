// Stand-ins for Apple, Google and GitHub. Patch 099 fixes the provider
// endpoints in code, so the stack resolves those names to the edge and the
// edge forwards them here. Each provider behaves as the patch expects of the
// real one: Apple and Google issue RS256 ID tokens under a published key set,
// GitHub answers the user and email API.
//
// A scenario chooses who signs in next with `PUT /__control/identity`.

import { createPrivateKey, createSign, generateKeyPairSync, randomBytes } from 'node:crypto'
import type { KeyObject } from 'node:crypto'
import type { RecordedRequest } from './server.ts'

export type Provider = 'apple' | 'google' | 'github'

export type NextIdentity = {
  provider: Provider
  subject: string
  email?: string
  emailVerified?: boolean | string
  name?: string
  /** The provider reports that the person refused. */
  deny?: boolean
  /** Overrides for negative cases: claims merged into the ID token, or replaced GitHub email rows. */
  claims?: Record<string, unknown>
  githubEmails?: unknown[]
}

type Answer = { status: number; body: unknown; headers?: Record<string, string> }
type Grant = { identity: NextIdentity; nonce?: string; clientId: string; redirectUri: string }

const ISSUER = { apple: 'https://appleid.apple.com', google: 'https://accounts.google.com' } as const

const b64url = (value: unknown) => Buffer.from(JSON.stringify(value)).toString('base64url')

export class IdentityProviders {
  private readonly next = new Map<Provider, NextIdentity>()
  private readonly grants = new Map<string, Grant>()
  private readonly tokens = new Map<string, NextIdentity>()
  private key!: KeyObject
  private jwk!: Record<string, unknown>

  async init(): Promise<void> {
    const pair = generateKeyPairSync('rsa', { modulusLength: 2048 })
    this.key = createPrivateKey(pair.privateKey.export({ type: 'pkcs8', format: 'pem' }))
    this.jwk = { ...pair.publicKey.export({ format: 'jwk' }), kid: 'parity-1', alg: 'RS256', use: 'sig' }
  }

  setNext(identity: NextIdentity): void {
    this.next.set(identity.provider, identity)
  }

  private idToken(provider: 'apple' | 'google', grant: Grant): string {
    const now = Math.floor(Date.now() / 1000)
    const { identity } = grant
    const claims: Record<string, unknown> = {
      iss: ISSUER[provider],
      aud: grant.clientId,
      sub: identity.subject,
      nonce: grant.nonce,
      iat: now,
      exp: now + 300,
      email: identity.email,
      email_verified: identity.emailVerified ?? true,
      ...(provider === 'google' && identity.name ? { name: identity.name } : {}),
      ...identity.claims,
    }
    const signingInput = `${b64url({ alg: 'RS256', typ: 'JWT', kid: 'parity-1' })}.${b64url(claims)}`
    const signature = createSign('RSA-SHA256').update(signingInput).sign(this.key).toString('base64url')
    return `${signingInput}.${signature}`
  }

  /** Answers a provider request, or returns undefined when the host is not a provider. */
  async handle(request: RecordedRequest): Promise<Answer | undefined> {
    const at = `${request.host}${request.path}`
    const form = new URLSearchParams(request.body)
    const json = (body: unknown): Answer => ({ status: 200, body, headers: { 'content-type': 'application/json' } })

    // Authorization endpoints: the browser arrives here from the PDS.
    const authorize: Record<string, Provider> = {
      'appleid.apple.com/auth/authorize': 'apple',
      'accounts.google.com/o/oauth2/v2/auth': 'google',
      'github.com/login/oauth/authorize': 'github',
    }
    const provider = authorize[at]
    if (provider) {
      const identity = this.next.get(provider)
      const redirectUri = request.query.redirect_uri ?? ''
      const state = request.query.state ?? ''
      if (!identity) return { status: 400, body: `No identity queued for ${provider}` }
      const params = new URLSearchParams({ state })
      if (identity.deny) {
        params.set('error', 'access_denied')
      } else {
        const code = `code-${randomBytes(12).toString('hex')}`
        this.grants.set(code, { identity, nonce: request.query.nonce, clientId: request.query.client_id ?? '', redirectUri })
        params.set('code', code)
      }
      if (provider === 'apple') {
        // Apple answers with a cross-site form post. The name arrives once, in `user`.
        if (identity.name && !identity.deny) {
          const [firstName, ...rest] = identity.name.split(' ')
          params.set('user', JSON.stringify({ name: { firstName, lastName: rest.join(' ') }, email: identity.email }))
        }
        const inputs = [...params].map(([k, v]) => `<input type="hidden" name="${k}" value="${v.replace(/"/g, '&quot;')}">`).join('')
        const page = `<!doctype html><body onload="document.forms[0].submit()"><form method="post" action="${redirectUri}">${inputs}</form></body>`
        return { status: 200, body: page, headers: { 'content-type': 'text/html' } }
      }
      return { status: 302, body: '', headers: { location: `${redirectUri}?${params}` } }
    }

    // Token endpoints: the PDS calls these.
    if (at === 'appleid.apple.com/auth/token' || at === 'oauth2.googleapis.com/token') {
      const grant = this.grants.get(form.get('code') ?? '')
      if (!grant) return { status: 400, body: { error: 'invalid_grant' }, headers: { 'content-type': 'application/json' } }
      this.grants.delete(form.get('code') ?? '')
      const which = at.startsWith('appleid') ? 'apple' : 'google'
      return json({ id_token: this.idToken(which, grant), access_token: 'unused-access', token_type: 'Bearer', expires_in: 300 })
    }
    if (at === 'appleid.apple.com/auth/keys' || at === 'www.googleapis.com/oauth2/v3/certs') {
      return json({ keys: [this.jwk] })
    }
    if (at === 'github.com/login/oauth/access_token') {
      const grant = this.grants.get(form.get('code') ?? '')
      if (!grant) return json({ error: 'bad_verification_code' })
      this.grants.delete(form.get('code') ?? '')
      const token = `gho_${randomBytes(12).toString('hex')}`
      this.tokens.set(token, grant.identity)
      return json({ access_token: token, token_type: 'bearer', scope: 'read:user,user:email' })
    }
    if (request.host === 'api.github.com') {
      const identity = this.tokens.get((request.headers.authorization ?? '').replace(/^(Bearer|token) /i, ''))
      if (!identity) return { status: 401, body: { message: 'Bad credentials' }, headers: { 'content-type': 'application/json' } }
      if (request.path === '/user') {
        return json({ id: Number(identity.subject), login: 'parity-user', name: identity.name ?? null, email: null })
      }
      if (request.path === '/user/emails') {
        return json(
          identity.githubEmails ?? [{ email: identity.email, primary: true, verified: identity.emailVerified ?? true, visibility: 'private' }],
        )
      }
    }
    return undefined
  }
}
