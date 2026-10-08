// The scenario runtime. A scenario is one `node:test` test that drives one
// target and writes a transcript: every HTTP exchange and every observation,
// normalised for oracle O1. Assertions inside a scenario state what the SPEC
// requires of any conforming server; the transcript is what two targets are
// compared on afterwards (`pnpm parity compare`).

import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { Fixtures } from './lib/fixtures.ts'
import { Normaliser } from './lib/normalise.ts'
import { Target } from './lib/target.ts'
import { HOSTS, targetSpec } from './stack/targets.ts'

/** Response headers that SPEC sections 4.2, 4.3, 4.4 and 6.4 name. Nothing else is compared. */
const COMPARED_HEADERS = [
  'content-type',
  'www-authenticate',
  'retry-after',
  'ratelimit-limit',
  'ratelimit-remaining',
  'ratelimit-policy',
  'access-control-allow-origin',
  'access-control-allow-methods',
  'access-control-allow-headers',
  'access-control-expose-headers',
  'access-control-max-age',
  'atproto-repo-rev',
  'atproto-content-labelers',
  'content-security-policy',
  'x-content-type-options',
  'location',
]

export type Account = {
  name: string
  handle: string
  email: string
  password: string
  did: string
  accessJwt: string
  refreshJwt: string
}

export type Request = {
  method?: string
  /** Defaults to the PDS. Handle hosts and fixtures are reached by name. */
  host?: string
  path: string
  query?: Record<string, string | number | boolean | string[] | undefined>
  json?: unknown
  body?: Uint8Array | string
  contentType?: string
  headers?: Record<string, string>
  /** An account (its access token), a bearer token, or the admin credential. */
  auth?: Account | string | 'admin'
  ip?: string
  /** Send the rate-limit bypass key. Default true. */
  bypass?: boolean
  /** Keep the exchange out of the transcript. For polling and for setup that is not under test. */
  silent?: boolean
  redirect?: 'follow' | 'manual'
}

export type Result = {
  status: number
  headers: Headers
  text: string
  bytes: Uint8Array
  /** The parsed body when the response is JSON, otherwise undefined. */
  json: any
}

type Entry = { step: string } & Record<string, unknown>

export class Scenario {
  readonly name: string
  /** Six hex characters that keep this scenario's handles and addresses apart from every other scenario's. */
  readonly tag: string
  readonly target: Target
  readonly norm = new Normaliser()
  readonly entries: Entry[] = []
  /** The outside services of the stack: AppView, report service, OAuth clients, identity providers. */
  readonly fixtures: Fixtures
  /** The client address the PDS sees for this scenario. */
  readonly ip: string
  private readonly cleanups: (() => void | Promise<void>)[] = []

  constructor(name: string, target: Target) {
    this.name = name
    this.target = target
    this.fixtures = new Fixtures(target)
    const digest = createHash('sha256').update(name).digest()
    this.tag = digest.subarray(0, 3).toString('hex')
    this.ip = `198.51.100.${1 + (digest[3]! % 250)}`
  }

  /** A handle on the hosted domain that no other scenario uses. */
  handle(name: string): string {
    return `${name}-${this.tag}${HOSTS.handleDomain}`
  }

  email(name: string): string {
    return `${name}-${this.tag}@example.com`
  }

  onCleanup(fn: () => void | Promise<void>): void {
    this.cleanups.push(fn)
  }

  async cleanup(): Promise<void> {
    for (const fn of this.cleanups.reverse()) await fn()
  }

  /** Records an observation that is not an HTTP exchange. */
  note(step: string, value: unknown): void {
    this.entries.push({ step, note: this.norm.value(value) })
  }

  /**
   * Records a value that must be byte-identical on every target, without
   * aliasing. An MST root over the same records is one (SPEC 6.6).
   */
  exact(step: string, value: unknown): void {
    this.entries.push({ step, exact: value })
  }

  async http(step: string, request: Request): Promise<Result> {
    const method = request.method ?? (request.json !== undefined || request.body !== undefined ? 'POST' : 'GET')
    const url = new URL(`https://${request.host ?? HOSTS.pds}${request.path}`)
    for (const [key, value] of Object.entries(request.query ?? {})) {
      if (value === undefined) continue
      for (const item of Array.isArray(value) ? value : [value]) url.searchParams.append(key, String(item))
    }
    const headers: Record<string, string> = { ...request.headers }
    let body: Uint8Array | string | undefined = request.body
    if (request.json !== undefined) {
      body = JSON.stringify(request.json)
      headers['content-type'] ??= 'application/json'
    } else if (request.contentType) {
      headers['content-type'] = request.contentType
    }
    if (request.auth === 'admin') headers.authorization = this.target.adminAuth()
    else if (typeof request.auth === 'string') headers.authorization = `Bearer ${request.auth}`
    else if (request.auth) headers.authorization = `Bearer ${request.auth.accessJwt}`

    const fetch = this.target.fetch({ ip: request.ip ?? this.ip, bypassRateLimit: request.bypass })
    const response = await fetch(url, { method, headers, body, redirect: request.redirect ?? 'manual' })
    const bytes = new Uint8Array(await response.arrayBuffer())
    const type = response.headers.get('content-type') ?? ''
    const text = /json|text|html|xml|javascript/.test(type) || bytes.byteLength === 0 ? Buffer.from(bytes).toString('utf8') : ''
    let json: any
    if (type.includes('json') && text) {
      try {
        json = JSON.parse(text)
      } catch {
        json = undefined
      }
    }
    const result: Result = { status: response.status, headers: response.headers, text, bytes, json }
    if (!request.silent) this.record(step, method, url, result)
    return result
  }

