// Normalisation for oracle O1 (SPEC 17.3). Two targets never agree on the
// identifiers they generate, so each kind of server-generated value is
// replaced by an alias numbered by first sight: the first DID a scenario sees
// is `did:plc:<1>` on every target. Field order is ignored by sorting keys.
// Times are replaced by a marker because the Reference has no clock hook.

import { CID } from 'multiformats/cid'

type Rule = { kind: string; pattern: RegExp; render?: (alias: number, match: string) => string }

const b64url = (part: string): unknown => JSON.parse(Buffer.from(part, 'base64url').toString('utf8'))

const RULES: Rule[] = [
  { kind: 'did', pattern: /did:plc:[a-z2-7]{24}/g, render: (n) => `did:plc:<${n}>` },
  { kind: 'didkey', pattern: /did:key:z[1-9A-HJ-NP-Za-km-z]{40,}/g, render: (n) => `did:key:<${n}>` },
  { kind: 'multikey', pattern: /\bz(?:Q3s|Dn)[1-9A-HJ-NP-Za-km-z]{40,}/g },
  { kind: 'cid', pattern: /\bbaf[a-z2-7]{56}\b/g },
  { kind: 'tid', pattern: /\b[2-7][2-7a-z]{12}\b/g },
  { kind: 'invite', pattern: /\bpds-linkjar-social-[a-z2-7]{5}-[a-z2-7]{5}\b/g },
  { kind: 'oauth', pattern: /\b(?:req|cod|tok|ref|dev|ses)-[0-9a-f]{16,}\b/g },
  { kind: 'email-token', pattern: /\b[A-Z2-7]{5}-[A-Z2-7]{5}\b/g },
  { kind: 'app-password', pattern: /\b[a-z2-7]{4}-[a-z2-7]{4}-[a-z2-7]{4}-[a-z2-7]{4}\b/g },
]

const TIME = /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})/g
const JWT = /\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*/g

export class Normaliser {
  private readonly aliases = new Map<string, number>()
  private readonly counters = new Map<string, number>()

  /** The alias number of a value, assigned on first sight within its kind. */
  alias(kind: string, value: string): number {
    const key = `${kind}\u0000${value}`
    let alias = this.aliases.get(key)
    if (alias === undefined) {
      alias = (this.counters.get(kind) ?? 0) + 1
      this.counters.set(kind, alias)
      this.aliases.set(key, alias)
    }
    return alias
  }

  /**
   * A JWT becomes its comparable claims. The lifetime is kept because the
   * Candidate must mint tokens that live as long as the Reference's.
   */
  private jwt(token: string): string {
    try {
      const [head, body] = token.split('.') as [string, string]
      const header = b64url(head) as Record<string, unknown>
      const claims = b64url(body) as Record<string, unknown>
      const parts: string[] = [`alg=${String(header.alg)}`]
      if (header.typ !== undefined) parts.push(`typ=${String(header.typ)}`)
      for (const name of ['scope', 'iss', 'sub', 'aud', 'lxm', 'client_id']) {
        if (claims[name] !== undefined) parts.push(`${name}=${this.text(String(claims[name]))}`)
      }
      if (typeof claims.exp === 'number' && typeof claims.iat === 'number') parts.push(`lifetime=${claims.exp - claims.iat}`)
      for (const name of ['jti', 'cnf', 'nonce']) if (claims[name] !== undefined) parts.push(name)
      return `<jwt ${parts.join(' ')}>`
    } catch {
      return '<jwt unreadable>'
    }
  }

  text(input: string): string {
    let out = input.replace(JWT, (token) => this.jwt(token))
    out = out.replace(TIME, '<time>')
    for (const rule of RULES) {
      out = out.replace(rule.pattern, (match) => {
        const alias = this.alias(rule.kind, match)
        return rule.render ? rule.render(alias, match) : `<${rule.kind}:${alias}>`
      })
    }
    return out
  }

  /** Deep-normalises a decoded JSON or DAG-CBOR value. */
  value(input: unknown): unknown {
    if (typeof input === 'string') return this.text(input)
    if (input === null || typeof input !== 'object') return input
    if (input instanceof Uint8Array) return `<bytes:${input.byteLength}>`
    const cid = CID.asCID(input)
    if (cid) return this.text(cid.toString())
    if (Array.isArray(input)) return input.map((item) => this.value(item))
    const out: Record<string, unknown> = {}
    for (const key of Object.keys(input).sort()) {
      out[this.text(key)] = this.value((input as Record<string, unknown>)[key])
    }
    return out
  }
}
