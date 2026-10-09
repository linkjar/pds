// Starts, stops and inspects the compose stack of one target.

import { execFile, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { ensureSecrets, pdsEnv, writePdsEnv } from './env.ts'
import type { Profile, Secrets } from './env.ts'
import { COMPOSE_FILE, EDGE_NAMES, TARGET_NAMES, composeImages, targetSpec } from './targets.ts'
import type { TargetName, TargetSpec } from './targets.ts'
import { ensureTls } from './tls.ts'

/** What a scenario process needs to reach a running stack. */
export type StackState = {
  target: TargetName
  profile: Profile
  image: string
  linkjar: boolean
  edgePort: number
  fixturePort: number
  caFile: string
  secrets: Secrets
}

type RunResult = { status: number; stdout: string; stderr: string }

function composeEnv(spec: TargetSpec, secrets: Secrets): NodeJS.ProcessEnv {
  return {
    ...process.env,
    ...composeImages(),
    PARITY_TARGET: spec.name,
    PDS_IMAGE: spec.image,
    RUN_DIR: spec.runDir,
    EDGE_PORT: String(spec.edgePort),
    FIXTURE_PORT: String(spec.fixturePort),
    RELAY_ADMIN_PASSWORD: secrets.relayAdminPassword,
    PUBNET_SUBNET: `2001:db8:5eed:${TARGET_NAMES.indexOf(spec.name)}::/64`,
  }
}

/** Runs `docker compose` for a target. Throws on a non-zero exit unless `allowFailure`. */
export function compose(
  target: TargetName,
  args: string[],
  opts: { allowFailure?: boolean; inherit?: boolean; input?: string } = {},
): RunResult {
  const spec = targetSpec(target)
  const secrets = ensureSecrets(spec.runDir)
  const result = spawnSync('docker', ['compose', '-f', COMPOSE_FILE, ...args], {
    env: composeEnv(spec, secrets),
    encoding: 'utf8',
    input: opts.input,
    stdio: opts.inherit ? 'inherit' : ['pipe', 'pipe', 'pipe'],
    maxBuffer: 64 * 1024 * 1024,
  })
  const run = { status: result.status ?? 1, stdout: result.stdout ?? '', stderr: result.stderr ?? '' }
  if (run.status !== 0 && !opts.allowFailure) {
    throw new Error(`docker compose ${args.join(' ')} failed (${run.status})\n${run.stderr || run.stdout}`)
  }
  return run
}

export function stateFile(target: TargetName): string {
  return join(targetSpec(target).runDir, 'stack.json')
}

export function readState(target: TargetName): StackState {
  const file = stateFile(target)
  if (!existsSync(file)) {
    throw new Error(`No running stack for target ${target}. Run: pnpm parity stack up ${target}`)
  }
  return JSON.parse(readFileSync(file, 'utf8'))
}

/**
 * Brings the stack up with a profile. `fresh` removes the data volumes first,
 * which every scenario run does so that state never leaks between runs.
 */
export async function up(target: TargetName, profile: Profile, opts: { fresh?: boolean } = {}): Promise<StackState> {
  const spec = targetSpec(target)
  mkdirSync(spec.runDir, { recursive: true })
  const secrets = ensureSecrets(spec.runDir)
  const newTls = await ensureTls(spec.runDir, EDGE_NAMES)
  writePdsEnv(spec, pdsEnv(spec, profile, secrets))
  // Containers read the CA and the certificate at start, so new material means new containers.
  if (opts.fresh || newTls) compose(target, ['down', '--volumes', '--remove-orphans'], { allowFailure: true })
  ensurePlcImage(target)
  compose(target, ['up', '--detach', '--wait', '--wait-timeout', '240'], { inherit: true })
  const state: StackState = {
    target,
    profile,
    image: spec.image,
    linkjar: spec.linkjar,
    edgePort: spec.edgePort,
    fixturePort: spec.fixturePort,
    caFile: join(spec.runDir, 'ca/ca.crt'),
    secrets,
  }
  writeFileSync(stateFile(target), JSON.stringify(state, null, 2) + '\n', { mode: 0o600 })
  return state
}

/** Builds the PLC directory image from the pinned commit when it is absent. */
function ensurePlcImage(target: TargetName): void {
  const tag = `parity-plc:${composeImages().PLC_COMMIT}`
  const present = spawnSync('docker', ['image', 'inspect', tag], { stdio: 'ignore' }).status === 0
  if (!present) compose(target, ['build', 'plc'], { inherit: true })
}

export function down(target: TargetName): void {
  compose(target, ['down', '--volumes', '--remove-orphans'], { allowFailure: true })
  rmSync(stateFile(target), { force: true })
}

/** Restarts the PDS container and waits until it is healthy again. */
export function restartPds(target: TargetName): void {
  compose(target, ['restart', 'pds'])
  compose(target, ['up', '--detach', '--wait', '--wait-timeout', '120', 'pds'])
}

/** Runs a command inside the PDS container. */
export function execPds(target: TargetName, command: string[], input?: string): RunResult {
  return compose(target, ['exec', '-T', 'pds', ...command], { input, allowFailure: true })
}

/**
 * Starts a second, throwaway PDS container with some variables changed and
 * reports whether it stays up. It has its own data directory and no network
 * name, so the running PDS is not disturbed. Used for configurations that
 * must refuse to start.
 */
export function bootPds(target: TargetName, env: Record<string, string>, waitMs = 90_000): { started: boolean; output: string } {
  const spec = targetSpec(target)
  const secrets = ensureSecrets(spec.runDir)
  const name = `parity-${target}-boot-${Date.now()}`
  // /tmp exists and is writable in the image; the server does not create its data directory.
  const overrides = { PDS_DATA_DIRECTORY: '/tmp', PDS_BLOBSTORE_DISK_LOCATION: '/tmp/blocks', ...env }
  const args = ['compose', '-f', COMPOSE_FILE, 'run', '--detach', '--no-deps', '--name', name]
  for (const [key, value] of Object.entries(overrides)) args.push('-e', `${key}=${value}`)
  const options = { env: composeEnv(spec, secrets), encoding: 'utf8' as const }
  const launched = spawnSync('docker', [...args, 'pds'], options)
  if (launched.status !== 0) throw new Error(`could not start the boot container: ${launched.stderr}`)
  // The answer is one of two events: the process exits, or it serves its health route.
  const deadline = Date.now() + waitMs
  let outcome: 'exited' | 'healthy' | undefined
  while (outcome === undefined && Date.now() < deadline) {
    const state = spawnSync('docker', ['inspect', '--format', '{{.State.Running}}', name], { encoding: 'utf8' })
    if (state.stdout.trim() !== 'true') {
      outcome = 'exited'
      break
    }
    const health = spawnSync('docker', ['exec', name, 'node', '-e', "fetch('http://127.0.0.1:3000/xrpc/_health').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"], { encoding: 'utf8' })
    if (health.status === 0) outcome = 'healthy'
    else spawnSync('sleep', ['0.5'])
  }
  const output = spawnSync('docker', ['logs', name], { encoding: 'utf8' })
  spawnSync('docker', ['rm', '--force', '--volumes', name], { encoding: 'utf8' })
  if (outcome === undefined) throw new Error(`the boot container neither exited nor became healthy within ${waitMs} ms`)
  return { started: outcome === 'healthy', output: (output.stdout ?? '') + (output.stderr ?? '') }
}

export type Database = 'account' | 'sequencer' | 'did_cache' | { actor: string }

/** Runs one SQL statement against a database of the target's data directory. */
export function sqlite(target: TargetName, database: Database, sql: string, params: unknown[] = []): Record<string, unknown>[] {
  let file: string
  if (typeof database === 'string') {
    file = `/app/data/${database}.sqlite`
  } else {
    const shard = createHash('sha256').update(database.actor).digest('hex').slice(0, 2)
    file = `/app/data/actors/${shard}/${database.actor}/store.sqlite`
  }
  const run = compose(target, ['run', '--rm', '--no-deps', '-T', 'sqlite'], { input: JSON.stringify({ file, sql, params }) })
  return JSON.parse(run.stdout)
}

/**
 * Like `execPds`, without blocking the caller's event loop. A measurement
 * that samples the container while it also reads a stream needs this.
 */
export function execPdsAsync(target: TargetName, command: string[]): Promise<string> {
  const spec = targetSpec(target)
  const secrets = ensureSecrets(spec.runDir)
  return new Promise((resolve) => {
    execFile('docker', ['compose', '-f', COMPOSE_FILE, 'exec', '-T', 'pds', ...command], { env: composeEnv(spec, secrets), encoding: 'utf8' }, (_error, stdout) => resolve(stdout ?? ''))
  })
}

export function logs(target: TargetName, service: string, tail = 200): string {
  const run = compose(target, ['logs', '--no-color', '--tail', String(tail), service], { allowFailure: true })
  return run.stdout + run.stderr
}
