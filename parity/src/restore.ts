// The restore run: the Candidate serves a data directory that the Reference
// wrote (SPEC 8.2, 18.1), and must answer every read as the Reference did.
//
// One stack, two servers, one after the other on the same volume:
//
//   1. The Reference image starts on a fresh stack and the seed scenarios
//      write accounts, records and blobs.
//   2. The read scenarios run against the Reference.
//   3. The PDS container is replaced by the Candidate image. Nothing else in
//      the stack changes, and the data directory is the one from step 1.
//   4. The same read scenarios run against the Candidate.
//   5. The Candidate's `verify` command checks the data directory.
//   6. The two sets of transcripts are compared.
//
// Because both servers read the same bytes, binary answers are compared by
// their hash and no difference is expected at all.

import { rmSync, writeFileSync } from 'node:fs'
import { basename, join } from 'node:path'
import { compareGroups } from './compare.ts'
import { runFiles, scenarioFiles } from './run.ts'
import { down, execPds, swapPds, up } from './stack/compose.ts'
import { CANDIDATE_TAG, targetSpec } from './stack/targets.ts'

export type RestoreOptions = {
  /** Leave the stack running afterwards, with the Candidate serving. */
  keep?: boolean
  /** Rewrite the recorded difference list instead of checking against it. */
  update?: boolean
}

/** Returns the process exit status. */
export async function restore(options: RestoreOptions = {}): Promise<number> {
  const target = 'candidate' as const
  // The health check of the stack depends on which image runs, and tells the
  // Candidate by this variable.
  process.env.PARITY_CANDIDATE_IMAGE ??= CANDIDATE_TAG
  const candidate = process.env.PARITY_CANDIDATE_IMAGE
  const reference = targetSpec('reference').image
  const runDir = targetSpec(target).runDir

  const files = scenarioFiles('restore')
  // Files that start with 0 write; all others only read.
  const seeds = files.filter((file) => basename(file).startsWith('0'))
  const reads = files.filter((file) => !basename(file).startsWith('0'))

  for (const group of ['restore-seed', 'restore-reference', 'restore-candidate']) {
    rmSync(join(runDir, 'transcripts', group), { recursive: true, force: true })
  }
  await up(target, 'restore', { fresh: true, image: reference })

  const steps: [string, () => Promise<number> | number][] = [
    ['seed, on the Reference', () => runFiles(target, seeds, { PARITY_TRANSCRIPTS: 'restore-seed' })],
    ['reads, on the Reference', () => runFiles(target, reads, { PARITY_TRANSCRIPTS: 'restore-reference' })],
    [
      'reads, on the Candidate',
      () => {
        swapPds(target, candidate)
        return runFiles(target, reads, { PARITY_TRANSCRIPTS: 'restore-candidate' })
      },
    ],
    ['verify', () => verify(runDir)],
    ['compare', () => compareGroups(target, 'restore-reference', 'restore-candidate', { update: options.update })],
  ]
  let status = 0
  for (const [name, step] of steps) {
    console.log(`\n── restore: ${name}`)
    status = await step()
    if (status !== 0) {
      console.error(`restore: "${name}" failed`)
      break
    }
  }
  if (!options.keep) down(target)
  return status
}

/** Runs the Candidate's `verify` on the data directory it serves. Zero differences are required (SPEC 18.2). */
function verify(runDir: string): number {
  const run = execPds('candidate', ['linkjar-pds', 'verify', '--json'])
  writeFileSync(join(runDir, 'verify.json'), run.stdout)
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
