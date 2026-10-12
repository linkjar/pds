// The data directory of the read measurements: one repository of
// PARITY_RESTORE_RECORDS records (100,000 unless set), written through the
// Reference in batches of 200.

import assert from 'node:assert/strict'
import { writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { scenario } from '../../src/scenario.ts'
import { targetSpec } from '../../src/stack/targets.ts'

const NOTE = 'com.example.parity.note'
export const RECORDS = Number(process.env.PARITY_RESTORE_RECORDS ?? 100_000)

scenario('00-seed', { timeoutMs: 3_600_000 }, async (s) => {
  const big = await s.createAccount('big')
  for (let start = 0; start < RECORDS; start += 200) {
    const writes = []
    for (let i = start; i < Math.min(start + 200, RECORDS); i++) {
      writes.push({ $type: 'com.atproto.repo.applyWrites#create', collection: NOTE, rkey: `r${String(i).padStart(6, '0')}`, value: { $type: NOTE, text: `record ${i} of the read measurements`, createdAt: '2026-01-01T00:00:00.000Z' } })
    }
    const applied = await s.procedure('applyWrites', 'com.atproto.repo.applyWrites', { repo: big.did, writes }, { auth: big, silent: true })
    assert.equal(applied.status, 200, applied.text)
  }
  writeFileSync(join(targetSpec(s.target.name).runDir, 'restore-perf-seed.json'), JSON.stringify({ big, records: RECORDS }, null, 2) + '\n', { mode: 0o600 })
  s.note('seeded', { records: RECORDS })
})
