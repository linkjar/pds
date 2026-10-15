// The smoke test of a running PDS (docs/runbooks/test-host.md). It talks to
// one public URL the way a client does, with the official SDK at the pin,
// and needs no stack, fixture or container of the harness.
//
// Without --write it only reads: health, discovery, the OAuth metadata, a
// pushed authorization request of a loopback client and its page, and the
// firehose upgrade. With --write it also creates one account, uses it the
// way the Bluesky app does, and deletes it. The account's DID stays in the
// PLC directory the server names, which is why --write is never the default.

import assert from 'node:assert/strict'
import { createHash, randomBytes, webcrypto } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { AtpAgent } from '@atproto/api'
import { Agent as Dispatcher, buildConnector, request as undiciRequest, fetch as undiciFetch } from 'undici'
import WebSocket from 'ws'
import { decodeFrame } from './lib/firehose.ts'
import type { Frame } from './lib/firehose.ts'

export type SmokeOptions = {
  /** The PDS, for example https://pds.example.com. */
  url: string
  /** Create, use and delete one account. */
  write: boolean
  /** Needed with --write when the server requires invite codes, and to delete the account. Read from PDS_ADMIN_PASSWORD. */
  adminPassword?: string
  /** The handle domain of the account, such as `.example.com`. The first of `availableUserDomains` by default. */
  handleDomain?: string
  email?: string
  /** Keep the account at the end, to sign in to it from an app. */
  keepAccount: boolean
  /** A CA in PEM to trust in place of the system's, such as the CA of a parity stack. */
  caFile?: string
  /** `host:port` to connect to in place of the address of the URL's name, like curl's --connect-to. */
  connectTo?: string
}

type Check = { name: string; ok: boolean; detail: string }

const PNG_1X1 = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
  'base64',
)

class Client {
  readonly url: URL
  readonly ca?: string
  readonly connectTo?: { host: string; port: number }
  private readonly dispatcher: Dispatcher

  constructor(opts: SmokeOptions) {
    this.url = new URL(opts.url)
    this.ca = opts.caFile ? readFileSync(opts.caFile, 'utf8') : undefined
    if (opts.connectTo) {
      const at = opts.connectTo.lastIndexOf(':')
      this.connectTo = { host: opts.connectTo.slice(0, at), port: Number(opts.connectTo.slice(at + 1)) }
    }
    const connect = buildConnector(this.ca ? { ca: this.ca } : {})
    const target = this.connectTo
    this.dispatcher = new Dispatcher({
      connect: (o, callback) =>
        connect(
          target ? { ...o, hostname: target.host, port: String(target.port), servername: o.servername ?? o.hostname } : o,
          callback,
        ),
    })
  }

  fetch: typeof globalThis.fetch = (async (input: Parameters<typeof globalThis.fetch>[0], init?: RequestInit) => {
    const request = new Request(input, init)
    const hasBody = request.method !== 'GET' && request.method !== 'HEAD'
    const response = await undiciFetch(request.url, {
      method: request.method,
      headers: [...request.headers.entries()],
      body: hasBody ? Buffer.from(await request.arrayBuffer()) : undefined,
      redirect: 'manual',
      dispatcher: this.dispatcher,
    })
    return response as unknown as Response
  }) as typeof globalThis.fetch

  /**
   * A GET that sends the headers as given. `fetch` sets `sec-fetch-mode`
   * to `cors` itself, and a page of the authorization server wants the
   * `navigate` of a browser.
   */
  async navigate(url: string, headers: Record<string, string>): Promise<{ status: number; type: string; text: string }> {
    const response = await undiciRequest(url, { method: 'GET', headers, dispatcher: this.dispatcher })
    const type = response.headers['content-type']
    return { status: response.statusCode, type: Array.isArray(type) ? type.join(',') : (type ?? ''), text: await response.body.text() }
  }

  href(path: string): string {
    return new URL(path, this.url).toString()
  }

