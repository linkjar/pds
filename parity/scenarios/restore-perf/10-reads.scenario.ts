// Read measurements on a restored data directory: the same requests, timed
// on the Reference and on the Candidate over the same bytes (SPEC 16.1, S3,
// and the point reads beside it). Timings go to a file, not to the
// transcript; the transcript holds what must be equal on both servers.

import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { memory, memoryGrowth, summary, timed } from '../../src/lib/perf.ts'
import { scenario } from '../../src/scenario.ts'
import type { Account } from '../../src/scenario.ts'
import { targetSpec } from '../../src/stack/targets.ts'

const NOTE = 'com.example.parity.note'
const MiB = 1024 * 1024

scenario('10-reads', { timeoutMs: 3_600_000 }, async (s) => {
  const seed: { big: Account; records: number } = JSON.parse(readFileSync(join(targetSpec(s.target.name).runDir, 'restore-perf-seed.json'), 'utf8'))
  const did = seed.big.did
  const quiet = { silent: true } as const
  // The same keys on both servers: a fixed stride through the repository.
  const keys = Array.from({ length: 200 }, (_, i) => `r${String((i * 7919) % seed.records).padStart(6, '0')}`)
  const idle = await memory(s)

  // The export: five runs, and the memory growth over them.
  const exports: number[] = []
  let size = 0
  let hash = ''
  const [growth] = await memoryGrowth(s, async () => {
    for (let run = 0; run < 5; run++) {
      const [ms, result] = await timed(() => s.query('getRepo', 'com.atproto.sync.getRepo', { did }, quiet))
      assert.equal(result.status, 200)
      exports.push(ms)
      size = result.bytes.byteLength
      hash = createHash('sha256').update(result.bytes).digest('hex')
    }
  })
  s.exact('the export', { bytes: size, sha256: hash })

  const each = async (nsid: string, params: (key: string) => Record<string, string | number>) => {
    const times: number[] = []
    for (const key of keys) {
      const [ms, result] = await timed(() => s.query(nsid, nsid, params(key), quiet))
      assert.equal(result.status, 200, result.text)
      times.push(ms)
    }
    return summary(times)
  }
  const results = {
    records: seed.records,
    idleMiB: Math.round((idle / MiB) * 10) / 10,
    export: { runs: exports.map((ms) => Math.round(ms)), medianMs: Math.round([...exports].sort((a, b) => a - b)[2]!), MiB: Math.round((size / MiB) * 10) / 10, memoryGrowthMiB: growth },
    getLatestCommit: await each('com.atproto.sync.getLatestCommit', () => ({ did })),
    repoGetRecord: await each('com.atproto.repo.getRecord', (rkey) => ({ repo: did, collection: NOTE, rkey })),
    syncGetRecord: await each('com.atproto.sync.getRecord', (rkey) => ({ did, collection: NOTE, rkey })),
    listRecords100: await each('com.atproto.repo.listRecords', (rkey) => ({ repo: did, collection: NOTE, limit: 100, cursor: rkey })),
    describeRepo: await each('com.atproto.repo.describeRepo', () => ({ repo: did })),
  }
  // One file for each server; the transcript says only that it was written.
  const dir = join(targetSpec(s.target.name).runDir, 'perf')
  mkdirSync(dir, { recursive: true })
  writeFileSync(join(dir, `reads-${process.env.PARITY_TRANSCRIPTS ?? 'unknown'}.json`), JSON.stringify({ image: s.target.state.image, measuredAt: new Date().toISOString(), results }, null, 2) + '\n')
  s.note('measurements', { recorded: true })
})
