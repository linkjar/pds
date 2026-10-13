// Oracle O3, last clause: a Sync 1.1 relay in strict mode accepts every
// commit. The relay drops a commit that fails validation (signature, MST
// inversion against `prevData`, data limits), so a commit that reaches the
// relay's own firehose was accepted.
//
// This is `default/07-relay` without the account events of unit 5. In the
// Candidate's pass the relay first followed the Reference on this data
// directory and then finds another server behind the same name, with the
// same log.

import assert from 'node:assert/strict'
import { forDids, isEvent, subscribe } from '../../src/lib/firehose.ts'
import { summariseAll } from '../../src/oracles/firehose.ts'
import { scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const POST = 'app.bsky.feed.post'
const post = (text: string) => ({ $type: POST, text, createdAt: '2026-01-01T00:00:00.000Z' })

scenario('21-relay', { timeoutMs: 240_000 }, async (s) => {
  const alice = await s.createAccount('alice')
  const fromPds = subscribe(s.target, HOSTS.pds)
  const fromRelay = subscribe(s.target, HOSTS.relay)
  s.onCleanup(() => (fromPds.close(), fromRelay.close()))
  await fromPds.quiet()

  const hosts = await s.eventually('the relay to list the PDS', async () => {
    const result = await s.http('relay hosts', { host: HOSTS.relay, path: '/xrpc/com.atproto.sync.listHosts', silent: true })
    return result.json?.hosts?.length > 0 && result
  })
  s.note('relay hosts', hosts.json.hosts.map((h: Record<string, unknown>) => ({ hostname: h.hostname, status: h.status })))
  assert.equal(hosts.json.hosts[0].hostname, HOSTS.pds)

  const mine = forDids([alice.did])
  const write = (nsid: string, input: Record<string, unknown>) =>
    s.procedure(nsid, `com.atproto.repo.${nsid}`, { repo: alice.did, ...input }, { auth: alice, silent: true })
  await write('createRecord', { collection: POST, rkey: '3aaaaaaaaaaa2', record: post('one') })
  await write('putRecord', { collection: POST, rkey: '3aaaaaaaaaaa2', record: post('one, edited') })
  await write('applyWrites', {
    writes: Array.from({ length: 50 }, (_, i) => ({ $type: 'com.atproto.repo.applyWrites#create', collection: POST, rkey: `3aaaaaaaaab${String.fromCharCode(97 + Math.floor(i / 26))}${String.fromCharCode(97 + (i % 26))}`, value: post(`bulk ${i}`) })),
  })
  await write('deleteRecord', { collection: POST, rkey: '3aaaaaaaaaaa2' })
  const more = await write('createRecord', { collection: POST, rkey: '3aaaaaaaaaaa3', record: post('the last one') })
  assert.equal(more.status, 200)

  const isLast = (f: Parameters<typeof mine>[0]) => isEvent(f) && f.type === '#commit' && f.body.rev === more.json.commit.rev
  await fromPds.waitFor(isLast)
  // The relay may have to find the server again first; it retries with a delay.
  await fromRelay.waitFor(isLast, 1, 120_000)
  await fromRelay.settle()

  const pdsEvents = await summariseAll(fromPds.frames.filter(mine))
  const relayEvents = await summariseAll(fromRelay.frames.filter(mine))
  s.note('events on the PDS firehose', pdsEvents.map((e) => e.type))
  s.note('events on the relay firehose', relayEvents.map((e) => e.type))

  // Every commit the PDS emitted came out of the relay, unchanged.
  assert.equal(relayEvents.length, pdsEvents.length, 'the strict relay dropped no commit')
  assert.deepEqual(s.norm.value(relayEvents), s.norm.value(pdsEvents))

  const status = await s.http('relay getRepoStatus', { host: HOSTS.relay, path: '/xrpc/com.atproto.sync.getRepoStatus', query: { did: alice.did } })
  assert.equal(status.status, 200)
  assert.equal(status.json.active, true)
  assert.equal(status.json.rev, more.json.commit.rev)
  const latest = await s.http('relay getLatestCommit', { host: HOSTS.relay, path: '/xrpc/com.atproto.sync.getLatestCommit', query: { did: alice.did } })
  assert.equal(latest.json.cid, more.json.commit.cid)
})
