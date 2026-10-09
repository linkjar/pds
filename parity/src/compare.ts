// Oracle O1 across two targets: compares the transcripts of the same
// scenarios. Two boots of the same image must show no difference, which is
// the check on the normaliser itself. Reference against Stock Reference must
// show exactly the differences recorded in `differences/`.

import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { PROFILES } from './stack/env.ts'
import { PARITY_DIR, targetSpec } from './stack/targets.ts'
import type { TargetName } from './stack/targets.ts'

type Transcript = { scenario: string; profile: string; outcome: string; entries: Record<string, unknown>[] }

export type Difference = {
  scenario: string
  step: string
  /** Where in the entry the values differ. */
  path: string
  left: unknown
  right: unknown
}

function transcripts(target: TargetName): Map<string, Transcript> {
  const out = new Map<string, Transcript>()
  // The performance profile records measurements, which no two runs share.
  for (const profile of PROFILES.filter((name) => name !== 'perf')) {
    const dir = join(targetSpec(target).runDir, 'transcripts', profile)
    if (!existsSync(dir)) continue
    for (const file of readdirSync(dir).filter((name) => name.endsWith('.json')).sort()) {
      const transcript: Transcript = JSON.parse(readFileSync(join(dir, file), 'utf8'))
      out.set(`${profile}/${transcript.scenario}`, transcript)
    }
  }
  return out
}

function walk(left: unknown, right: unknown, path: string, out: { path: string; left: unknown; right: unknown }[]): void {
  if (JSON.stringify(left) === JSON.stringify(right)) return
  const bothObjects = left !== null && right !== null && typeof left === 'object' && typeof right === 'object'
  if (!bothObjects || Array.isArray(left) !== Array.isArray(right)) {
    out.push({ path, left, right })
    return
  }
  if (Array.isArray(left) && Array.isArray(right)) {
    const length = Math.max(left.length, right.length)
    for (let i = 0; i < length; i++) walk(left[i], right[i], `${path}[${i}]`, out)
    return
  }
  const a = left as Record<string, unknown>
  const b = right as Record<string, unknown>
  for (const key of [...new Set([...Object.keys(a), ...Object.keys(b)])].sort()) {
    walk(a[key], b[key], path ? `${path}.${key}` : key, out)
  }
}

export function diffTranscripts(left: Map<string, Transcript>, right: Map<string, Transcript>): Difference[] {
  const differences: Difference[] = []
  for (const key of [...new Set([...left.keys(), ...right.keys()])].sort()) {
    const a = left.get(key)
    const b = right.get(key)
    if (!a || !b) {
      differences.push({ scenario: key, step: '(scenario)', path: 'present', left: Boolean(a), right: Boolean(b) })
      continue
    }
    if (a.outcome !== b.outcome) {
      differences.push({ scenario: key, step: '(scenario)', path: 'outcome', left: a.outcome, right: b.outcome })
    }
    // A scenario that one target skipped is one difference, not one for each step the other target took.
    if (a.outcome.startsWith('skipped') || b.outcome.startsWith('skipped')) continue
    const length = Math.max(a.entries.length, b.entries.length)
    for (let i = 0; i < length; i++) {
      const found: { path: string; left: unknown; right: unknown }[] = []
      walk(a.entries[i], b.entries[i], '', found)
      const step = String(a.entries[i]?.step ?? b.entries[i]?.step)
      for (const item of found) differences.push({ scenario: key, step, ...item })
    }
  }
  return differences
}

export type CompareOptions = {
  /** Rewrite the recorded difference list instead of checking against it. */
  update?: boolean
}

/** Returns the process exit status. */
export function compare(left: TargetName, right: TargetName, options: CompareOptions = {}): number {
  const a = transcripts(left)
  const b = transcripts(right)
  if (a.size === 0 || b.size === 0) {
    console.error(`No transcripts for ${a.size === 0 ? left : right}. Run: pnpm parity run <target>`)
    return 2
  }
  const failed = [...a, ...b].filter(([, transcript]) => transcript.outcome.startsWith('failed'))
  for (const [key, transcript] of failed) console.error(`${key}: ${transcript.outcome}`)
  const differences = diffTranscripts(a, b)
  const file = join(PARITY_DIR, 'differences', `${left}-vs-${right}.json`)
  const expected: Difference[] = existsSync(file) ? JSON.parse(readFileSync(file, 'utf8')) : []
  const render = (list: Difference[]) => JSON.stringify(list, null, 2) + '\n'

  if (options.update) {
    mkdirSync(dirname(file), { recursive: true })
    writeFileSync(file, render(differences))
    console.log(`${differences.length} difference(s) between ${left} and ${right} written to ${file}`)
    return failed.length > 0 ? 1 : 0
  }
  const key = (d: Difference) => JSON.stringify(d)
  const known = new Set(expected.map(key))
  const seen = new Set(differences.map(key))
  const unexpected = differences.filter((d) => !known.has(key(d)))
  const missing = expected.filter((d) => !seen.has(key(d)))
  console.log(
    `${a.size} scenario(s), ${left} against ${right}: ${differences.length} difference(s), ${expected.length} recorded, ` +
      `${unexpected.length} unexpected, ${missing.length} recorded but not seen`,
  )
  for (const d of unexpected) {
    console.log(`\n+ ${d.scenario} · ${d.step} · ${d.path}\n    ${left}: ${JSON.stringify(d.left)}\n    ${right}: ${JSON.stringify(d.right)}`)
  }
  for (const d of missing) console.log(`\n- recorded but not seen: ${d.scenario} · ${d.step} · ${d.path}`)
  return unexpected.length === 0 && missing.length === 0 && failed.length === 0 ? 0 : 1
}
