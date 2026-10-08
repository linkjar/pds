// The scenario's view of the fixture server (src/fixtures/server.ts).

import type { CannedXrpc, Document, RecordedRequest } from '../fixtures/server.ts'
import type { NextIdentity } from '../fixtures/identity-providers.ts'
import type { Target } from './target.ts'

export class Fixtures {
  private readonly base: string

  constructor(target: Target) {
    this.base = `http://127.0.0.1:${target.state.fixturePort}/__control`
  }

  private async call(method: string, path: string, body?: unknown): Promise<any> {
    const response = await fetch(`${this.base}${path}`, {
      method,
      headers: body === undefined ? undefined : { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
    if (!response.ok) throw new Error(`fixture control ${method} ${path}: ${response.status} ${await response.text()}`)
    return response.json()
  }

  /** A position in the request log. Requests recorded later have an index at or above it. */
  async cursor(): Promise<number> {
    return (await this.call('GET', '/cursor')).next
  }

  /** Requests a fixture host received since `since`. */
  requests(host: string, since = 0): Promise<RecordedRequest[]> {
    return this.call('GET', `/requests?host=${encodeURIComponent(host)}&since=${since}`)
  }

  /** Serves a document, for example OAuth client metadata, at `https://<host><path>`. */
  serve(document: Document): Promise<void> {
    return this.call('PUT', '/document', document)
  }

  remove(host: string, path: string): Promise<void> {
    return this.call('DELETE', '/document', { host, path })
  }

  /** Sets what a fixture service answers for one XRPC method. */
  xrpc(canned: CannedXrpc): Promise<void> {
    return this.call('PUT', '/xrpc', canned)
  }

  hcaptcha(success: boolean): Promise<void> {
    return this.call('PUT', '/hcaptcha', { success })
  }

  /** Chooses who the next external sign-in with this provider is. */
  identity(next: NextIdentity): Promise<void> {
    return this.call('PUT', '/identity', next)
  }
}
