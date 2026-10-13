// Creates the accounts of a writes run, on the Reference (src/writes.ts).
// The scenarios that follow take them in order through `s.createAccount`.

import { savePool } from '../../src/lib/pool.ts'
import { scenario } from '../../src/scenario.ts'
import type { Account } from '../../src/scenario.ts'

/** More than the scenarios of one stage take. */
const SIZE = 24

scenario('00-pool', { timeoutMs: 300_000 }, async (s) => {
  const accounts: Account[] = []
  for (let i = 0; i < SIZE; i++) {
    accounts.push(await s.createAccount(`pool${String(i).padStart(2, '0')}`, {}, { silent: true }))
  }
  savePool(s.target.name, accounts)
})
