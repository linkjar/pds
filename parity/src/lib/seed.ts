// What the seed scenarios of a restore run wrote, for the read scenarios
// that follow in other processes (src/restore.ts).

import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Account } from '../scenario.ts'
import { targetSpec } from '../stack/targets.ts'
import type { TargetName } from '../stack/targets.ts'

export type Seed = {
  /** The accounts by the role they play in the read scenarios. */
  accounts: Record<'alice' | 'big' | 'deactivated' | 'takendown' | 'moderated' | 'renamed', Account>
  /** Revisions of alice's commits, oldest first. */
  revs: string[]
  /** Record keys of alice's posts that still exist. */
  posts: string[]
  /** A record key that existed and was deleted. */
  deletedPost: string
  blobs: { text: string; png: string; deactivated: string; takendown: string; moderatedGone: string; moderatedKept: string }
  /** The record of `moderated` that was taken down, and one that was not. */
  moderatedRecord: { takenDown: string; kept: string }
  /** The handle `renamed` had before it took its current one. */
  renamedFrom: string
  /** How many records `big` holds. */
  bigCount: number
}

const file = (target: TargetName) => join(targetSpec(target).runDir, 'restore-seed.json')

export function saveSeed(target: TargetName, seed: Seed): void {
  writeFileSync(file(target), JSON.stringify(seed, null, 2) + '\n', { mode: 0o600 })
}

export function loadSeed(target: TargetName): Seed {
  if (!existsSync(file(target))) throw new Error('No seed. The read scenarios of the restore profile run through: pnpm parity restore')
  return JSON.parse(readFileSync(file(target), 'utf8'))
}
