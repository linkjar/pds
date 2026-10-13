// The firehose of commits: SPEC 7.1 (events), 7.2 (framing) and 7.3
// (cursors, VP-1). Oracle O3.
//
// This is `default/06-firehose` without the account events of SPEC 10.4,
// which need the account endpoints of unit 5. The account exists before the
// scenario starts, so the stream is joined after its first commit. The log
// already holds the events the Reference wrote, and a cursor reads through
// them.

import assert from 'node:assert/strict'
import { forDids, isEvent, subscribe } from '../../src/lib/firehose.ts'
import { checkRepoStream, summariseAll } from '../../src/oracles/firehose.ts'
import { scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const POST = 'app.bsky.feed.post'
const post = (text: string) => ({ $type: POST, text, createdAt: '2026-01-01T00:00:00.000Z' })

scenario('20-firehose', {}, async (s) => {
  const alice = await s.createAccount('alice')

  // No cursor: live events only (SPEC 7.3). Nothing arrives until something happens.
  const live = subscribe(s.target, HOSTS.pds)
  s.onCleanup(() => live.close())
  await live.quiet()
  assert.equal(live.frames.length, 0, 'a subscription without a cursor replays nothing')

  // SPEC 7.1: one commit per write, with the operation and its links.
  const mine = forDids([alice.did])
  const write = (step: string, nsid: string, input: Record<string, unknown>) =>
    s.procedure(step, `com.atproto.repo.${nsid}`, { repo: alice.did, ...input }, { auth: alice, silent: true })
  await write('create', 'createRecord', { collection: POST, rkey: '3aaaaaaaaaaa2', record: post('one') })
  await write('update', 'putRecord', { collection: POST, rkey: '3aaaaaaaaaaa2', record: post('one, edited') })
  await write('batch', 'applyWrites', {
    writes: [
      { $type: 'com.atproto.repo.applyWrites#create', collection: POST, rkey: '3aaaaaaaaaaa3', value: post('two') },
      { $type: 'com.atproto.repo.applyWrites#create', collection: POST, rkey: '3aaaaaaaaaaa4', value: post('three') },
      { $type: 'com.atproto.repo.applyWrites#delete', collection: POST, rkey: '3aaaaaaaaaaa2' },
    ],
  })
  const last = await write('delete', 'deleteRecord', { collection: POST, rkey: '3aaaaaaaaaaa3' })
  // A write that changes nothing makes no commit and no event.
  await write('delete again', 'deleteRecord', { collection: POST, rkey: '3aaaaaaaaaaa3' })
  const afterWrites = await live.waitFor(mine, 4)
  await live.settle()
  assert.equal(live.frames.filter(mine).length, 4, 'four writes, four commits, and no event for the no-op')
  s.note('events of four writes', await summariseAll(afterWrites))
  await checkRepoStream(s, alice.did, live.frames, { midStream: true })

  // SPEC 7.3: cursors.
  const all = live.frames.filter(isEvent)
  const firstSeq = all[0]!.body.seq as number
  const lastSeq = all.at(-1)!.body.seq as number
  assert.equal(all.at(-1)!.body.rev, last.json.commit.rev)

  // cursor=0: the whole retained window, from the oldest event. The
  // Reference wrote the first events of this log.
  const fromZero = subscribe(s.target, HOSTS.pds, 0)
  s.onCleanup(() => fromZero.close())
  await fromZero.waitFor((f) => isEvent(f) && f.body.seq === lastSeq)
  assert.equal(fromZero.frames.filter(isEvent)[0]!.body.seq, 1, 'cursor=0 starts at the oldest event')
  const seqs = fromZero.frames.filter(isEvent).map((f) => f.body.seq as number)
  assert.deepEqual(seqs, [...seqs].sort((a, b) => a - b), 'backfill is in seq order')
  assert.equal(new Set(seqs).size, seqs.length, 'no duplicates')
  s.note('event types from cursor 0', [...new Set((await summariseAll(fromZero.frames)).map((e) => e.type))].sort())

  // VP-1: a cursor inside the window is exclusive. The first frame is cursor + 1.
  const mid = firstSeq + 1
  const fromMid = subscribe(s.target, HOSTS.pds, mid)
  s.onCleanup(() => fromMid.close())
  await fromMid.waitFor((f) => isEvent(f) && f.body.seq === lastSeq)
  assert.equal(fromMid.frames.filter(isEvent)[0]!.body.seq, mid + 1, 'VP-1: backfill is exclusive of the cursor')
  s.note('VP-1 first seq minus cursor', (fromMid.frames.filter(isEvent)[0]!.body.seq as number) - mid)

  // Backfill, then live, without a gap or a duplicate.
  await s.procedure('a write after the backfill', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa5', record: post('four') }, { auth: alice, silent: true })
  await fromMid.waitFor((f) => isEvent(f) && f.body.seq === lastSeq + 1)
  const stitched = fromMid.frames.filter(isEvent).map((f) => f.body.seq as number)
  assert.deepEqual(stitched, Array.from({ length: stitched.length }, (_, i) => mid + 1 + i), 'contiguous from backfill into live')

  // The cursor at the head: nothing is replayed.
  const fromHead = subscribe(s.target, HOSTS.pds, lastSeq + 1)
  s.onCleanup(() => fromHead.close())
  await new Promise((resolve) => setTimeout(resolve, 500))
  assert.equal(fromHead.frames.length, 0)

  // A cursor above the head: FutureCursor, then close.
  const future = subscribe(s.target, HOSTS.pds, lastSeq + 1_000_000)
  const closed = await future.closed
  s.note('cursor above the head', { frames: await summariseAll(future.frames), closeCode: closed.code })
  assert.equal(future.frames.length, 1)
  assert.equal(future.frames[0]!.kind, 'error')
  assert.equal((future.frames[0] as { error: string }).error, 'FutureCursor')

  // A cursor that is not a number.
  const garbage = subscribe(s.target, HOSTS.pds, 'abc')
  const garbageClosed = await Promise.race([garbage.closed, new Promise<null>((resolve) => setTimeout(() => resolve(null), 2_000))])
  s.note('cursor that is not a number', { frames: await summariseAll(garbage.frames), closed: garbageClosed })
  garbage.close()

  // SPEC 7.2: the endpoint without an upgrade. The Reference has no HTTP
  // route for it, so the request falls to the catch-all proxy.
  await s.http('subscribeRepos as a plain GET', { path: '/xrpc/com.atproto.sync.subscribeRepos' })
  await s.http('subscribeRepos as a POST', { method: 'POST', path: '/xrpc/com.atproto.sync.subscribeRepos' })
})
