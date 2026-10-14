// The writes run: the Candidate takes over a data directory that the
// Reference started, and must answer every write and serve the firehose as
// the Reference does on a directory of its own (SPEC 17.3: the response
// comparison, the repository CID check and the firehose check).
//
// The Candidate has no account endpoint yet, so every pass starts on the
// Reference, which creates a pool of accounts (scenarios/writes/00-pool).
// A pass is one fresh stack:
//
//   1. The Reference image starts and creates the pool.
//   2. In the Candidate's pass, the PDS container is replaced by the
//      Candidate image on the same data directory.
//   3. The scenarios of the stage run. They take their accounts from the
//      pool.
//
// Each stage runs one pass on each server, and the two sets of transcripts
// are compared. The differences that may show are the deliberate ones of
// SPEC 2.3, recorded in differences/reference-vs-candidate.json. After the
// Candidate's pass its `verify` command checks the data directory.
//
// A last pass has no counterpart on the Reference: the Candidate stops
// between a commit and its event, and repairs the log at the next start
// (SPEC 7.4).

import { cpSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { compareGroups } from './compare.ts'
import { writesReport } from './writes-report.ts'
import { runFiles, saveLogs } from './run.ts'
import { down, execPds, swapPds, up } from './stack/compose.ts'
import type { Profile } from './stack/env.ts'
import { PARITY_DIR, targetSpec } from './stack/targets.ts'

export type WritesOptions = {
  /** Rewrite the recorded difference list instead of checking against it. */
  update?: boolean
  /** Leave the last stack running afterwards. */
  keep?: boolean
  /** Run only the stages whose name contains this text. */
  only?: string
  /**
   * Run the write measurements instead of the scenarios: the single-write,
   * batch-write and export scenarios of SPEC 16.1, on each server
   * (scenarios/perf/01-repository).
   */
  perf?: boolean
}

const scenario = (path: string) => join(PARITY_DIR, 'scenarios', `${path}.scenario.ts`)
const POOL = scenario('writes/00-pool')

type Stage = {
  name: string
  profile: Profile
  files: string[]
  /** Scenarios that run on the Reference before the takeover, in both passes. */
  seed?: string[]
  /** The transcript groups, when the stage is compared on its own. */
  groups?: [string, string]
}

const STAGES: Stage[] = [
  {
    name: 'records',
    profile: 'writes',
    files: ['default/02-sessions', 'default/03-repo-writes', 'default/04-repo-reads', 'default/05-blobs', 'default/15-import', 'writes/20-firehose', 'writes/21-relay', 'writes/22-elsewhere', 'writes/24-create-account'].map(scenario),
  },
  // The two rows of SPEC 7.3 that depend on configured limits.
  { name: 'cursor-limits', profile: 'writes-firehose', files: [scenario('firehose/01-cursor-limits')] },
]

const REFERENCE = 'writes-reference'
const CANDIDATE = 'writes-candidate'

// Blobs in the S3-compatible store (SPEC 8.1). The Reference stores a blob
// before the takeover, so the Candidate reads and deletes an object that
// the Reference wrote. The stage has its own difference list: the blob
// scenario runs in the `records` stage on the disk store as well.
const S3: Stage = {
  name: 's3',
  profile: 'writes-s3',
  seed: [scenario('writes/01-seed-blob')],
  files: ['default/05-blobs', 'writes/23-seeded-blob'].map(scenario),
  groups: ['writes-s3-reference', 'writes-s3-candidate'],
}
STAGES.push(S3)
const CRASH = 'writes-crash'

const PERF: Stage = { name: 'perf', profile: 'writes-perf', files: [scenario('perf/01-repository')] }

/** Returns the process exit status. */
export async function writes(options: WritesOptions = {}): Promise<number> {
  const target = 'candidate' as const
  const spec = targetSpec(target)
  const reference = targetSpec('reference').image
  if (options.perf) return measure(options)
  const stages = STAGES.filter((stage) => !options.only || stage.name.includes(options.only))
  const crash = !options.only || 'crash'.includes(options.only)
  if (!options.only) {
    for (const group of [REFERENCE, CANDIDATE, CRASH, ...(S3.groups ?? [])]) rmSync(join(spec.runDir, 'transcripts', group), { recursive: true, force: true })
  }

  /** One fresh stack: the pool on the Reference, then the scenarios on one of the two servers. */
  const pass = async (label: string, profile: Profile, files: string[], group: string, takeover?: Profile, seed: string[] = []): Promise<number> => {
    console.log(`\n── writes: ${label}`)
    await up(target, profile, { fresh: true, image: reference })
    let status = await runFiles(target, [POOL, ...seed], { PARITY_TRANSCRIPTS: `${group}-pool` })
    if (status !== 0) return status
    if (takeover) swapPds(target, spec.image, takeover)
    status = await runFiles(target, files, { PARITY_TRANSCRIPTS: group, PARITY_ACCOUNT_POOL: '1' })
    if (status === 0 && takeover) status = verify(spec.runDir, label)
    return status
  }

  let status = 0
  try {
    for (const stage of stages) {
      const [left, right] = stage.groups ?? [REFERENCE, CANDIDATE]
      status = await pass(`${stage.name}, on the Reference`, stage.profile, stage.files, left, undefined, stage.seed)
      if (status === 0) status = await pass(`${stage.name}, on the Candidate`, stage.profile, stage.files, right, stage.profile, stage.seed)
      if (status !== 0) {
        console.error(`writes: stage "${stage.name}" failed`)
        break
      }
    }
    if (status === 0 && crash) {
      status = await pass('the crash boundary, on the Candidate', 'writes', [scenario('writes-crash/30-crash')], CRASH, 'writes-crash')
      if (status !== 0) console.error('writes: the crash boundary failed')
    }
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    console.error(`writes: a step threw. Service logs are in ${saveLogs(target)}`)
    status = 1
  } finally {
    if (!options.keep) down(target)
  }
  if (status !== 0) return status
  console.log('\n── writes: compare')
  // A stage with groups of its own has a difference list of its own.
  if (stages.some((stage) => !stage.groups)) {
    status = compareGroups(target, REFERENCE, CANDIDATE, {
      update: options.update,
      label: 'writes',
      hint: 'pnpm parity writes',
      names: ['reference', 'candidate'],
    })
  }
  for (const stage of stages) {
    if (!stage.groups) continue
    const result = compareGroups(target, stage.groups[0], stage.groups[1], {
      update: options.update,
      label: `writes-${stage.name}`,
      hint: `pnpm parity writes --only=${stage.name}`,
      names: [`reference-${stage.name}`, `candidate-${stage.name}`],
    })
    status = status || result
  }
  return status
}

/**
 * The write measurements: one fresh stack for each server, the same
 * scenario on each, and one page with both (results/perf-writes.md).
 */
async function measure(options: WritesOptions): Promise<number> {
  const target = 'candidate' as const
  const spec = targetSpec(target)
  const reference = targetSpec('reference').image
  let status = 0
  try {
    for (const [server, takeover] of [['reference', false], ['candidate', true]] as const) {
      console.log(`\n── writes: measurements, on the ${server === 'reference' ? 'Reference' : 'Candidate'}`)
      await up(target, PERF.profile, { fresh: true, image: reference })
      status = await runFiles(target, [POOL], { PARITY_TRANSCRIPTS: 'writes-perf-pool' })
      if (status !== 0) break
      if (takeover) swapPds(target, spec.image, PERF.profile)
      rmSync(join(spec.runDir, 'perf'), { recursive: true, force: true })
      status = await runFiles(target, PERF.files, { PARITY_TRANSCRIPTS: `writes-perf-${server}`, PARITY_ACCOUNT_POOL: '1' })
      if (status !== 0) break
      // Both passes write to the same place; keep each server's results.
      cpSync(join(spec.runDir, 'perf'), join(spec.runDir, `perf-writes-${server}`), { recursive: true })
      if (takeover) status = verify(spec.runDir, 'measurements')
    }
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    console.error(`writes: a step threw. Service logs are in ${saveLogs(target)}`)
    status = 1
  } finally {
    if (!options.keep) down(target)
  }
  if (status === 0) console.log(`\n${writesReport()}`)
  return status
}

/** Runs the Candidate's `verify` on the data directory it serves. Zero differences are required (SPEC 18.2). */
function verify(runDir: string, label: string): number {
  const run = execPds('candidate', ['linkjar-pds', 'verify', '--json'])
  writeFileSync(join(runDir, `verify-${label.replace(/\W+/g, '-')}.json`), run.stdout)
  let report: { differences?: number; findings?: unknown[] }
  try {
    report = JSON.parse(run.stdout)
  } catch {
    console.error(`verify did not report:\n${run.stdout}${run.stderr}`)
    return 1
  }
  console.log(JSON.stringify({ ...report, findings: undefined }))
  if (report.differences !== 0) {
    console.error(JSON.stringify(report.findings, null, 2))
    return 1
  }
  return run.status
}
