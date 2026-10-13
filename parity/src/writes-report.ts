// The write measurements of `pnpm parity writes --perf` as one Markdown
// page: SPEC 16.1, S1 to S3, on the Reference and on the Candidate.

import { readFileSync, writeFileSync } from 'node:fs'
import { arch, cpus, platform, release, totalmem } from 'node:os'
import { join } from 'node:path'
import { PARITY_DIR, targetSpec, upstream } from './stack/targets.ts'

type S1 = { records: number; n: number; p50: number; p99: number; max: number; carBytesPerWrite: number; netBlockRowsPerWrite: number; netBlockBytesPerWrite: number }
type S2 = { records: number; n: number; p50: number; p99: number; max: number }
type S3 = { records: number; wallMs: number; carMiB: number; blocks: number; memoryGrowthMiB: number }
type Saved = { scale: string; image: string; measuredAt: string; results: { S1: S1[]; S2: S2[]; S3: S3 } }

const table = (head: string[], rows: (string | number)[][]) =>
  [`| ${head.join(' | ')} |`, `|${head.map(() => '---').join('|')}|`, ...rows.map((row) => `| ${row.join(' | ')} |`)].join('\n')

/** Writes parity/results/perf-writes.md from the last run and returns its text. */
export function writesReport(): string {
  const runDir = targetSpec('candidate').runDir
  const load = (server: string): Saved => JSON.parse(readFileSync(join(runDir, `perf-writes-${server}`, 'S1-S3.json'), 'utf8'))
  const reference = load('reference')
  const candidate = load('candidate')
  const r = reference.results
  const c = candidate.results
  const count = (n: number) => n.toLocaleString('en-US')
  const ratio = (a: number, b: number) => (b > 0 ? `${Math.round((a / b) * 10) / 10}×` : '')
  const out = [
    '# Write measurements: the Reference and the Candidate',
    '',
    'Measured by `pnpm parity writes --perf`: SPEC §16.1, S1 to S3. Each server runs the same scenario on a stack of its own. The Reference creates the account; in the Candidate\'s pass the Candidate then takes over the data directory and does every write. Latencies are wall-clock times at the client, through the TLS edge. Memory is the anonymous memory of the container.',
    '',
    'One run on the host below. It places the two servers beside each other on that host; it is not a benchmark of either. The requests are sequential, so the numbers say nothing about throughput under load.',
    '',
    table(
      ['', ''],
      [
        ['Reference', `\`${reference.image}\``],
        ['Upstream', `\`${upstream.tag}\` at \`${upstream.commit.slice(0, 8)}\``],
        ['Candidate', `\`${candidate.image}\``],
        ['Scale', candidate.scale],
        ['Measured', candidate.measuredAt.slice(0, 10)],
        ['Host', `${cpus()[0]?.model ?? 'unknown CPU'}, ${cpus().length} cores, ${Math.round(totalmem() / 2 ** 30)} GiB, ${platform()} ${release()} ${arch()}, Docker`],
      ],
    ),
    '',
    `## S1: one \`createRecord\`, p50 / p99 ms over ${r.S1[0]?.n ?? 0} writes`,
    '',
    table(
      ['Records in the repository', 'Reference', 'Candidate', 'Reference ÷ Candidate at p50', 'Bytes of new blocks per write, Reference', 'Candidate'],
      r.S1.map((row, i) => {
        const other = c.S1[i]!
        return [count(row.records), `${row.p50} / ${row.p99}`, `${other.p50} / ${other.p99}`, ratio(row.p50, other.p50), count(row.carBytesPerWrite), count(other.carBytesPerWrite)]
      }),
    ),
    '',
    'The bytes of new blocks are the CAR slice of the commit on the firehose: the commit, the tree nodes on the path to the record, and the record. The two servers build the same tree, so the same writes give the same slices.',
    '',
    '## S2: `applyWrites` with 200 operations, p50 / max ms over five batches',
    '',
    table(
      ['Records in the repository', 'Reference', 'Candidate', 'Reference ÷ Candidate at p50'],
      r.S2.map((row, i) => {
        const other = c.S2[i]!
        return [count(row.records), `${row.p50} / ${row.max}`, `${other.p50} / ${other.max}`, ratio(row.p50, other.p50)]
      }),
    ),
    '',
    '## S3: `getRepo` export',
    '',
    table(
      ['', 'Reference', 'Candidate', 'Reference ÷ Candidate'],
      [
        ['Records', count(r.S3.records), count(c.S3.records), ''],
        ['Wall ms', count(r.S3.wallMs), count(c.S3.wallMs), ratio(r.S3.wallMs, c.S3.wallMs)],
        ['CAR MiB', r.S3.carMiB, c.S3.carMiB, ''],
        ['Blocks', count(r.S3.blocks), count(c.S3.blocks), ''],
        ['Memory growth MiB', r.S3.memoryGrowthMiB, c.S3.memoryGrowthMiB, ''],
      ],
    ),
    '',
  ].join('\n')
  writeFileSync(join(PARITY_DIR, 'results', 'perf-writes.md'), out)
  return out
}
