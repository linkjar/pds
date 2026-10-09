// SPEC 16.1, S1 to S3: one write on repositories of growing size, a batch of
// 200 operations, and the export of the largest repository.

import assert from 'node:assert/strict'
import { readCarWithRoot } from '@atproto/repo'
import { SCALE, memoryGrowth, saveResults, summary, timed } from '../../src/lib/perf.ts'
import { scenario } from '../../src/scenario.ts'
import type { Account, Scenario } from '../../src/scenario.ts'

const NOTE = 'com.example.parity.note'
const SIZES = SCALE === 'full' ? [10, 1_000, 10_000, 100_000] : [10, 400, 2_000]
const SAMPLES = SCALE === 'full' ? 100 : 20

async function blocks(s: Scenario, account: Account): Promise<{ rows: number; bytes: number }> {
  const [row] = await s.target.sqlite({ actor: account.did }, 'select count(*) as rows, coalesce(sum(length(content)), 0) as bytes from repo_block')
  return { rows: Number(row!.rows), bytes: Number(row!.bytes) }
}

scenario('01-repository', { timeoutMs: 3 * 60 * 60 * 1000 }, async (s) => {
  const alice = await s.createAccount('alice', {}, { silent: true })
  let written = 0
  const record = (n: number) => ({ $type: NOTE, n, text: `record ${n} of a repository that grows` })
  const batch = (count: number) => {
    const writes = Array.from({ length: count }, () => ({ $type: 'com.atproto.repo.applyWrites#create', collection: NOTE, rkey: `r${String(written).padStart(7, '0')}`, value: record(written++) }))
    return s.http('seed', { method: 'POST', path: '/xrpc/com.atproto.repo.applyWrites', json: { repo: alice.did, writes }, auth: alice, silent: true })
  }
  const single = () => {
    const n = written++
    return s.http('write', { method: 'POST', path: '/xrpc/com.atproto.repo.createRecord', json: { repo: alice.did, collection: NOTE, rkey: `r${String(n).padStart(7, '0')}`, record: record(n) }, auth: alice, silent: true })
  }

  const s1: Record<string, unknown>[] = []
  const s2: Record<string, unknown>[] = []
  for (const size of SIZES) {
    while (written < size) {
      const response = await batch(Math.min(200, size - written))
      assert.equal(response.status, 200, response.text)
    }
    // S1: single creates at this size.
    const before = await blocks(s, alice)
    const latencies: number[] = []
    for (let i = 0; i < SAMPLES; i++) {
      const [ms, response] = await timed(single)
      assert.equal(response.status, 200, response.text)
      latencies.push(ms)
    }
    const after = await blocks(s, alice)
    s1.push({ records: size, ...summary(latencies), blockRowsPerWrite: Math.round(((after.rows - before.rows) / SAMPLES) * 10) / 10, blockBytesPerWrite: Math.round((after.bytes - before.bytes) / SAMPLES) })
    // S2: batches of 200 at this size.
    const batches: number[] = []
    for (let i = 0; i < 5; i++) {
      const [ms, response] = await timed(() => batch(200))
      assert.equal(response.status, 200, response.text)
      batches.push(ms)
    }
    s2.push({ records: size, ...summary(batches) })
  }

  // S3: export of the largest repository.
  const [growth, [ms, exported]] = await memoryGrowth(s, () => timed(() => s.http('export', { path: '/xrpc/com.atproto.sync.getRepo', query: { did: alice.did }, silent: true })))
  assert.equal(exported.status, 200)
  const car = await readCarWithRoot(exported.bytes)
  const s3 = { records: written, wallMs: Math.round(ms), carMiB: Math.round((exported.bytes.byteLength / (1024 * 1024)) * 10) / 10, blocks: car.blocks.size, memoryGrowthMiB: growth }

  saveResults(s, 'S1-S3', { S1: s1, S2: s2, S3: s3 })
})
