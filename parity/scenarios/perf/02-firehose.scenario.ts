// SPEC 16.1, S9, S5 and S4: many writers across actors, a long backfill, and
// delivery lag with a growing number of subscribers.
//
// S4 runs last, so that its largest step cannot disturb the other
// measurements, and it records the write throughput of the same writers
// before the subscribers and after they left.

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
const BURST_SECONDS = SCALE === 'full' ? 10 : 4

const write = (s: Scenario, account: Account, n: number) =>
  s.http('write', { method: 'POST', path: '/xrpc/com.atproto.repo.createRecord', json: { repo: account.did, collection: NOTE, record: { $type: NOTE, n } }, auth: account, silent: true })

type Paced = { sent: Map<string, number>; failed: Record<string, number> }

/**
 * Sends `perSecond` writes a second for `seconds` and returns when each was
 * sent, by the `rev` it produced. A write the server did not answer with 200
 * is counted, not retried: under enough load a failed write is the result.
 */
async function paced(s: Scenario, account: Account, perSecond: number, seconds: number, latencies: number[]): Promise<Paced> {
  const sent = new Map<string, number>()
  const failed: Record<string, number> = {}
  const start = performance.now()
  const pending: Promise<void>[] = []
  for (let i = 0; i < perSecond * seconds; i++) {
    const due = start + (i * 1000) / perSecond
    const wait = due - performance.now()
    if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait))
    const at = performance.now()
    pending.push(
      write(s, account, i).then(
        (response) => {
          if (response.status !== 200) {
            failed[String(response.status)] = (failed[String(response.status)] ?? 0) + 1
            return
          }
          latencies.push(performance.now() - at)
          sent.set(response.json.commit.rev, at)
        },
        () => {
          failed.network = (failed.network ?? 0) + 1
        },
      ),
    )
  }
  await Promise.all(pending)
  return { sent, failed }
}

/** How many subscribers decode what they receive. The rest only hold a connection and count. */
const PROBES = 20

type Listeners = { probes: Map<string, number>[]; received: () => number; close: () => void }

/**
 * Opens `count` subscriptions. Decoding every frame on every one of a
 * thousand connections would measure this process and not the server, so at
 * most `PROBES` of them record when each commit arrived.
 */
function listen(s: Scenario, count: number): Listeners {
  const probes: Map<string, number>[] = []
  const subs: Subscription[] = []
  for (let i = 0; i < count; i++) {
    if (i < PROBES) {
      const arrivals = new Map<string, number>()
      probes.push(arrivals)
      subs.push(subscribe(s.target, HOSTS.pds, undefined, { keep: false, onFrame: (frame) => { if (frame.kind === 'event' && frame.type === '#commit') arrivals.set(String(frame.body.rev), frame.at) } }))
    } else {
      subs.push(subscribe(s.target, HOSTS.pds, undefined, { countOnly: true }))
    }
  }
  const close = () => { for (const sub of subs) sub.close() }
  s.onCleanup(close)
  return { probes, received: () => subs.reduce((sum, sub) => sum + sub.received(), 0), close }
}

/** Lag from the moment a write was sent to the moment each probing subscriber saw its commit. */
function lags(listeners: Listeners, sent: Map<string, number>): number[] {
  const out: number[] = []
  for (const arrivals of listeners.probes) for (const [rev, at] of sent) if (arrivals.has(rev)) out.push(arrivals.get(rev)! - at)
  return out
}