  /**
   * Adds an exchange to the transcript. `http` calls this itself; a caller
   * that sent the request silently, to decide first whether it counts, calls
   * it for the exchange that did.
   */
  record(step: string, method: string, url: URL | string, result: Result): void {
    const target = typeof url === 'string' ? url : decodeURIComponent(url.host + url.pathname + url.search)
    const compared: Record<string, string> = {}
    for (const name of COMPARED_HEADERS) {
      const value = result.headers.get(name)
      if (value !== null) compared[name] = this.norm.text(value)
    }
    // The reset time is a clock reading; only its presence is comparable.
    if (result.headers.has('ratelimit-reset')) compared['ratelimit-reset'] = '<time>'
    if (result.headers.has('dpop-nonce')) compared['dpop-nonce'] = '<nonce>'
    // Milliseconds between the AppView's revision and the local one: a clock reading.
    if (result.headers.has('atproto-upstream-lag')) compared['atproto-upstream-lag'] = '<ms>'
    const type = result.headers.get('content-type') ?? ''
    this.entries.push({
      step,
      // Decoded, so that an identifier in the query is aliased like any other.
      request: `${method} ${this.norm.text(target)}`,
      status: result.status,
      headers: compared,
      body: this.recordedBody(type, result.text, result.json, result.bytes),
    })
  }

  private recordedBody(type: string, text: string, json: unknown, bytes: Uint8Array): unknown {
    if (json !== undefined) return this.norm.value(json)
    if (bytes.byteLength === 0) return ''
    // HTML pages are outside C6: the Candidate renders its own.
    if (type.includes('html')) return '<html>'
    if (text) return this.norm.text(text)
    return `<bytes:${bytes.byteLength}>`
  }

  /** `GET /xrpc/<nsid>`. */
  query(step: string, nsid: string, params: Request['query'] = {}, extra: Partial<Request> = {}): Promise<Result> {
    return this.http(step, { path: `/xrpc/${nsid}`, query: params, ...extra })
  }

  /** `POST /xrpc/<nsid>` with a JSON body. */
  procedure(step: string, nsid: string, input: unknown = undefined, extra: Partial<Request> = {}): Promise<Result> {
    return this.http(step, { method: 'POST', path: `/xrpc/${nsid}`, json: input, ...extra })
  }

  /** Creates an account through the legacy endpoint and records the exchange. */
  async createAccount(name: string, input: Record<string, unknown> = {}, extra: Partial<Request> = {}): Promise<Account> {
    const account = { name, handle: this.handle(name), email: this.email(name), password: `${name}-password-${this.tag}` }
    const result = await this.procedure(
      `create account ${name}`,
      'com.atproto.server.createAccount',
      { handle: account.handle, email: account.email, password: account.password, ...input },
      extra,
    )
    assert.equal(result.status, 200, `createAccount ${name}: ${result.text}`)
    return { ...account, did: result.json.did, accessJwt: result.json.accessJwt, refreshJwt: result.json.refreshJwt }
  }

  /** Waits until `probe` returns a value, polling quietly. */
  async eventually<T>(what: string, probe: () => Promise<T | undefined | false>, timeoutMs = 15_000): Promise<T> {
    const deadline = Date.now() + timeoutMs
    for (;;) {
      const value = await probe()
      if (value) return value
      if (Date.now() > deadline) throw new Error(`Timed out waiting for ${what}`)
      await new Promise((resolve) => setTimeout(resolve, 150))
    }
  }
}

/**
 * Asserts that the server refused a request. Where the SPEC names the error
 * the scenario passes it; where it does not, the exact status and name are
 * whatever the Reference answers, and the transcript holds other targets to it.
 */
export function refused(result: Result, error?: string): void {
  assert.ok(result.status >= 400 && result.status < 600, `expected a refusal, got ${result.status}: ${result.text.slice(0, 200)}`)
  if (error !== undefined) assert.equal(result.json?.error, error)
}

export type ScenarioOptions = {
  /** The scenario exercises LinkJar behaviour (SPEC 13). It is skipped, and recorded as skipped, on a stock target. */
  linkjar?: boolean
  timeoutMs?: number
}

/** Declares a scenario. One call per file; the file name is the scenario name. */
export function scenario(name: string, options: ScenarioOptions, run: (s: Scenario) => Promise<void>): void {
  test(name, { timeout: options.timeoutMs ?? 120_000 }, async (t) => {
    const target = Target.fromEnv()
    const s = new Scenario(name, target)
    const file = join(targetSpec(target.name).runDir, 'transcripts', target.state.profile, `${name}.json`)
    mkdirSync(dirname(file), { recursive: true })
    const write = (outcome: string) =>
      writeFileSync(
        file,
        JSON.stringify({ scenario: name, profile: target.state.profile, outcome, entries: s.entries }, null, 2) + '\n',
      )
    if (options.linkjar && !target.linkjar) {
      write('skipped: LinkJar behaviour, not present in a stock build')
      await target.close()
      t.skip('LinkJar behaviour, not present in a stock build')
      return
    }
    try {
      await run(s)
      write('passed')
    } catch (error) {
      write(`failed: ${error instanceof Error ? error.message : String(error)}`)
      throw error
    } finally {
      await s.cleanup()
      await target.close()
    }
  })
}
