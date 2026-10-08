// The two rows of SPEC 7.3 that depend on configured limits, with the
// limits made small: a five-second backfill window and a subscriber buffer
// of five events.

import assert from 'node:assert/strict'
import { isEvent, subscribe } from '../../src/lib/firehose.ts'
import { summariseAll } from '../../src/oracles/firehose.ts'
import { scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const NOTE = 'com.example.parity.note'

scenario('01-cursor-limits', { timeoutMs: 180_000 }, async (s) => {
  const alice = await s.createAccount('alice', {}, { silent: true })
  const write = (rkey: string, record: Record<string, unknown>) =>
    s.http('write', { method: 'POST', path: '/xrpc/com.atproto.repo.applyWrites', json: { repo: alice.did, writes: [{ $type: 'com.atproto.repo.applyWrites#create', collection: NOTE, rkey, value: { $type: NOTE, ...record } }] }, auth: alice, silent: true })

  // A cursor older than PDS_REPO_BACKFILL_LIMIT_MS: an #info frame, then the events still in the window.
  assert.equal((await write('old', { n: 1 })).status, 200)
  await new Promise((resolve) => setTimeout(resolve, 6_500))
  const recent = await write('recent', { n: 2 })
  assert.equal(recent.status, 200)
  const outdated = subscribe(s.target, HOSTS.pds, 1)
  s.onCleanup(() => outdated.close())
  await outdated.waitFor((f) => isEvent(f) && f.type === '#commit')
  await outdated.settle()
  s.note('cursor older than the backfill window', await summariseAll(outdated.frames))
  const [info, ...rest] = outdated.frames.filter(isEvent)
  assert.equal(info!.type, '#info')
  assert.equal(info!.body.name, 'OutdatedCursor')
  assert.equal(rest.length, 1, 'only the event inside the window follows')
  assert.equal(rest[0]!.body.rev, recent.json.commit.rev)

  // cursor=0 against the same window.
  const fromZero = subscribe(s.target, HOSTS.pds, 0)
  s.onCleanup(() => fromZero.close())
  await fromZero.waitFor((f) => isEvent(f) && f.type === '#commit')
  await fromZero.settle()
  s.note('cursor=0 with events older than the window', (await summariseAll(fromZero.frames)).map((e) => e.type))

  // A subscriber that stops reading. Commits of about 900 KB fill the
  // transport quickly; once it is full the server's own buffer of five
  // events overflows and the server ends the stream with ConsumerTooSlow.
  const slow = subscribe(s.target, HOSTS.pds)
  await slow.quiet()
  slow.pause()
  const filler = 'x'.repeat(900_000)
  for (let i = 0; i < 60; i++) assert.equal((await write(`big-${String(i).padStart(2, '0')}`, { filler })).status, 200)
  slow.resume()
  const closed = await Promise.race([slow.closed, new Promise<null>((resolve) => setTimeout(() => resolve(null), 30_000))])
  const last = slow.frames.at(-1)
  s.note('a subscriber that stopped reading', { last: last?.kind === 'error' ? { error: last.error, message: last.message } : last?.kind, closed })
  assert.ok(closed, 'the server closed the stream')
  assert.equal(last?.kind, 'error')
  assert.equal((last as { error: string }).error, 'ConsumerTooSlow')
  assert.ok(slow.frames.filter(isEvent).length < 60, 'the stream ended before every event was delivered')
})
