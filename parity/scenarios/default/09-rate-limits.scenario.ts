// Rate limits: SPEC 4.3 and VP-12. Every limiter of the Reference is read
// off the response headers of one request, and two are driven to 429.
//
// Requests here do not send the bypass key, and each group uses its own
// client address so that no bucket is shared with another scenario.

import assert from 'node:assert/strict'
import { refused, scenario } from '../../src/scenario.ts'
import type { Request, Result, Scenario } from '../../src/scenario.ts'

const POST = 'app.bsky.feed.post'

/** The limiter a response reports: `<limit>;w=<window seconds>` and what is left. */
const policy = (result: Result) => ({
  policy: result.headers.get('ratelimit-policy'),
  remaining: Number(result.headers.get('ratelimit-remaining')),
})

// Thirty failed sign-ins each cost a password hash; allow for a busy host.
scenario('09-rate-limits', { timeoutMs: 300_000 }, async (s) => {
  const ip = (n: number) => `203.0.113.${n}`
  const limited = (step: string, request: Request, address: string): Promise<Result> => s.http(step, { ...request, bypass: false, ip: address })
  const xrpcGet = (step: string, nsid: string, query: Request['query'], address: string, extra: Partial<Request> = {}) =>
    limited(step, { path: `/xrpc/${nsid}`, query, ...extra }, address)
  const xrpcPost = (step: string, nsid: string, json: unknown, address: string, extra: Partial<Request> = {}) =>
    limited(step, { method: 'POST', path: `/xrpc/${nsid}`, json, ...extra }, address)

  const alice = await s.createAccount('alice')

  // The bypass key (SPEC 4.3): no limiter runs and no header is set.
  const bypassed = await s.query('with the bypass key', 'com.atproto.server.describeServer')
  assert.equal(bypassed.headers.get('ratelimit-limit'), null)

  // global-ip: 3,000 points in five minutes, by client address.
  const first = await xrpcGet('global-ip, first request', 'com.atproto.server.describeServer', {}, ip(1))
  assert.deepEqual(policy(first), { policy: '3000;w=300', remaining: 2999 })
  assert.ok(first.headers.get('ratelimit-reset'))
  assert.equal(first.headers.get('access-control-expose-headers'), 'RateLimit-Limit, RateLimit-Reset, RateLimit-Remaining, RateLimit-Policy')
  const second = await xrpcGet('global-ip, second request', 'com.atproto.server.describeServer', {}, ip(1))
  assert.equal(policy(second).remaining, 2998)
  const otherAddress = await xrpcGet('global-ip, another address', 'com.atproto.server.describeServer', {}, ip(2))
  assert.equal(policy(otherAddress).remaining, 2999, 'the bucket is per address')

  // Routes with their own limiter. Each reports the limiter with the least room.
  const observed: Record<string, unknown> = {}
  const see = (name: string, result: Result) => (observed[name] = policy(result))

  see('sync.getRepo', await xrpcGet('getRepo', 'com.atproto.sync.getRepo', { did: alice.did }, ip(3)))
  see('server.createAccount', await xrpcPost('createAccount', 'com.atproto.server.createAccount', { handle: s.handle('limited'), email: s.email('limited'), password: 'limited-password' }, ip(4)))
  see('server.createSession', await xrpcPost('createSession', 'com.atproto.server.createSession', { identifier: alice.handle, password: alice.password }, ip(5)))
  see('server.requestPasswordReset', await xrpcPost('requestPasswordReset', 'com.atproto.server.requestPasswordReset', { email: s.email('nobody') }, ip(6)))
  see('server.resetPassword', await xrpcPost('resetPassword', 'com.atproto.server.resetPassword', { token: 'AAAAA-BBBBB', password: 'whatever-password' }, ip(7)))
  see('server.deleteAccount', await xrpcPost('deleteAccount', 'com.atproto.server.deleteAccount', { did: alice.did, password: 'wrong', token: 'AAAAA-BBBBB' }, ip(8)))
  see('server.requestEmailConfirmation', await xrpcPost('requestEmailConfirmation', 'com.atproto.server.requestEmailConfirmation', undefined, ip(9), { auth: alice }))
  see('server.requestEmailUpdate', await xrpcPost('requestEmailUpdate', 'com.atproto.server.requestEmailUpdate', undefined, ip(10), { auth: alice }))
  see('server.requestAccountDelete', await xrpcPost('requestAccountDelete', 'com.atproto.server.requestAccountDelete', undefined, ip(11), { auth: alice }))
  see('identity.updateHandle', await xrpcPost('updateHandle', 'com.atproto.identity.updateHandle', { handle: s.handle('alicia') }, ip(12), { auth: alice }))
  see('repo.uploadBlob', await limited('uploadBlob', { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body: 'limited', contentType: 'text/plain', auth: alice }, ip(13)))
  s.note('VP-12 limiter reported by each route', observed)
  assert.deepEqual(observed, {
    'sync.getRepo': { policy: '6000;w=300', remaining: 5999 },
    'server.createAccount': { policy: '100;w=300', remaining: 99 },
    'server.createSession': { policy: '30;w=300', remaining: 29 },
    'server.requestPasswordReset': { policy: '15;w=3600', remaining: 14 },
    'server.resetPassword': { policy: '50;w=300', remaining: 49 },
    'server.deleteAccount': { policy: '50;w=300', remaining: 49 },
    'server.requestEmailConfirmation': { policy: '5;w=3600', remaining: 4 },
    'server.requestEmailUpdate': { policy: '5;w=3600', remaining: 4 },
    'server.requestAccountDelete': { policy: '5;w=3600', remaining: 4 },
    'identity.updateHandle': { policy: '10;w=300', remaining: 9 },
    'repo.uploadBlob': { policy: '1000;w=86400', remaining: 999 },
  })

  // The repository write limiters count by weight and by DID: create 3,
  // update 2, delete 1. A response reports the limiter with the fewest points
  // left, so the hourly bucket shows only once it has less room than the
  // address bucket: four batches of 200 creates spend 2,400 of its 5,000.
  const bob = await s.createAccount('bob')
  const record = (text: string) => ({ $type: POST, text, createdAt: '2026-01-01T00:00:00.000Z' })
  const fill = (batch: number) =>
    Array.from({ length: 200 }, (_, i) => ({ $type: 'com.atproto.repo.applyWrites#create', collection: 'com.example.parity.note', rkey: `b${batch}-${String(i).padStart(3, '0')}`, value: { $type: 'com.example.parity.note', n: i } }))
  const untouched = policy(await xrpcPost('first write', 'com.atproto.repo.createRecord', { repo: bob.did, collection: POST, rkey: '3aaaaaaaaaaa7', record: record('zero') }, ip(19), { auth: bob }))
  assert.deepEqual(untouched, { policy: '3000;w=300', remaining: 2999 }, 'the address bucket has less room at first')
  for (let batch = 0; batch < 4; batch++) {
    const filled = await s.http('fill the hourly bucket', { method: 'POST', path: '/xrpc/com.atproto.repo.applyWrites', json: { repo: bob.did, writes: fill(batch) }, auth: bob, bypass: false, ip: ip(50 + batch), silent: true })
    assert.equal(filled.status, 200)
  }
  const writes: Record<string, unknown> = {}
  writes.create = policy(await xrpcPost('createRecord', 'com.atproto.repo.createRecord', { repo: bob.did, collection: POST, rkey: '3aaaaaaaaaaa2', record: record('one') }, ip(20), { auth: bob }))
  writes.update = policy(await xrpcPost('putRecord', 'com.atproto.repo.putRecord', { repo: bob.did, collection: POST, rkey: '3aaaaaaaaaaa2', record: record('two') }, ip(21), { auth: bob }))
  writes.delete = policy(await xrpcPost('deleteRecord', 'com.atproto.repo.deleteRecord', { repo: bob.did, collection: POST, rkey: '3aaaaaaaaaaa2' }, ip(22), { auth: bob }))
  writes.batch = policy(
    await xrpcPost('applyWrites', 'com.atproto.repo.applyWrites', {
      repo: bob.did,
      writes: [
        { $type: 'com.atproto.repo.applyWrites#create', collection: POST, rkey: '3aaaaaaaaaaa3', value: record('a') },
        { $type: 'com.atproto.repo.applyWrites#create', collection: POST, rkey: '3aaaaaaaaaaa4', value: record('b') },
        { $type: 'com.atproto.repo.applyWrites#update', collection: POST, rkey: '3aaaaaaaaaaa7', value: record('c') },
        { $type: 'com.atproto.repo.applyWrites#delete', collection: 'com.example.parity.note', rkey: 'b0-000' },
      ],
    }, ip(23), { auth: bob }),
  )
  s.note('VP-12 repository write limiter', writes)
  // 5,000 - 3 (first write) - 2,400 (fill) = 2,597, then 3, 2, 1, and 3 + 3 + 2 + 1.
  assert.deepEqual(writes, {
    create: { policy: '5000;w=3600', remaining: 2594 },
    update: { policy: '5000;w=3600', remaining: 2592 },
    delete: { policy: '5000;w=3600', remaining: 2591 },
    batch: { policy: '5000;w=3600', remaining: 2582 },
  })

  // Exhaustion: createSession allows 30 attempts in five minutes for one identifier from one address.
  const attempt = (address: string, identifier: string) =>
    s.http('createSession attempt', { method: 'POST', path: '/xrpc/com.atproto.server.createSession', json: { identifier, password: 'wrong-password' }, bypass: false, ip: address, silent: true })
  for (let i = 0; i < 30; i++) assert.equal((await attempt(ip(30), bob.handle)).status, 401)
  const blocked = await xrpcPost('createSession, 31st attempt', 'com.atproto.server.createSession', { identifier: bob.handle, password: bob.password }, ip(30))
  assert.equal(blocked.status, 429)
  refused(blocked, 'RateLimitExceeded')
  assert.ok(Number(blocked.headers.get('retry-after')) > 0, '429 carries Retry-After')
  assert.equal(policy(blocked).policy, '30;w=300')
  assert.equal(policy(blocked).remaining, 0)
  s.note('429 exposes', blocked.headers.get('access-control-expose-headers'))
  // The key is the identifier and the address together.
  assert.equal((await attempt(ip(31), bob.handle)).status, 401, 'another address is not blocked')
  assert.equal((await attempt(ip(30), alice.handle)).status, 401, 'another identifier is not blocked')
  // The bypass key lifts the block.
  const lifted = await s.procedure('createSession with the bypass key while blocked', 'com.atproto.server.createSession', { identifier: bob.handle, password: bob.password }, { ip: ip(30) })
  assert.equal(lifted.status, 200)

  // Exhaustion by DID: five confirmation mails an hour.
  for (let i = 0; i < 5; i++) {
    const sent = await s.http('requestEmailConfirmation', { method: 'POST', path: '/xrpc/com.atproto.server.requestEmailConfirmation', auth: bob, bypass: false, ip: ip(40 + i), silent: true })
    assert.equal(sent.status, 200)
  }
  const sixth = await limited('requestEmailConfirmation, sixth in the hour', { method: 'POST', path: '/xrpc/com.atproto.server.requestEmailConfirmation', auth: bob }, ip(46))
  assert.equal(sixth.status, 429, 'the bucket follows the account across addresses')
  refused(sixth, 'RateLimitExceeded')
})
