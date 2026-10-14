// The crash boundary of SPEC 7.4, on the Candidate: the server stops after
// a commit is stored and before its event is in the log. At the next start
// it states the head with `#sync`, and the relay converges.
//
// The stack runs with one fail point armed to abort once
// (`writes-crash` in src/stack/env.ts). This scenario has no Reference
// pass: the Reference has no such hook, and it orders the two steps the
// other way round.

import assert from 'node:assert/strict'
import { forDids, isEvent, subscribe } from '../../src/lib/firehose.ts'
import { checkRepo } from '../../src/oracles/repo.ts'
import { scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const NOTE = 'com.example.parity.note'

scenario('30-crash', { timeoutMs: 300_000 }, async (s) => {
  const alice = await s.createAccount('alice')
  const before = await s.query('head before', 'com.atproto.sync.getLatestCommit', { did: alice.did }, { silent: true })
  const write = (rkey: string) =>
    s.procedure('write', 'com.atproto.repo.createRecord', { repo: alice.did, collection: NOTE, rkey, record: { $type: NOTE, text: rkey } }, { auth: alice, silent: true })

  // The server stops inside this write: the client gets no answer.
  const interrupted = await write('interrupted').then(
    (result) => ({ status: result.status }),
    () => ({ status: null }),
  )
  s.note('the write that the stop interrupted', interrupted)
  assert.notEqual(interrupted.status, 200, 'the client got no success')

  await s.target.restart()
  const relay = subscribe(s.target, HOSTS.relay)
  s.onCleanup(() => relay.close())

  // The commit was stored: the head moved, and the record is there.
  const after = await s.query('head after the restart', 'com.atproto.sync.getLatestCommit', { did: alice.did }, { silent: true })
  assert.equal(after.status, 200)
  assert.notEqual(after.json.cid, before.json.cid, 'the commit was durable before the stop')
  const record = await s.query('the interrupted record', 'com.atproto.repo.getRecord', { repo: alice.did, collection: NOTE, rkey: 'interrupted' }, { silent: true })
  assert.equal(record.status, 200)

  // The log states the head with #sync, and has no #commit for it.
  const replay = subscribe(s.target, HOSTS.pds, 0)
  s.onCleanup(() => replay.close())
  const mine = forDids([alice.did])
  const sync = await replay.waitFor((f) => mine(f) && isEvent(f) && f.type === '#sync' && f.body.rev === after.json.rev)
  assert.equal(sync.length, 1)
  await replay.settle()
  assert.equal(replay.frames.filter((f) => mine(f) && isEvent(f) && f.type === '#commit' && f.body.rev === after.json.rev).length, 0)

  // The next write follows the repaired head, and the strict relay takes it.
  const next = await write('next')
  assert.equal(next.status, 200, next.text)
  await relay.waitFor((f) => isEvent(f) && f.type === '#commit' && f.body.rev === next.json.commit.rev, 1, 120_000)
  const latest = await s.eventually('the relay to hold the new head', async () => {
    const result = await s.http('relay getLatestCommit', { host: HOSTS.relay, path: '/xrpc/com.atproto.sync.getLatestCommit', query: { did: alice.did }, silent: true })
    return result.json?.cid === next.json.commit.cid && result
  }, 60_000)
  assert.equal(latest.json.rev, next.json.commit.rev)
  s.note('after the repair', { syncSent: true, relayConverged: true })
  await checkRepo(s, 'after the repair', alice)
})
