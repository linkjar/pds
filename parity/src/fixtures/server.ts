// The outside world of a stack, served from the harness process. The edge
// forwards every name that is not the PDS, the PLC directory, the relay or
// the mail catcher to this server, which decides by the Host header.
//
// It records each request so that a scenario can assert what the PDS sent,
// and it serves documents and canned XRPC answers that a scenario registers
// through the control API under `/__control/`, reachable from the host only.

import { createServer } from 'node:http'
import type { IncomingMessage, Server, ServerResponse } from 'node:http'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { HOSTS, REPO_DIR } from '../stack/targets.ts'
import { IdentityProviders } from './identity-providers.ts'

export type RecordedRequest = {
  index: number
  host: string
  method: string
  path: string
  query: Record<string, string>
  headers: Record<string, string>
  body: string
}

export type Document = { host: string; path: string; status?: number; contentType?: string; body: string; headers?: Record<string, string> }
export type CannedXrpc = { host: string; nsid: string; status?: number; body?: unknown; headers?: Record<string, string> }

const LOGO = readFileSync(join(REPO_DIR, 'legacy/tests/fixtures/linkjar-128.png'))

/** DID documents of the two services the PDS forwards to. */
function didDocument(host: string): unknown {
  const did = `did:web:${host}`
  const service =
    host === HOSTS.appview
      ? [
          { id: '#bsky_appview', type: 'BskyAppView', serviceEndpoint: `https://${host}` },
          { id: '#bsky_notif', type: 'BskyNotificationService', serviceEndpoint: `https://${host}` },
        ]
      : [{ id: '#atproto_labeler', type: 'AtprotoLabeler', serviceEndpoint: `https://${host}` }]
  return { '@context': ['https://www.w3.org/ns/did/v1'], id: did, service }
}

export class FixtureServer {
  readonly requests: RecordedRequest[] = []
  private readonly documents = new Map<string, Document>()
  private readonly canned = new Map<string, CannedXrpc>()
  readonly identityProviders = new IdentityProviders()
  /** What `api.hcaptcha.com/siteverify` answers. */
  hcaptchaSuccess = true
  readonly port: number
  private server: Server | undefined

  constructor(port: number) {
    this.port = port
  }

  async start(): Promise<void> {
    await this.identityProviders.init()
    this.server = createServer((req, res) => {
      this.handle(req, res).catch((error) => {
        res.writeHead(500, { 'content-type': 'text/plain' }).end(`fixture error: ${String(error)}`)
      })
    })
    // Docker Desktop reaches the host on loopback. On Linux the containers
    // arrive on the bridge gateway address, so the server must listen there too.
    const bind = process.env.PARITY_FIXTURE_BIND ?? (process.platform === 'linux' ? '0.0.0.0' : '127.0.0.1')
    await new Promise<void>((resolve, reject) => {
      this.server!.once('error', reject)
      this.server!.listen(this.port, bind, resolve)
    })
  }

  async stop(): Promise<void> {
    await new Promise<void>((resolve) => (this.server ? this.server.close(() => resolve()) : resolve()))
  }

  private async handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const chunks: Buffer[] = []
    for await (const chunk of req) chunks.push(chunk as Buffer)
    const body = Buffer.concat(chunks).toString('utf8')
    const host = (req.headers.host ?? '').toLowerCase()
    const url = new URL(req.url ?? '/', `https://${host}`)
    const direct = /^(127\.0\.0\.1|localhost)(:\d+)?$/.test(host)

    if (url.pathname.startsWith('/__control/')) {
      // The control API never answers a request that came through the edge.
      if (!direct) return void res.writeHead(404).end()
      return this.control(req.method ?? 'GET', url, body, res)
    }

    const headers: Record<string, string> = {}
    for (const [name, value] of Object.entries(req.headers)) if (typeof value === 'string') headers[name] = value
    const recorded: RecordedRequest = {
      index: this.requests.length,
      host,
      method: req.method ?? 'GET',
      path: url.pathname,
      query: Object.fromEntries(url.searchParams),
      headers,
      body,
    }
    this.requests.push(recorded)

    const send = (status: number, payload: unknown, extra: Record<string, string> = {}) => {
      const isText = typeof payload === 'string' || Buffer.isBuffer(payload)
      res.writeHead(status, { 'content-type': isText ? 'text/plain' : 'application/json', ...extra })
      res.end(isText ? (payload as string | Buffer) : JSON.stringify(payload))
    }

    const document = this.documents.get(`${host}${url.pathname}`)
    if (document) {
      return send(document.status ?? 200, document.body, {
        'content-type': document.contentType ?? 'application/json',
        ...document.headers,
      })
    }
    if (host === HOSTS.web && url.pathname === '/logo/linkjar-128.png') return send(200, LOGO, { 'content-type': 'image/png' })
    if ((host === HOSTS.appview || host === HOSTS.mod) && url.pathname === '/.well-known/did.json') {
      return send(200, didDocument(host))
    }
    if (url.pathname.startsWith('/xrpc/')) {
      const nsid = url.pathname.slice('/xrpc/'.length)
      const canned = this.canned.get(`${host} ${nsid}`)
      if (canned) return send(canned.status ?? 200, canned.body ?? {}, canned.headers)
      if (host === HOSTS.appview || host === HOSTS.mod) return send(200, {})
    }
    if (host === 'api.hcaptcha.com' && url.pathname === '/siteverify') {
      return send(200, { success: this.hcaptchaSuccess, hostname: HOSTS.pds, challenge_ts: new Date().toISOString() })
    }
    const identity = await this.identityProviders.handle(recorded)
    if (identity) return send(identity.status, identity.body, identity.headers)
    // An OAuth client's redirect target. The browser driver reads the URL; the page is never used.
    if (url.pathname.endsWith('/callback')) return send(200, 'callback received', { 'content-type': 'text/plain' })
    send(404, { error: 'NotFound', message: `No fixture for ${host}${url.pathname}` })
  }

  private control(method: string, url: URL, body: string, res: ServerResponse): void {
    const json = (status: number, payload: unknown): void => {
      res.writeHead(status, { 'content-type': 'application/json' }).end(JSON.stringify(payload))
    }
    const input = body ? JSON.parse(body) : undefined
    const route = `${method} ${url.pathname.slice('/__control'.length)}`
    switch (route) {
      case 'GET /health':
        return json(200, { ok: true })
      case 'GET /requests': {
        const host = url.searchParams.get('host')
        const since = Number(url.searchParams.get('since') ?? 0)
        return json(200, this.requests.filter((r) => r.index >= since && (!host || r.host === host)))
      }
      case 'GET /cursor':
        return json(200, { next: this.requests.length })
      case 'PUT /document': {
        const document = input as Document
        this.documents.set(`${document.host}${document.path}`, document)
        return json(200, {})
      }
      case 'DELETE /document':
        this.documents.delete(`${input.host}${input.path}`)
        return json(200, {})
      case 'PUT /xrpc': {
        const canned = input as CannedXrpc
        this.canned.set(`${canned.host} ${canned.nsid}`, canned)
        return json(200, {})
      }
      case 'PUT /hcaptcha':
        this.hcaptchaSuccess = Boolean(input.success)
        return json(200, {})
      case 'PUT /identity':
        this.identityProviders.setNext(input)
        return json(200, {})
      default:
        return json(404, { error: `no control route ${route}` })
    }
  }
}
