// SPEC 16.1, S4, S9 and S5: delivery lag with a growing number of
// subscribers, many writers across actors, and a long backfill.

import assert from 'node:assert/strict'
import { performance } from 'node:perf_hooks'
import { subscribe } from '../../src/lib/firehose.ts'
import type { Subscription } from '../../src/lib/firehose.ts'
import { SCALE, memory, memoryGrowth, saveResults, summary, timed } from '../../src/lib/perf.ts'
import { scenario } from '../../src/scenario.ts'
import type { Account, Scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const NOTE = 'com.example.parity.note'
const SUBSCRIBERS = SCALE === 'full' ? [1, 10, 100, 1_000] : [1, 10, 50]
const S4_SECONDS = SCALE === 'full' ? 10 : 3
const S9 = SCALE === 'full' ? { writers: 50, seconds: 30 } : { writers: 10, seconds: 5 }
const S5_EVENTS = SCALE === 'full' ? 100_000 : 3_000

const write = (s: Scenario, account: Account, n: number) =>
  s.http('write', { method: 'POST', path: '/xrpc/com.atproto.repo.createRecord', json: { repo: account.did, collection: NOTE, record: { $type: NOTE, n } }, auth: account, silent: true })

/** Paces `count` writes at `perSecond` and returns when each was sent, by the `rev` it produced. */
async function paced(s: Scenario, account: Account, perSecond: number, seconds: number, latencies: number[]): Promise<Map<string, number>> {
  const sent = new Map<string, number>()
  const start = performance.now()
  const pending: Promise<void>[] = []
  for (let i = 0; i < perSecond * seconds; i++) {
    const due = start + (i * 1000) / perSecond
    const wait = due - performance.now()
    if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait))
    const at = performance.now()
    pending.push(
      write(s, account, i).then((response) => {
        assert.equal(response.status, 200, response.text)
        latencies.push(performance.now() - at)
        sent.set(response.json.commit.rev, at)
      }),
    )
  }
  await Promise.all(pending)
  return sent
}

/** Lag from the moment a write was sent to the moment each subscriber saw its commit. */
function lags(subscribers: { sub: Subscription; arrivals: Map<string, number> }[], sent: Map<string, number>): number[] {
  const out: number[] = []
  for (const { arrivals } of subscribers) for (const [rev, at] of sent) if (arrivals.has(rev)) out.push(arrivals.get(rev)! - at)
  return out
}

function listen(s: Scenario, count: number): { sub: Subscription; arrivals: Map<string, number> }[] {
  return Array.from({ length: count }, () => {
    const sub = subscribe(s.target, HOSTS.pds)
    const arrivals = new Map<string, number>()
    let seen = 0
    const timer = setInterval(() => {
      for (; seen < sub.frames.length; seen++) {
        const frame = sub.frames[seen]!
        if (frame.kind === 'event' && frame.type === '#commit') arrivals.set(String(frame.body.rev), frame.at)
      }
    }, 20)
    s.onCleanup(() => (clearInterval(timer), sub.close()))
    return { sub, arrivals }
  })
}

scenario('02-firehose', { timeoutMs: 3 * 60 * 60 * 1000 }, async (s) => {
  const writer = await s.createAccount('writer', {}, { silent: true })

  // S4: one writer at 50 commits a second.
  const s4: Record<string, unknown>[] = []
  for (const count of SUBSCRIBERS) {
    const subscribers = listen(s, count)
    await new Promise((resolve) => setTimeout(resolve, 1_500))
    const latencies: number[] = []
    const before = memory(s)
    const sent = await paced(s, writer, 50, S4_SECONDS, latencies)
    await new Promise((resolve) => setTimeout(resolve, 2_500))
    const all = lags(subscribers, sent)
    s4.push({ subscribers: count, commits: sent.size, delivered: all.length, expected: sent.size * count, lagMs: summary(all), commitMs: summary(latencies), memoryMiB: Math.round(memory(s) / (1024 * 1024)), memoryGrowthMiB: Math.round(((memory(s) - before) / (1024 * 1024)) * 10) / 10 })
    for (const { sub } of subscribers) sub.close()
  }

  // S9: many writers across actors, two commits a second each.
  const writers: Account[] = []
  for (let i = 0; i < S9.writers; i++) writers.push(await s.createAccount(`w${i}`, {}, { silent: true }))
  const observer = listen(s, 1)
  await new Promise((resolve) => setTimeout(resolve, 1_500))
  const commitLatencies: number[] = []
  const sentByAll = new Map<string, number>()
  await Promise.all(writers.map(async (account) => { for (const [rev, at] of await paced(s, account, 2, S9.seconds, commitLatencies)) sentByAll.set(rev, at) }))
  await new Promise((resolve) => setTimeout(resolve, 2_500))
  const s9 = {
    writers: S9.writers,
    commits: sentByAll.size,
    lagMs: summary(lags(observer, sentByAll)),
    commitMs: summary(commitLatencies),
    // Not observable from outside the Reference. The Candidate reports its own counter (SPEC 16.2, T7).
    sequencerFsyncs: null,
  }

  // S5: backfill from a cursor. The events are produced first, across the same actors.
  const head = async () => {
    const [row] = await s.target.sqlite('sequencer', 'select max(seq) as seq from repo_seq')
    return Number(row!.seq)
  }
  let have = await head()
  const perWriter = Math.ceil(Math.max(0, S5_EVENTS - have) / writers.length)
  const [seedMs] = await timed(() =>
    Promise.all(writers.map(async (account) => { for (let i = 0; i < perWriter; i++) assert.equal((await write(s, account, i)).status, 200) })),
  )
  have = await head()
  const from = have - S5_EVENTS
  assert.ok(from >= 0, `only ${have} events were sequenced`)
  const [growth, backfillMs] = await memoryGrowth(s, async () => {
    const sub = subscribe(s.target, HOSTS.pds, from)
    const start = performance.now()
    await sub.waitFor((frame) => frame.kind === 'event' && frame.body.seq === have, 1, 30 * 60 * 1000)
    const elapsed = performance.now() - start
    assert.equal(sub.frames.filter((frame) => frame.kind === 'event').length >= S5_EVENTS, true)
    sub.close()
    return elapsed
  })
  const s5 = { events: S5_EVENTS, wallMs: Math.round(backfillMs), eventsPerSecond: Math.round(S5_EVENTS / (backfillMs / 1000)), memoryGrowthMiB: growth, seedCommitsPerSecond: Math.round((perWriter * writers.length) / (seedMs / 1000)) }

  saveResults(s, 'S4-S5-S9', { S4: s4, S9: s9, S5: s5 })
})
