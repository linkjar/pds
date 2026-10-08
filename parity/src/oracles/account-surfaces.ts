// One record of everything an account's status changes: what the owner can
// still do, and what the rest of the network can still read. Scenarios take
// it before and after a status change and compare.

import type { Account, Scenario } from '../scenario.ts'

export const NOTE = 'com.example.parity.note'

/** Every surface an account status changes, as one comparable record. */
export async function surfaces(s: Scenario, label: string, account: Account, blobCid: string): Promise<Record<string, unknown>> {
  const brief = (result: { status: number; json: any }) => (result.status === 200 ? 200 : `${result.status} ${result.json?.error ?? ''}`.trim())
  const out: Record<string, unknown> = {}
  const anonymous = { silent: true } as const
  const authed = { auth: account, silent: true } as const
  out.getSession = brief(await s.query('x', 'com.atproto.server.getSession', {}, authed))
  out.describeRepo = brief(await s.query('x', 'com.atproto.repo.describeRepo', { repo: account.did }, anonymous))
  out.getRecord = brief(await s.query('x', 'com.atproto.repo.getRecord', { repo: account.did, collection: NOTE, rkey: 'one' }, anonymous))
  out.listRecords = brief(await s.query('x', 'com.atproto.repo.listRecords', { repo: account.did, collection: NOTE }, anonymous))
  out.getRepo = brief(await s.query('x', 'com.atproto.sync.getRepo', { did: account.did }, anonymous))
  out.getRepoAsOwner = brief(await s.query('x', 'com.atproto.sync.getRepo', { did: account.did }, authed))
  out.getLatestCommit = brief(await s.query('x', 'com.atproto.sync.getLatestCommit', { did: account.did }, anonymous))
  out.syncGetRecord = brief(await s.query('x', 'com.atproto.sync.getRecord', { did: account.did, collection: NOTE, rkey: 'one' }, anonymous))
  out.getBlob = brief(await s.query('x', 'com.atproto.sync.getBlob', { did: account.did, cid: blobCid }, anonymous))
  out.getBlobAsOwner = brief(await s.query('x', 'com.atproto.sync.getBlob', { did: account.did, cid: blobCid }, authed))
  out.listBlobs = brief(await s.query('x', 'com.atproto.sync.listBlobs', { did: account.did }, anonymous))
  out.createRecord = brief(await s.procedure('x', 'com.atproto.repo.createRecord', { repo: account.did, collection: NOTE, rkey: `w-${label}`, record: { $type: NOTE, label } }, authed))
  out.uploadBlob = brief(await s.http('x', { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body: `upload ${label}`, contentType: 'text/plain', ...authed }))
  out.proxy = brief(await s.query('x', 'app.bsky.graph.getFollows', { actor: account.did }, authed))
  out.getServiceAuth = brief(await s.query('x', 'com.atproto.server.getServiceAuth', { aud: 'did:web:appview.test', lxm: 'app.bsky.feed.getTimeline' }, authed))
  out.getPreferences = brief(await s.query('x', 'app.bsky.actor.getPreferences', {}, authed))
  const status = await s.query('x', 'com.atproto.sync.getRepoStatus', { did: account.did }, anonymous)
  out.getRepoStatus = status.status === 200 ? { active: status.json.active, status: status.json.status ?? null } : brief(status)
  const wellKnown = await s.http('x', { host: account.handle, path: '/.well-known/atproto-did', silent: true })
  out.atprotoDid = wellKnown.status
  s.note(`surfaces while ${label}`, out)
  return out
}
