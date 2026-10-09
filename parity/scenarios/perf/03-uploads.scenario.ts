// SPEC 16.1, S8: twenty concurrent uploads of 5 MiB.

import assert from 'node:assert/strict'
import { randomBytes } from 'node:crypto'
import { SCALE, memoryGrowth, saveResults, summary, timed } from '../../src/lib/perf.ts'
import { scenario } from '../../src/scenario.ts'

const UPLOADS = SCALE === 'full' ? 20 : 5
const SIZE = 5 * 1024 * 1024

scenario('03-uploads', { timeoutMs: 30 * 60 * 1000 }, async (s) => {
  await s.target.restart()
  const alice = await s.createAccount('alice', {}, { silent: true })
  const bodies = Array.from({ length: UPLOADS }, () => randomBytes(SIZE))
  const [growth, [wall, latencies]] = await memoryGrowth(s, () =>
    timed(() =>
      Promise.all(
        bodies.map(async (body) => {
          const [ms, response] = await timed(() => s.http('upload', { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body, contentType: 'application/octet-stream', auth: alice, silent: true }))
          assert.equal(response.status, 200, response.text)
          assert.equal(response.json.blob.size, SIZE)
          return ms
        }),
      ),
    ),
  )
  saveResults(s, 'S8', { uploads: UPLOADS, bytesEach: SIZE, wallMs: Math.round(wall), uploadMs: summary(latencies), memoryGrowthMiB: growth })
})
