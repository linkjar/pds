// Read requests of a restore run (src/restore.ts). Both servers read the
// same data directory, so a body is recorded twice: normalised, as every
// scenario records it, and as the hash of its bytes, which must be equal.

import { createHash } from 'node:crypto'
import type { Request, Result, Scenario } from '../scenario.ts'

export async function read(s: Scenario, step: string, request: Request): Promise<Result> {
  const result = await s.http(step, request)
  s.exact(`${step}: body`, { bytes: result.bytes.byteLength, sha256: createHash('sha256').update(result.bytes).digest('hex') })
  return result
}

/** `GET /xrpc/<nsid>` with its body hash recorded. */
export function get(s: Scenario, step: string, nsid: string, params: Request['query'] = {}, extra: Partial<Request> = {}): Promise<Result> {
  return read(s, step, { path: `/xrpc/${nsid}`, query: params, ...extra })
}

/** A request whose query is written out, for the cases a parameter map cannot express. */
export function raw(s: Scenario, step: string, pathAndQuery: string, extra: Partial<Request> = {}): Promise<Result> {
  return read(s, step, { path: pathAndQuery, ...extra })
}
