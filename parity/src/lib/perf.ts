// Measurement helpers for the performance scenarios (SPEC 16.1). The numbers
// are taken from outside the server, so they hold for any target: wall-clock
// latency at the client, and the container's anonymous memory from its
// cgroup, which is resident memory without the page cache.

import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { performance } from 'node:perf_hooks'
import type { Scenario } from '../scenario.ts'
import { targetSpec } from '../stack/targets.ts'

export type Scale = 'full' | 'smoke'

/** `smoke` runs the same code at a fraction of the size, to check the harness; only `full` is a baseline. */
export const SCALE: Scale = process.env.PARITY_PERF_SCALE === 'smoke' ? 'smoke' : 'full'

export function percentile(values: number[], p: number): number {
  if (values.length === 0) return Number.NaN
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1)]!
}

const round = (ms: number) => Math.round(ms * 10) / 10

/** p50, p99 and the maximum of a set of latencies, in milliseconds. */
export function summary(values: number[]): { n: number; p50: number; p99: number; max: number } {
  return { n: values.length, p50: round(percentile(values, 50)), p99: round(percentile(values, 99)), max: round(Math.max(...values)) }
}

/** Runs `fn` and returns how long it took, in milliseconds, with its result. */
export async function timed<T>(fn: () => Promise<T>): Promise<[number, T]> {
  const start = performance.now()
  const value = await fn()
  return [performance.now() - start, value]
}

/** Anonymous memory of the PDS container, in bytes. */
export function memory(s: Scenario): number {
  const stat = s.target.exec(['cat', '/sys/fs/cgroup/memory.stat']).stdout
  const anon = /^anon (\d+)$/m.exec(stat)
  return anon ? Number(anon[1]) : Number.NaN
}

/** Samples memory while `fn` runs and returns the growth of the peak over the starting value, in MiB. */
export async function memoryGrowth<T>(s: Scenario, fn: () => Promise<T>): Promise<[number, T]> {
  const before = memory(s)
  let peak = before
  let running = true
  const sampler = (async () => {
    while (running) {
      peak = Math.max(peak, memory(s))
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  })()
  try {
    const value = await fn()
    peak = Math.max(peak, memory(s))
    return [Math.round(((peak - before) / (1024 * 1024)) * 10) / 10, value]
  } finally {
    running = false
    await sampler
  }
}

/** Writes one scenario's measurements where `pnpm parity perf report` collects them. */
export function saveResults(s: Scenario, id: string, results: unknown): void {
  const dir = join(targetSpec(s.target.name).runDir, 'perf')
  mkdirSync(dir, { recursive: true })
  writeFileSync(join(dir, `${id}.json`), JSON.stringify({ scale: SCALE, image: s.target.state.image, measuredAt: new Date().toISOString(), results }, null, 2) + '\n')
  s.note(`${id} measurements`, { scale: SCALE, recorded: true })
}
