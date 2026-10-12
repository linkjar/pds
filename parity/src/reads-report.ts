// The read measurements of `pnpm parity restore --perf` as one Markdown
// page: the Reference and the Candidate on the same data directory.

import { readFileSync, writeFileSync } from 'node:fs'
import { arch, cpus, platform, release, totalmem } from 'node:os'
import { join } from 'node:path'
import { PARITY_DIR, targetSpec, upstream } from './stack/targets.ts'

type Latency = { n: number; p50: number; p99: number; max: number }
type Reads = {
  records: number
  idleMiB: number
  export: { runs: number[]; medianMs: number; MiB: number; memoryGrowthMiB: number }
  getLatestCommit: Latency
  repoGetRecord: Latency
  syncGetRecord: Latency
  listRecords100: Latency
  describeRepo: Latency
}
type Saved = { image: string; measuredAt: string; results: Reads }

const table = (head: string[], rows: (string | number)[][]) =>
  [`| ${head.join(' | ')} |`, `|${head.map(() => '---').join('|')}|`, ...rows.map((row) => `| ${row.join(' | ')} |`)].join('\n')

/** Writes parity/results/perf-reads.md from the last run and returns its text. */
export function readsReport(): string {
  const dir = join(targetSpec('candidate').runDir, 'perf')
  const load = (group: string): Saved => JSON.parse(readFileSync(join(dir, `reads-${group}.json`), 'utf8'))
  const reference = load('restore-reference')
  const candidate = load('restore-candidate')
  const r = reference.results
  const c = candidate.results
  const runs = r.export.runs.length
  const ratio = (a: number, b: number) => (b > 0 ? `${Math.round((a / b) * 10) / 10}×` : '')
  const latency = (name: string, a: Latency, b: Latency) => [name, `${a.p50} / ${a.p99}`, `${b.p50} / ${b.p99}`, ratio(a.p50, b.p50)]
  const out = [
    '# Read measurements: the Reference and the Candidate on one data directory',
    '',
    'Measured by `pnpm parity restore --perf`. The Reference writes one repository and answers the requests below; the Candidate replaces it on the same volume and answers them again. Latencies are wall-clock times at the client, through the TLS edge, one request at a time. Memory is the anonymous memory of the PDS container, without the page cache. Each server is started fresh before its requests. Both servers returned the same export, byte for byte.',
    '',
    'One run on the host below. It places the two servers beside each other on that host; it is not a benchmark of either.',
    '',
    table(
      ['', ''],
      [
        ['Reference', `\`${reference.image}\``],
        ['Upstream', `\`${upstream.tag}\` at \`${upstream.commit.slice(0, 8)}\``],
        ['Candidate', `\`${candidate.image}\``],
        ['Records in the repository', r.records.toLocaleString('en-US')],
        ['Measured', candidate.measuredAt.slice(0, 10)],
        ['Host', `${cpus()[0]?.model ?? 'unknown CPU'}, ${cpus().length} cores, ${Math.round(totalmem() / 2 ** 30)} GiB, ${platform()} ${release()} ${arch()}, Docker`],
      ],
    ),
    '',
    '## The export: `sync.getRepo`',
    '',
    table(
      ['', 'Reference', 'Candidate', 'Reference ÷ Candidate'],
      [
        [`Median of ${runs} runs, ms`, r.export.medianMs.toLocaleString('en-US'), c.export.medianMs.toLocaleString('en-US'), ratio(r.export.medianMs, c.export.medianMs)],
        [`The ${runs} runs, ms`, r.export.runs.join(', '), c.export.runs.join(', '), ''],
        ['CAR MiB', r.export.MiB, c.export.MiB, ''],
        [`Memory growth over the ${runs} runs, MiB`, r.export.memoryGrowthMiB, c.export.memoryGrowthMiB, ''],
        ['Memory before the first request, MiB', r.idleMiB, c.idleMiB, ratio(r.idleMiB, c.idleMiB)],
      ],
    ),
    '',
    `## Point reads: p50 / p99 ms over ${r.getLatestCommit.n} requests`,
    '',
    table(
      ['Method', 'Reference', 'Candidate', 'Reference ÷ Candidate at p50'],
      [
        latency('`sync.getLatestCommit`', r.getLatestCommit, c.getLatestCommit),
        latency('`repo.getRecord`', r.repoGetRecord, c.repoGetRecord),
        latency('`sync.getRecord` (record with proof)', r.syncGetRecord, c.syncGetRecord),
        latency('`repo.listRecords`, 100 records', r.listRecords100, c.listRecords100),
        latency('`repo.describeRepo`', r.describeRepo, c.describeRepo),
      ],
    ),
    '',
    'The requests are sequential, so the numbers say nothing about throughput under load. The Candidate opens a SQLite connection for each read in this unit; the actor store cache of SPEC §8.3 arrives with the write path.',
    '',
  ].join('\n')
  writeFileSync(join(PARITY_DIR, 'results', 'perf-reads.md'), out)
  return out
}