  agent(): AtpAgent {
    return new AtpAgent({ service: this.url.toString(), fetch: this.fetch })
  }

  async json(path: string, init?: RequestInit): Promise<{ status: number; body: Record<string, unknown>; headers: Headers }> {
    const response = await this.fetch(this.href(path), init)
    const text = await response.text()
    let body: Record<string, unknown> = {}
    try {
      body = text ? JSON.parse(text) : {}
    } catch {
      body = { text: text.slice(0, 200) }
    }
    return { status: response.status, body, headers: response.headers }
  }

  socket(pathAndQuery: string): WebSocket {
    const host = this.url.host
    const scheme = this.url.protocol === 'http:' ? 'ws' : 'wss'
    if (!this.connectTo) return new WebSocket(`${scheme}://${host}${pathAndQuery}`, this.ca ? { ca: this.ca } : {})
    const options: WebSocket.ClientOptions & { servername: string } = {
      ca: this.ca,
      servername: this.url.hostname,
      headers: { host },
    }
    return new WebSocket(`${scheme}://${this.connectTo.host}:${this.connectTo.port}${pathAndQuery}`, options)
  }

  async close(): Promise<void> {
    await this.dispatcher.close()
  }
}

/** A DPoP key and proof (RFC 9449), for the pushed authorization request. */
async function dpopProof(method: string, url: string, nonce?: string): Promise<string> {
  const pair = await webcrypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify'])
  const { kty, crv, x, y } = (await webcrypto.subtle.exportKey('jwk', pair.publicKey)) as Record<string, string>
  const b64 = (value: unknown) => Buffer.from(JSON.stringify(value)).toString('base64url')
  const claims: Record<string, unknown> = { jti: randomBytes(12).toString('hex'), htm: method, htu: url, iat: Math.floor(Date.now() / 1000) }
  if (nonce) claims.nonce = nonce
  const input = `${b64({ typ: 'dpop+jwt', alg: 'ES256', jwk: { kty, crv, x, y } })}.${b64(claims)}`
  const signature = await webcrypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, pair.privateKey, Buffer.from(input))
  return `${input}.${Buffer.from(signature).toString('base64url')}`
}

/** Opens `subscribeRepos` and collects frames until `close`. */
function firehose(client: Client, cursor?: number) {
  const frames: Frame[] = []
  const socket = client.socket(`/xrpc/com.atproto.sync.subscribeRepos${cursor === undefined ? '' : `?cursor=${cursor}`}`)
  const opened = new Promise<void>((resolve, reject) => {
    socket.once('open', () => resolve())
    socket.once('unexpected-response', (_request, response) => reject(new Error(`upgrade answered ${response.statusCode}`)))
    socket.once('error', reject)
  })
  socket.on('message', (data: Buffer) => frames.push(decodeFrame(new Uint8Array(data))))
  return {
    opened,
    async waitFor(match: (frame: Frame) => boolean, timeoutMs = 15_000): Promise<Frame> {
      const until = Date.now() + timeoutMs
      while (Date.now() < until) {
        const found = frames.find(match)
        if (found) return found
        await new Promise((resolve) => setTimeout(resolve, 100))
      }
      throw new Error(`no matching frame in ${timeoutMs} ms; saw ${frames.length}`)
    },
    close: () => socket.terminate(),
  }
}

