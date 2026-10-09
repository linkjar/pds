// Collects the measurements of a performance run into one Markdown table.

import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { arch, cpus, platform, release, totalmem } from 'node:os'
import { join } from 'node:path'
import { PARITY_DIR, targetSpec, upstream } from './stack/targets.ts'
import type { TargetName } from './stack/targets.ts'

type Latency = { n: number; p50: number; p99: number; max: number }
type Saved = { scale: string; image: string; measuredAt: string; results: any }

const ms = (l: Latency) => (l.n === 0 ? 'none' : `${l.p50} / ${l.p99}`)
/** The writes that were not answered with 200, by status. */
const failures = (failed: Record<string, number> = {}) => {
  const parts = Object.entries(failed).map(([status, count]) => `${count} × ${status}`)
  return parts.length === 0 ? '' : ` (${parts.join(', ')})`
}
const table = (head: string[], rows: (string | number)[][]) =>
  [`| ${head.join(' | ')} |`, `|${head.map(() => '---').join('|')}|`, ...rows.map((row) => `| ${row.join(' | ')} |`)].join('\n')

export function perfReport(target: TargetName): string {
  const dir = join(targetSpec(target).runDir, 'perf')
  if (!existsSync(dir)) throw new Error(`No measurements for ${target}. Run: pnpm parity run ${target} --profile perf`)
  const saved: Record<string, Saved> = {}
  for (const file of readdirSync(dir).filter((name) => name.endsWith('.json'))) saved[file.replace('.json', '')] = JSON.parse(readFileSync(join(dir, file), 'utf8'))
  const first = Object.values(saved)[0]
  if (!first) throw new Error(`No measurements in ${dir}`)
  const repo = saved['S1-S3']?.results
  const fire = saved['S4-S5-S9']?.results
  const uploads = saved.S8?.results
  const out: string[] = []
  out.push(`# Performance baseline: ${target}`, '')
  out.push(
    `SPEC §16.1 scenarios S1 to S5, S8 and S9, measured by the parity harness from outside the server. Latencies are wall-clock times at the client, through the TLS edge. Memory is the anonymous memory of the PDS container, without the page cache.`,
    '',
    `One run on the host below. It places the Reference for the comparison with the Candidate on the same host (SPEC §16.2, T1); it is not a benchmark of the Reference.`,
    '',
  )
  out.push(
    table(
      ['', ''],
      [
        ['Image', `\`${first.image}\``],
        ['Upstream', `\`${upstream.tag}\` at \`${upstream.commit.slice(0, 8)}\``],
        ['Scale', first.scale === 'full' ? 'full' : `${first.scale} (not a baseline)`],
        ['Measured', first.measuredAt.slice(0, 10)],
        ['Host', `${cpus()[0]?.model.trim() ?? 'unknown CPU'}, ${cpus().length} cores, ${Math.round(totalmem() / 2 ** 30)} GiB, ${platform()} ${release()} ${arch()}, Docker`],
      ],
    ),
    '',
  )
  if (repo) {
    out.push('## S1: one `createRecord`', '', table(['Records in the repository', 'p50 / p99 ms', 'Bytes of new blocks per write', 'Net block rows per write', 'Net block bytes per write'], repo.S1.map((r: any) => [r.records.toLocaleString('en'), ms(r), r.carBytesPerWrite.toLocaleString('en'), r.netBlockRowsPerWrite, r.netBlockBytesPerWrite.toLocaleString('en')])), '', 'The bytes of new blocks are the CAR slice of the commit on the firehose: the commit, the tree nodes on the path to the record, and the record. Net growth is what the block table keeps after the Reference deletes the nodes and the commit that the write replaced.', '')
    out.push('## S2: `applyWrites` with 200 operations', '', table(['Records in the repository', 'p50 / max ms'], repo.S2.map((r: any) => [r.records.toLocaleString('en'), `${r.p50} / ${r.max}`])), '')
    out.push('## S3: `getRepo` export', '', table(['Records', 'Wall ms', 'CAR MiB', 'Blocks', 'Memory growth MiB'], [[repo.S3.records.toLocaleString('en'), repo.S3.wallMs.toLocaleString('en'), repo.S3.carMiB, repo.S3.blocks.toLocaleString('en'), repo.S3.memoryGrowthMiB]]), '')
  }
  if (fire) {
    out.push('## S4: one writer at 50 commits a second', '', table(['Subscribers', 'Writes answered 200', 'Frames delivered', 'Lag p50 / p99 ms', 'Commit p50 / p99 ms', 'Memory MiB'], fire.S4.map((r: any) => [r.subscribers.toLocaleString('en'), `${r.commits} of ${r.attempted}${failures(r.failed)}`, `${r.delivered.toLocaleString('en')} of ${r.expected.toLocaleString('en')}`, ms(r.lagMs), ms(r.commitMs), r.memoryMiB])), '')
    out.push('Lag runs from the moment the write was sent to the moment a subscriber received its commit, so it includes the commit itself. At most twenty subscribers record arrivals; the others hold a connection and count frames.', '')
    if (fire.aftermath) {
      const a = fire.aftermath
      const row = (label: string, r: any) => [label, r.commitsPerSecond, r.failed, r.slowestMs.toLocaleString('en')]
      out.push(
        `### After the subscribers left`,
        '',
        `Write throughput of ${a.before.writers} writers, each writing as fast as it is answered, around the S4 run that ended with ${a.subscribersBefore.toLocaleString('en')} subscribers. Every subscriber had disconnected before the second and third measurement.`,
        '',
        table(['When', 'Commits per second', 'Failed writes', 'Slowest write ms'], [row('Before any subscriber', a.before), row('Right after they left', a.rightAfter), row('Fifteen seconds later', a.fifteenSecondsLater)]),
        '',
      )
    }
    out.push('## S5: backfill from a cursor', '', table(['Events', 'Wall ms', 'Events per second', 'Memory growth MiB'], [[fire.S5.events.toLocaleString('en'), fire.S5.wallMs.toLocaleString('en'), fire.S5.eventsPerSecond.toLocaleString('en'), fire.S5.memoryGrowthMiB]]), '')
    out.push('## S9: writers across actors, two commits a second each', '', table(['Writers', 'Writes answered 200', 'Lag p50 / p99 ms', 'Commit p50 / p99 ms', 'Sequencer fsyncs'], [[fire.S9.writers, `${fire.S9.commits.toLocaleString('en')} of ${fire.S9.attempted.toLocaleString('en')}${failures(fire.S9.failed)}`, ms(fire.S9.lagMs), ms(fire.S9.commitMs), 'not observable from outside the Reference']]), '')
  }
  if (uploads) {
    out.push('## S8: concurrent uploads of 5 MiB', '', table(['Uploads', 'Wall ms', 'Upload p50 / max ms', 'Memory growth MiB'], [[uploads.uploads, uploads.wallMs.toLocaleString('en'), `${uploads.uploadMs.p50} / ${uploads.uploadMs.max}`, uploads.memoryGrowthMiB]]), '')
  }
  out.push('S6 (idle accounts) and S7 (the token endpoint) are measured from unit 5 on. See the [harness README](../README.md#performance-scenarios).', '')
  const text = out.join('\n')
  const file = join(PARITY_DIR, 'results', `perf-${target}.md`)
  mkdirSync(join(PARITY_DIR, 'results'), { recursive: true })
  writeFileSync(file, text)
  return text
}
