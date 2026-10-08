// A running stack as the scenarios see it. Every name resolves to the stack's
// TLS edge on the host, with the stack's CA as the only trust anchor, so the
// SDK talks to `https://pds.linkjar.social` exactly as a real client would.

import { readFileSync } from 'node:fs'
import { AtpAgent } from '@atproto/api'
import { Agent, buildConnector, fetch as undiciFetch } from 'undici'
import WebSocket from 'ws'
import { bootPds, execPds, readState, restartPds, sqlite } from '../stack/compose.ts'
import type { Database, StackState } from '../stack/compose.ts'
import { HOSTS, isTargetName } from '../stack/targets.ts'
import type { TargetName } from '../stack/targets.ts'

export type FetchLike = typeof globalThis.fetch

export type ClientOptions = {
  /** The address the PDS sees as the client. Scenarios that count limiter points pick their own. */
  ip?: string
  /** Send the rate-limit bypass key. On by default so that unrelated scenarios never share a bucket. */
  bypassRateLimit?: boolean
  headers?: Record<string, string>
}

export class Target {
  readonly name: TargetName
  readonly state: StackState
  readonly pdsUrl = `https://${HOSTS.pds}`
  readonly plcUrl = `https://${HOSTS.plc}`
  readonly relayUrl = `https://${HOSTS.relay}`
  readonly mailUrl = `https://${HOSTS.mail}`
  readonly serviceDid = `did:web:${HOSTS.pds}`
  readonly ca: string
  private readonly dispatcher: Agent

  constructor(name: TargetName) {
    this.name = name
    this.state = readState(name)
    this.ca = readFileSync(this.state.caFile, 'utf8')
    const connect = buildConnector({ ca: this.ca })
    const port = String(this.state.edgePort)
    this.dispatcher = new Agent({
      connect: (opts, callback) =>
        connect({ ...opts, hostname: '127.0.0.1', port, servername: opts.servername ?? opts.hostname }, callback),
    })
  }

  /** The target named by PARITY_TARGET, which the runner sets for every scenario process. */
  static fromEnv(): Target {
    const name = process.env.PARITY_TARGET ?? ''
    if (!isTargetName(name)) throw new Error('PARITY_TARGET is not set. Run scenarios through: pnpm parity run <target>')
    return new Target(name)
  }

  get linkjar(): boolean {
    return this.state.linkjar
  }

  /** A fetch that reaches the stack. Only https URLs are routable. */
  fetch(opts: ClientOptions = {}): FetchLike {
    const extra: Record<string, string> = { ...opts.headers }
    if (opts.ip) extra['x-forwarded-for'] = opts.ip
    if (opts.bypassRateLimit !== false && this.state.secrets.rateLimitBypassKey) {
      extra['x-ratelimit-bypass'] = this.state.secrets.rateLimitBypassKey
    }
    const dispatcher = this.dispatcher
    return (async (input: Parameters<FetchLike>[0], init?: RequestInit) => {
      const request = new Request(input, init)
      const headers = new Headers(request.headers)
      for (const [key, value] of Object.entries(extra)) if (!headers.has(key)) headers.set(key, value)
      const hasBody = request.method !== 'GET' && request.method !== 'HEAD'
      const response = await undiciFetch(request.url, {
        method: request.method,
        headers: [...headers.entries()],
        body: hasBody ? Buffer.from(await request.arrayBuffer()) : undefined,
        redirect: request.redirect,
        dispatcher,
      })
      return response as unknown as Response
    }) as FetchLike
  }

  /** An SDK agent for the PDS. */
  agent(opts: ClientOptions = {}): AtpAgent {
    return new AtpAgent({ service: this.pdsUrl, fetch: this.fetch(opts) })
  }

  adminAuth(): string {
    return 'Basic ' + Buffer.from(`admin:${this.state.secrets.adminPassword}`).toString('base64')
  }

  /** Opens a WebSocket to a host of the stack, for example the PDS or the relay firehose. */
  socket(host: string, pathAndQuery: string, headers: Record<string, string> = {}): WebSocket {
    // The socket connects to the edge by address; the TLS server name and the Host header carry the name.
    const options: WebSocket.ClientOptions & { servername: string } = {
      ca: this.ca,
      servername: host,
      headers: { host, ...headers },
    }
    return new WebSocket(`wss://127.0.0.1:${this.state.edgePort}${pathAndQuery}`, options)
  }

  /** One SQL statement against the data directory. See compose/sqlite-tool.cjs for why this exists. */
  async sqlite(database: Database, sql: string, params: unknown[] = []): Promise<Record<string, unknown>[]> {
    return sqlite(this.name, database, sql, params)
  }

  /** Restarts the PDS and waits until it is healthy. */
  async restart(): Promise<void> {
    restartPds(this.name)
  }

  /** Starts a throwaway PDS with changed variables and reports whether it stays up. */
  boot(env: Record<string, string>): { started: boolean; output: string } {
    return bootPds(this.name, env)
  }

  /** Runs a command in the PDS container. Used to look at files on disk, never to change server state. */
  exec(command: string[]): { status: number; stdout: string; stderr: string } {
    return execPds(this.name, command)
  }

  async close(): Promise<void> {
    await this.dispatcher.close()
  }
}