export async function smoke(opts: SmokeOptions): Promise<number> {
  const client = new Client(opts)
  const checks: Check[] = []
  const check = async <T>(name: string, fn: () => Promise<T>, detail?: (value: T) => string): Promise<T | undefined> => {
    try {
      const value = await fn()
      const line = { name, ok: true, detail: detail ? detail(value) : '' }
      checks.push(line)
      console.log(`ok    ${name}${line.detail ? `  ${line.detail}` : ''}`)
      return value
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      checks.push({ name, ok: false, detail: message })
      console.log(`FAIL  ${name}  ${message}`)
      return undefined
    }
  }
  const origin = client.url.origin

  // Reads.
  await check(
    'health',
    async () => {
      const r = await client.json('/xrpc/_health')
      assert.equal(r.status, 200, `status ${r.status}`)
      return r.body
    },
    (body) => (body.version ? `version ${body.version}` : ''),
  )
  const server = await check(
    'describeServer',
    async () => {
      const r = await client.json('/xrpc/com.atproto.server.describeServer')
      assert.equal(r.status, 200, `status ${r.status}`)
      assert.ok(typeof r.body.did === 'string', 'no did')
      assert.ok(Array.isArray(r.body.availableUserDomains), 'no availableUserDomains')
      return r.body as { did: string; availableUserDomains: string[]; inviteCodeRequired?: boolean }
    },
    (body) => `${body.did}, domains ${body.availableUserDomains.join(' ')}, invites ${body.inviteCodeRequired ? 'required' : 'not required'}`,
  )
  await check('oauth-protected-resource', async () => {
    const r = await client.json('/.well-known/oauth-protected-resource')
    assert.equal(r.status, 200, `status ${r.status}`)
    assert.equal(r.body.resource, origin)
    assert.ok(Array.isArray(r.body.authorization_servers) && r.body.authorization_servers.length > 0, 'no authorization_servers')
  })
  const metadata = await check(
    'oauth-authorization-server',
    async () => {
      const r = await client.json('/.well-known/oauth-authorization-server')
      assert.equal(r.status, 200, `status ${r.status}`)
      assert.equal(r.body.issuer, origin, `issuer ${r.body.issuer}`)
      for (const field of ['authorization_endpoint', 'token_endpoint', 'pushed_authorization_request_endpoint']) {
        assert.ok(typeof r.body[field] === 'string', `no ${field}`)
      }
      return r.body as { pushed_authorization_request_endpoint: string; authorization_endpoint: string }
    },
    (body) => `PAR at ${new URL(body.pushed_authorization_request_endpoint).pathname}`,
  )
  if (metadata) {
    // The loopback client of the atproto OAuth profile needs no metadata
    // document, so the server can be tried without publishing a client.
    const clientId = 'http://localhost'
    const redirectUri = 'http://127.0.0.1/'
    const requestUri = await check('oauth pushed authorization request', async () => {
      const par = metadata.pushed_authorization_request_endpoint
      const verifier = randomBytes(32).toString('base64url')
      const form = new URLSearchParams({
        client_id: clientId,
        redirect_uri: redirectUri,
        response_type: 'code',
        scope: 'atproto',
        state: randomBytes(8).toString('hex'),
        code_challenge: createHash('sha256').update(verifier).digest('base64url'),
        code_challenge_method: 'S256',
      }).toString()
      const send = async (nonce?: string) =>
        client.fetch(par, {
          method: 'POST',
          headers: { 'content-type': 'application/x-www-form-urlencoded', dpop: await dpopProof('POST', par, nonce) },
          body: form,
        })
      let response = await send()
      if (response.status === 400) {
        const nonce = response.headers.get('dpop-nonce')
        const body = (await response.json()) as { error?: string }
        if (body.error === 'use_dpop_nonce' && nonce) response = await send(nonce)
        else throw new Error(`status 400: ${JSON.stringify(body)}`)
      }
      const body = (await response.json()) as { request_uri?: string }
      assert.equal(response.status, 201, `status ${response.status}: ${JSON.stringify(body)}`)
      assert.ok(body.request_uri, 'no request_uri')
      return body.request_uri
    })
    if (requestUri) {
      await check('oauth sign-in page', async () => {
        const page = new URL(metadata.authorization_endpoint)
        page.searchParams.set('client_id', clientId)
        page.searchParams.set('request_uri', requestUri)
        // The headers a browser sends on a navigation; the Reference refuses the page without them.
        const { status, type, text } = await client.navigate(page.toString(), {
          accept: 'text/html',
          'sec-fetch-mode': 'navigate',
          'sec-fetch-dest': 'document',
          'sec-fetch-site': 'cross-site',
        })
        const reason = text.match(/error_description[^,}]*/)?.[0] ?? ''
        assert.equal(status, 200, `status ${status} ${reason}`)
        assert.match(type, /text\/html/)
        assert.ok(text.includes('<form') || text.includes('<html'), 'no page')
      })
    }
  }
  await check('listRepos', async () => {
    const r = await client.json('/xrpc/com.atproto.sync.listRepos?limit=1')
    assert.equal(r.status, 200, `status ${r.status}`)
  })
  await check('subscribeRepos upgrade', async () => {
    const stream = firehose(client)
    try {
      await stream.opened
    } finally {
      stream.close()
    }
  })

  if (opts.write && server) await writeJourney(client, opts, server, check)

  await client.close()
  const failed = checks.filter((c) => !c.ok)
  console.log(`\n${checks.length - failed.length} of ${checks.length} checks passed against ${origin}`)
  return failed.length === 0 ? 0 : 1
}