scenario('02-firehose', { timeoutMs: 3 * 60 * 60 * 1000 }, async (s) => {
  // A fresh process: the scenario before this one left a repository of 100,000 records behind.
  await s.target.restart()
  const writer = await s.createAccount('writer', {}, { silent: true })
  const writers: Account[] = []
  for (let i = 0; i < S9.writers; i++) writers.push(await s.createAccount(`w${i}`, {}, { silent: true }))
  await new Promise((resolve) => setTimeout(resolve, 5_000))

  /** Every writer writes as fast as it is answered, for `seconds`. */
  const burst = async (seconds: number) => {
    const until = performance.now() + seconds * 1000
    let commits = 0
    let failed = 0
    let slowest = 0
    await Promise.all(
      writers.map(async (account) => {
        while (performance.now() < until) {
          const at = performance.now()
          const response = await write(s, account, commits).catch(() => undefined)
          if (response?.status === 200) commits++
          else failed++
          slowest = Math.max(slowest, performance.now() - at)
        }
      }),
    )
    return { writers: writers.length, commitsPerSecond: Math.round(commits / seconds), failed, slowestMs: Math.round(slowest) }
  }

  // S9: many writers across actors, two commits a second each.
  const observer = listen(s, 1)
  await new Promise((resolve) => setTimeout(resolve, 2_000))
  const commitLatencies: number[] = []
  const sentByAll = new Map<string, number>()
  const failedByAll: Record<string, number> = {}
  await Promise.all(
    writers.map(async (account) => {
      const { sent, failed } = await paced(s, account, 2, S9.seconds, commitLatencies)
      for (const [rev, at] of sent) sentByAll.set(rev, at)
      for (const [status, count] of Object.entries(failed)) failedByAll[status] = (failedByAll[status] ?? 0) + count
    }),
  )
  await new Promise((resolve) => setTimeout(resolve, 2_500))
  const s9 = {
    writers: S9.writers,
    attempted: S9.writers * 2 * S9.seconds,
    failed: failedByAll,
    commits: sentByAll.size,
    lagMs: summary(lags(observer, sentByAll)),
    commitMs: summary(commitLatencies),
    // Not observable from outside the Reference. The Candidate reports its own counter (SPEC 16.2, T7).
    sequencerFsyncs: null,
  }
  observer.close()

  // S5: backfill from a cursor. The events are produced first, across the same actors.
  const head = async () => {
    const [row] = await s.target.sqlite('sequencer', 'select max(seq) as seq from repo_seq')
    return Number(row!.seq)
  }
  let have = await head()
  const perWriter = Math.ceil(Math.max(0, S5_EVENTS - have) / writers.length)
  const [seedMs] = await timed(() =>
    Promise.all(
      writers.map(async (account) => {
        // Seeding only has to produce events; a write that fails is sent again.
        for (let i = 0; i < perWriter; i++) {
          for (let attempt = 0; ; attempt++) {
            const response = await write(s, account, i).catch(() => undefined)
            if (response?.status === 200) break
            assert.ok(attempt < 20, `seeding write kept failing: ${response?.status}`)
            await new Promise((resolve) => setTimeout(resolve, 250))
          }
        }
      }),
    ),
  )
  have = await head()
  const from = have - S5_EVENTS
  assert.ok(from >= 0, `only ${have} events were sequenced`)
  const [growth, backfillMs] = await memoryGrowth(s, async () => {
    let events = 0
    let done: () => void = () => undefined
    const finished = new Promise<void>((resolve) => (done = resolve))
    const start = performance.now()
    let fail: (error: Error) => void = () => undefined
    const broken = new Promise<never>((_, reject) => (fail = reject))
    const sub = subscribe(s.target, HOSTS.pds, from, {
      keep: false,
      onFrame: (frame) => {
        if (frame.kind === 'error') return fail(new Error(`the backfill ended with ${frame.error} after ${events} events`))
        events++
        if (frame.body.seq === have) done()
      },
    })
    void sub.closed.then(() => fail(new Error(`the server closed the backfill after ${events} events`)))
    // Cleared on the way out: a pending timer would keep the scenario's process alive.
    let stall: NodeJS.Timeout | undefined
    const stalled = new Promise<never>((_, reject) => (stall = setTimeout(() => reject(new Error(`backfill stalled after ${events} events`)), 30 * 60 * 1000)))
    try {
      await Promise.race([finished, broken, stalled])
    } finally {
      clearTimeout(stall)
    }
    const elapsed = performance.now() - start
    sub.close()
    assert.ok(events >= S5_EVENTS, `received ${events} of ${S5_EVENTS} events`)
    return elapsed
  })
  const s5 = { events: S5_EVENTS, wallMs: Math.round(backfillMs), eventsPerSecond: Math.round(S5_EVENTS / (backfillMs / 1000)), memoryGrowthMiB: growth, seedCommitsPerSecond: Math.round((perWriter * writers.length) / (seedMs / 1000)) }

  // S4: one writer at 50 commits a second, with the write throughput around it.
  await new Promise((resolve) => setTimeout(resolve, 3_000))
  const before = await burst(BURST_SECONDS)
  const s4: Record<string, unknown>[] = []
  for (const count of SUBSCRIBERS) {
    const listeners = listen(s, count)
    await new Promise((resolve) => setTimeout(resolve, 2_000))
    const baseline = listeners.received()
    const latencies: number[] = []
    const memoryBefore = await memory(s)
    const { sent, failed } = await paced(s, writer, 50, S4_SECONDS, latencies)
    await new Promise((resolve) => setTimeout(resolve, 3_000))
    s4.push({
      subscribers: count,
      attempted: 50 * S4_SECONDS,
      failed,
      commits: sent.size,
      delivered: listeners.received() - baseline,
      expected: sent.size * count,
      probes: listeners.probes.length,
      lagMs: summary(lags(listeners, sent)),
      commitMs: summary(latencies),
      memoryMiB: Math.round((await memory(s)) / (1024 * 1024)),
      memoryGrowthMiB: Math.round((((await memory(s)) - memoryBefore) / (1024 * 1024)) * 10) / 10,
    })
    listeners.close()
    await new Promise((resolve) => setTimeout(resolve, 1_000))
  }
  await new Promise((resolve) => setTimeout(resolve, 2_000))
  const rightAfter = await burst(BURST_SECONDS)
  await new Promise((resolve) => setTimeout(resolve, 15_000))
  const later = await burst(BURST_SECONDS)
  const aftermath = { subscribersBefore: SUBSCRIBERS.at(-1), before, rightAfter, fifteenSecondsLater: later, memoryMiB: Math.round((await memory(s)) / (1024 * 1024)) }

  saveResults(s, 'S4-S5-S9', { S9: s9, S5: s5, S4: s4, aftermath })
})
