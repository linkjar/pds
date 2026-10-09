// Command line of the parity harness. See README.md.
//
//   pnpm parity images verify
//   pnpm parity stack up <target> [--profile <name>] [--keep]
//   pnpm parity stack down <target>
//   pnpm parity stack logs <target> [service]
//   pnpm parity run <target> [--profile <name>] [--only <text>] [--keep] [--reuse]
//   pnpm parity compare <left> <right> [--update]
//   pnpm parity perf report <target>

import { spawnSync } from 'node:child_process'
import { PROFILES, isProfile } from './stack/env.ts'
import type { Profile } from './stack/env.ts'
import { compare } from './compare.ts'
import { perfReport } from './perf-report.ts'
import { runScenarios } from './run.ts'
import { down, logs, up } from './stack/compose.ts'
import { REPO_DIR, TARGET_NAMES, images, isTargetName, targetSpec, upstream } from './stack/targets.ts'
import type { TargetName } from './stack/targets.ts'

function fail(message: string): never {
  console.error(message)
  process.exit(2)
}

function flag(args: string[], name: string): string | undefined {
  const at = args.indexOf(`--${name}`)
  return at === -1 ? undefined : args[at + 1]
}

function target(value: string | undefined): TargetName {
  if (!value || !isTargetName(value)) fail(`Target must be one of: ${TARGET_NAMES.join(', ')}`)
  return value
}

function profile(args: string[]): Profile {
  const value = flag(args, 'profile') ?? 'default'
  if (!isProfile(value)) fail(`Profile must be one of: ${PROFILES.join(', ')}`)
  return value
}

function docker(args: string[]): { status: number; out: string } {
  const run = spawnSync('docker', args, { encoding: 'utf8' })
  return { status: run.status ?? 1, out: (run.stdout ?? '') + (run.stderr ?? '') }
}

/** Checks that the pinned Reference image is the one upstream.json describes. */
function verifyImages(): void {
  const expectedTag = `atproto-pds-${upstream.tag.split('@').at(-1)}-${upstream.revision}`
  if (images.reference.tag !== expectedTag) {
    fail(`images.json pins ${images.reference.tag}; upstream.json describes ${expectedTag}. Update the digest and the tag together.`)
  }
  const ref = `${images.reference.image}@${images.reference.digest}`
  const inspect = docker(['image', 'inspect', ref, '--format', '{{index .Config.Labels "org.opencontainers.image.revision"}}'])
  if (inspect.status !== 0) {
    const pull = docker(['pull', ref])
    if (pull.status !== 0) fail(`Cannot pull ${ref}:\n${pull.out}`)
  }
  const label = docker(['image', 'inspect', ref, '--format', '{{index .Config.Labels "org.opencontainers.image.revision"}}'])
  if (label.out.trim() !== images.reference.revision) {
    fail(`Reference image revision label is ${label.out.trim()}; images.json records ${images.reference.revision}.`)
  }
  console.log(`reference  ${ref}\n           tag ${images.reference.tag}, patch pin ${images.reference.revision.slice(0, 7)}`)
  console.log(`official   ${upstream.officialImage}`)
  console.log(`stock      ${targetSpec('stock').image} (PATCH_PROFILE=none build of legacy/)`)
}

function buildStock(): void {
  const args = ['build', '--build-arg', 'PATCH_PROFILE=none', '-t', images.stock.tag, `${REPO_DIR}/${images.stock.build.context}`]
  const run = spawnSync('docker', args, { stdio: 'inherit' })
  process.exit(run.status ?? 1)
}

async function main(): Promise<void> {
  const [group, command, ...rest] = process.argv.slice(2)
  if (group === 'images' && command === 'verify') return verifyImages()
  if (group === 'images' && command === 'build-stock') return buildStock()
  if (group === 'stack' && command === 'up') {
    const name = target(rest[0])
    const state = await up(name, profile(rest), { fresh: !rest.includes('--keep') })
    console.log(`${name} is up: ${state.image}\nedge https://127.0.0.1:${state.edgePort} (profile ${state.profile})`)
    return
  }
  if (group === 'stack' && command === 'down') return down(target(rest[0]))
  if (group === 'stack' && command === 'logs') {
    console.log(logs(target(rest[0]), rest[1] ?? 'pds', Number(flag(rest, 'tail') ?? 200)))
    return
  }
  if (group === 'run') {
    const args = [command ?? '', ...rest]
    const status = await runScenarios(target(args[0]), {
      profile: profile(args),
      only: flag(args, 'only'),
      keep: args.includes('--keep'),
      reuse: args.includes('--reuse'),
    })
    process.exit(status)
  }
  if (group === 'perf' && command === 'report') {
    console.log(perfReport(target(rest[0])))
    return
  }
  if (group === 'compare') {
    process.exit(compare(target(command), target(rest[0]), { update: rest.includes('--update') }))
  }
  fail('Usage: pnpm parity <images verify|images build-stock|stack up|stack down|stack logs|run|compare|perf report> ...')
}

await main()
