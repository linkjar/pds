// Stores one blob on the Reference before the Candidate takes over
// (src/writes.ts, the `s3` stage). `writes/23-seeded-blob` then reads and
// deletes it on the server under test.

import assert from 'node:assert/strict'
import { SEEDED_BYTES, SEEDED_NOTE, saveSeed } from '../../src/lib/pool.ts'
import { scenario } from '../../src/scenario.ts'

scenario('01-seed-blob', {}, async (s) => {
  const account = await s.createAccount('seeded', {}, { silent: true })
  const uploaded = await s.http('uploadBlob', { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body: SEEDED_BYTES, contentType: 'text/plain', auth: account })
  assert.equal(uploaded.status, 200)
  const blob = uploaded.json.blob
  const created = await s.procedure('record referencing the blob', 'com.atproto.repo.createRecord', { repo: account.did, collection: SEEDED_NOTE, rkey: 'seeded', record: { $type: SEEDED_NOTE, file: blob } }, { auth: account })
  assert.equal(created.status, 200)
  // A second upload stays staged: no record references it yet.
  const staged = await s.http('uploadBlob that stays staged', { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body: Buffer.from('staged by the Reference\n'), contentType: 'text/plain', auth: account })
  assert.equal(staged.status, 200)
  saveSeed(s.target.name, { account, blob, staged: staged.json.blob })
})
