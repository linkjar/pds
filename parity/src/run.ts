// Runs the scenarios of one profile against one target.

import { spawn } from 'node:child_process'
import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { FixtureServer } from './fixtures/server.ts'
import { down, logs, readState, up } from './stack/compose.ts'
import type { Profile } from './stack/env.ts'
import { PARITY_DIR, targetSpec } from './stack/targets.ts'
import type { TargetName } from './stack/targets.ts'

export type RunOptions = {
  profile: Profile
  /** Run only scenarios whose file name contains this text, or one of several separated by commas. */
  only?: string
  /** Leave the stack running afterwards, for inspection. */
  keep?: boolean
  /** Use the stack that is already running instead of a fresh one. */
  reuse?: boolean
}

export function scenarioFiles(profile: Profile, only?: string): string[] {
  const dir = join(PARITY_DIR, 'scenarios', profile)
  if (!existsSync(dir)) return []
  return readdirSync(dir)
    .filter((name) => name.endsWith('.scenario.ts'))
    .filter((name) => !only || only.split(',').some((part) => name.includes(part)))
    .sort()
    .map((name) => join(dir, name))
}

export async function runScenarios(target: TargetName, options: RunOptions): Promise<number> {
  const files = scenarioFiles(options.profile, options.only)
  if (files.length === 0) {
    console.error(`No scenarios for profile ${options.profile}${options.only ? ` matching "${options.only}"` : ''}`)
    return 2
  }
  if (options.reuse) {
    const state = readState(target)
    if (state.profile !== options.profile) throw new Error(`The running ${target} stack has profile ${state.profile}`)
  } else {
    // A fresh stack for every run: scenarios use fixed handles, and state
    // left by an earlier run would change what they observe.
    await up(target, options.profile, { fresh: true })
    // A full run replaces the profile's transcripts. A partial run replaces only those it writes.
    if (!options.only) rmSync(join(targetSpec(target).runDir, 'transcripts', options.profile), { recursive: true, force: true })
  }
  const fixtures = new FixtureServer(targetSpec(target).fixturePort)
  await fixtures.start()
  // One file at a time, in name order, so that both targets see the same
  // sequence of writes and the firehose scenarios see a quiet server.
  const child = spawn(process.execPath, ['--test', '--test-concurrency=1', '--test-reporter=spec', ...files], {
    cwd: PARITY_DIR,
    env: { ...process.env, PARITY_TARGET: target },
    stdio: 'inherit',
  })
  const status = await new Promise<number>((resolve) => child.on('exit', (code) => resolve(code ?? 1)))
  await fixtures.stop()
  if (status !== 0) {
    // The stack is about to go; keep what its services said.
    const dir = join(targetSpec(target).runDir, 'logs')
    mkdirSync(dir, { recursive: true })
    for (const service of ['pds', 'relay', 'plc', 'edge']) writeFileSync(join(dir, `${service}.log`), logs(target, service, 2000))
    console.error(`Scenarios failed. Service logs are in ${dir}`)
  }
  if (!options.keep && !options.reuse) down(target)
  return status
}
