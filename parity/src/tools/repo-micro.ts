// The single-write, batch-write and export scenarios of SPEC 16.1 as
// micro-benchmarks of the Reference's repository library, `@atproto/repo` at
// the pin, with a memory block store. The same sequence and the same records
// as `scenarios/perf/01-repository` and as `bench/src/bin/repo-bench.rs`, so
// the three sets of numbers line up.
//
//   node src/tools/repo-micro.ts [full|smoke] [out.json]
//
// One sample is the work the PDS does in the library for one write: load
// the repository at its head, format the commit (encode the records, change
// the tree, compare with the old tree, collect the proof, sign), write the
// CAR of the firehose blocks, and apply the commit to the store. HTTP,
// SQLite, validation and the sequencer are not in it.

import { writeFileSync } from 'node:fs'
import { performance } from 'node:perf_hooks'
import { Secp256k1Keypair } from '@atproto/crypto'
import { MemoryBlockstore, Repo, WriteOpAction, blocksToCarFile, getFullRepo } from '@atproto/repo'
import type { RecordWriteOp } from '@atproto/repo'

const NOTE = 'com.example.parity.note'
const DID = 'did:plc:ewvi7nxzyoun6zhxrhs64oiz'
const KEY_HEX = '9085d2bef69286a6cbb51623c8fa258629945cd55ca705cc4e66700396894e0c'

const full = (process.argv[2] ?? 'full') === 'full'
const sizes = full ? [10, 1_000, 10_000, 100_000] : [10, 400, 2_000]
const samples = full ? 100 : 20

const percentile = (values: number[], p: number): number => {
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1)]!
}
const round = (ms: number) => Math.round(ms * 1000) / 1000

const keypair = await Secp256k1Keypair.import(KEY_HEX, { exportable: true })
const storage = new MemoryBlockstore()
let repo: Repo | null = null
let written = 0

type Sample = { ms: number; eventCarBytes: number; newBlocks: number; removedBlocks: number }

async function write(count: number): Promise<Sample> {
  const ops = Array.from({ length: count }, () => {
    const n = written++
    return {
      action: WriteOpAction.Create,
      collection: NOTE,
      rkey: `r${String(n).padStart(7, '0')}`,
      record: { $type: NOTE, n, text: `record ${n} of a repository that grows` },
    } as RecordWriteOp
  })
  const start = performance.now()
  const commit = repo
    ? await (await Repo.load(storage, repo.cid)).formatCommit(ops, keypair)
    : await Repo.formatInitCommit(storage, DID, keypair, ops as never)
  const event = await blocksToCarFile(commit.cid, commit.relevantBlocks)
  await storage.applyCommit(commit)
  const ms = performance.now() - start
  repo = await Repo.load(storage, commit.cid)
  return { ms, eventCarBytes: event.byteLength, newBlocks: commit.newBlocks.size, removedBlocks: commit.removedCids.size() }
}

const started = performance.now()
const s1: unknown[] = []
const s2: unknown[] = []
for (const size of sizes) {
  while (written < size) await write(Math.min(200, size - written))
  let at = written
  const singles: Sample[] = []
  for (let i = 0; i < samples; i++) singles.push(await write(1))
  const mean = (pick: (s: Sample) => number) => singles.reduce((sum, s) => sum + pick(s), 0) / singles.length
  s1.push({
    size,
    recordsAtStart: at,
    p50Ms: round(percentile(singles.map((s) => s.ms), 50)),
    p99Ms: round(percentile(singles.map((s) => s.ms), 99)),
    eventCarBytesPerWrite: Math.round(mean((s) => s.eventCarBytes)),
    newBlocksPerWrite: mean((s) => s.newBlocks),
    removedBlocksPerWrite: mean((s) => s.removedBlocks),
  })
  at = written
  const batches: Sample[] = []
  for (let i = 0; i < 5; i++) batches.push(await write(200))
  s2.push({ size, recordsAtStart: at, p50Ms: round(percentile(batches.map((s) => s.ms), 50)), maxMs: round(Math.max(...batches.map((s) => s.ms))) })
  console.error(`@atproto/repo: size ${size} done at ${((performance.now() - started) / 1000).toFixed(1)}s`)
}

// The export scenario: three exports, bytes counted and dropped as a socket would take them.
const exports: { ms: number; bytes: number }[] = []
for (let i = 0; i < 3; i++) {
  const start = performance.now()
  let bytes = 0
  for await (const chunk of getFullRepo(storage, repo!.cid)) bytes += chunk.byteLength
  exports.push({ ms: performance.now() - start, bytes })
}
const best = exports.reduce((a, b) => (b.ms < a.ms ? b : a))

const report = {
  engine: '@atproto/repo 0.10.14 (Node ' + process.versions.node + ')',
  scale: full ? 'full' : 'smoke',
  S1: s1,
  S2: s2,
  S3: {
    records: written,
    wallMs: round(best.ms),
    wallMsAll: exports.map((e) => round(e.ms)),
    carBytes: best.bytes,
    carMiB: Math.round((best.bytes / (1024 * 1024)) * 10) / 10,
    blocks: storage.blocks.size,
  },
  treeRoot: repo!.commit.data.toString(),
  totalSeconds: Math.round((performance.now() - started) / 100) / 10,
}
const text = JSON.stringify(report, null, 2)
if (process.argv[3]) writeFileSync(process.argv[3], `${text}\n`)
console.log(text)