type CheckFn = <T>(name: string, fn: () => Promise<T>, detail?: (value: T) => string) => Promise<T | undefined>

async function writeJourney(
  client: Client,
  opts: SmokeOptions,
  server: { did: string; availableUserDomains: string[]; inviteCodeRequired?: boolean },
  check: CheckFn,
): Promise<void> {
  const admin = opts.adminPassword ? 'Basic ' + Buffer.from(`admin:${opts.adminPassword}`).toString('base64') : undefined
  const tag = randomBytes(3).toString('hex')
  const domain = opts.handleDomain ?? server.availableUserDomains[0] ?? '.test'
  const handle = `smoke-${tag}${domain.startsWith('.') ? domain : `.${domain}`}`
  const email = opts.email ?? `smoke-${tag}@example.com`
  const password = randomBytes(18).toString('base64url')

  let inviteCode: string | undefined
  if (server.inviteCodeRequired) {
    if (!admin) {
      await check('createInviteCode', async () => {
        throw new Error('the server requires invite codes: pass --admin-password')
      })
      return
    }
    inviteCode = await check('createInviteCode', async () => {
      const r = await client.json('/xrpc/com.atproto.server.createInviteCode', {
        method: 'POST',
        headers: { authorization: admin, 'content-type': 'application/json' },
        body: JSON.stringify({ useCount: 1 }),
      })
      assert.equal(r.status, 200, `status ${r.status}: ${JSON.stringify(r.body)}`)
      return String(r.body.code)
    })
    if (!inviteCode) return
  }

  // The firehose is open before the account exists, so its events are seen.
  const stream = firehose(client)
  await stream.opened.catch(() => undefined)

  const agent = client.agent()
  const did = await check(
    'createAccount',
    async () => {
      const r = await agent.createAccount({ handle, email, password, inviteCode })
      return r.data.did
    },
    (value) => `${handle} is ${value}`,
  )
  if (!did) {
    stream.close()
    return
  }

  try {
    await check('getSession', async () => {
      const r = await agent.com.atproto.server.getSession()
      assert.equal(r.data.did, did)
      assert.equal(r.data.handle, handle)
    })
    await check('resolveHandle', async () => {
      const r = await agent.com.atproto.identity.resolveHandle({ handle })
      assert.equal(r.data.did, did)
    })
    const blob = await check('uploadBlob', async () => {
      const r = await agent.com.atproto.repo.uploadBlob(PNG_1X1, { encoding: 'image/png' })
      return r.data.blob
    })
    await check('putRecord profile', async () => {
      await agent.com.atproto.repo.putRecord({
        repo: did,
        collection: 'app.bsky.actor.profile',
        rkey: 'self',
        record: { $type: 'app.bsky.actor.profile', displayName: 'Smoke test', ...(blob ? { avatar: blob } : {}) },
      })
    })
    const post = await check(
      'createRecord post',
      async () => {
        const r = await agent.com.atproto.repo.createRecord({
          repo: did,
          collection: 'app.bsky.feed.post',
          record: { $type: 'app.bsky.feed.post', text: `Smoke test ${tag}`, createdAt: new Date().toISOString() },
        })
        return r.data
      },
      (value) => value.uri,
    )
    if (post) {
      await check('getRecord', async () => {
        const rkey = post.uri.split('/').at(-1)!
        const r = await agent.com.atproto.repo.getRecord({ repo: did, collection: 'app.bsky.feed.post', rkey })
        assert.equal(r.data.cid, post.cid)
      })
      await check('firehose commit', async () => {
        const isPost = (op: { path?: string }) => op.path?.startsWith('app.bsky.feed.post/') ?? false
        await stream.waitFor((f) => f.kind === 'event' && f.type === '#commit' && f.body.repo === did && ((f.body.ops ?? []) as { path?: string }[]).some(isPost))
      })
    }
    await check('listRecords', async () => {
      const r = await agent.com.atproto.repo.listRecords({ repo: did, collection: 'app.bsky.feed.post' })
      assert.equal(r.data.records.length, 1)
    })
    if (blob) {
      await check('getBlob', async () => {
        const cid = blob.ref.toString()
        const response = await client.fetch(client.href(`/xrpc/com.atproto.sync.getBlob?did=${did}&cid=${cid}`))
        assert.equal(response.status, 200, `status ${response.status}`)
        assert.deepEqual(Buffer.from(await response.arrayBuffer()), PNG_1X1)
      })
    }
    await check('getRepo', async () => {
      const response = await client.fetch(client.href(`/xrpc/com.atproto.sync.getRepo?did=${did}`))
      assert.equal(response.status, 200, `status ${response.status}`)
      assert.match(response.headers.get('content-type') ?? '', /application\/vnd\.ipld\.car/)
      return (await response.arrayBuffer()).byteLength
    }, (bytes) => `${bytes} bytes`)
    await check('putPreferences and getPreferences', async () => {
      const preference = { $type: 'app.bsky.actor.defs#adultContentPref', enabled: false }
      await agent.app.bsky.actor.putPreferences({ preferences: [preference] })
      const r = await agent.app.bsky.actor.getPreferences()
      assert.deepEqual(r.data.preferences, [preference])
    })
    await check('app password', async () => {
      const created = await agent.com.atproto.server.createAppPassword({ name: `smoke-${tag}` })
      const other = client.agent()
      await other.login({ identifier: handle, password: created.data.password })
      assert.equal(other.session?.did, did)
      await agent.com.atproto.server.revokeAppPassword({ name: `smoke-${tag}` })
    })
    await check('refreshSession', async () => {
      const r = await client.json('/xrpc/com.atproto.server.refreshSession', {
        method: 'POST',
        headers: { authorization: `Bearer ${agent.session!.refreshJwt}` },
      })
      assert.equal(r.status, 200, `status ${r.status}: ${JSON.stringify(r.body)}`)
      assert.equal(r.body.did, did)
    })
    await check('sign in again', async () => {
      const other = client.agent()
      await other.login({ identifier: email, password })
      assert.equal(other.session?.did, did)
    })
  } finally {
    stream.close()
  }

  if (opts.keepAccount) {
    // The person who asked to keep the account signs in to it from an app.
    console.log(`\nkept ${handle} (${did}) with the password ${password}`)
    console.log('Delete it with com.atproto.admin.deleteAccount when the test is done.')
    return
  }
  if (!admin) {
    await check('deleteAccount', async () => {
      throw new Error(`pass --admin-password to delete ${did}`)
    })
    return
  }
  await check('deleteAccount', async () => {
    const r = await client.json('/xrpc/com.atproto.admin.deleteAccount', {
      method: 'POST',
      headers: { authorization: admin, 'content-type': 'application/json' },
      body: JSON.stringify({ did }),
    })
    assert.equal(r.status, 200, `status ${r.status}: ${JSON.stringify(r.body)}`)
  })
}
