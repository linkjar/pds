// Accounts that a seed scenario created on the Reference, for scenarios that
// then run against a server that cannot create accounts yet (src/writes.ts).
//
// The Candidate takes over the Reference's data directory before it has an
// account endpoint of its own. A scenario asks for an account as it always
// does, with `s.createAccount`; with PARITY_ACCOUNT_POOL set it gets the next
// account of the pool, with a session the Reference minted. Both passes of a
// writes run draw from a pool in the same order, so their transcripts name
// the same handles.

import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Account } from '../scenario.ts'
import { targetSpec } from '../stack/targets.ts'
import type { TargetName } from '../stack/targets.ts'

type Pool = { accounts: Account[]; next: number }

const file = (target: TargetName) => join(targetSpec(target).runDir, 'account-pool.json')

export const poolEnabled = () => process.env.PARITY_ACCOUNT_POOL === '1'

export function savePool(target: TargetName, accounts: Account[]): void {
  writeFileSync(file(target), JSON.stringify({ accounts, next: 0 } satisfies Pool, null, 2) + '\n', { mode: 0o600 })
}

/** The next account of the pool. Scenarios run one at a time, so the counter in the file is enough. */
export function takeFromPool(target: TargetName, name: string): Account {
  if (!existsSync(file(target))) throw new Error('No account pool. Scenarios that need one run through: pnpm parity writes')
  const pool: Pool = JSON.parse(readFileSync(file(target), 'utf8'))
  const account = pool.accounts[pool.next]
  if (!account) throw new Error(`The account pool is used up (${pool.accounts.length} accounts). Make it larger in scenarios/writes/00-pool.scenario.ts`)
  writeFileSync(file(target), JSON.stringify({ ...pool, next: pool.next + 1 }, null, 2) + '\n', { mode: 0o600 })
  return { ...account, name }
}
