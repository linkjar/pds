// A blob that the Reference stored before the takeover (SPEC 8.1): the
// server under test serves it, makes a staged upload of the Reference
// permanent, and removes both from the store with their last reference.

import assert from 'node:assert/strict'
import { SEEDED_BYTES, SEEDED_NOTE, loadSeed } from '../../src/lib/pool.ts'
import { checkRepo } from '../../src/oracles/repo.ts'
import { refused, scenario } from '../../src/scenario.ts'
import type { Account } from '../../src/scenario.ts'

type Blob = { ref: { $link: string }; mimeType: string; size: number }

scenario('23-seeded-blob', {}, async (s) => {
  const { account, blob, staged } = loadSeed<{ account: Account; blob: Blob; staged: Blob }>(s.target.name)

  const served = await s.query('getBlob of a blob the Reference stored', 'com.atproto.sync.getBlob', { did: account.did, cid: blob.ref.$link })
  assert.equal(served.status, 200)
  assert.deepEqual(Buffer.from(served.bytes), SEEDED_BYTES)
  assert.equal(served.headers.get('content-length'), String(SEEDED_BYTES.byteLength))

  // The upload that the Reference staged becomes permanent here.
  // On this store the Reference answers 500 here (default/05-blobs).
  refused(await s.query('getBlob of the staged upload', 'com.atproto.sync.getBlob', { did: account.did, cid: staged.ref.$link }))
  const second = await s.procedure('record referencing the staged upload', 'com.atproto.repo.createRecord', { repo: account.did, collection: SEEDED_NOTE, rkey: 'staged', record: { $type: SEEDED_NOTE, file: staged } }, { auth: account })
  assert.equal(second.status, 200)
  const moved = await s.query('getBlob of the upload made permanent', 'com.atproto.sync.getBlob', { did: account.did, cid: staged.ref.$link })
  assert.equal(moved.status, 200)
  assert.equal(Buffer.from(moved.bytes).toString(), 'staged by the Reference\n')
  const listed = await s.query('listBlobs', 'com.atproto.sync.listBlobs', { did: account.did })
  assert.deepEqual([...listed.json.cids].sort(), [blob.ref.$link, staged.ref.$link].sort())

  for (const [rkey, cid] of [['seeded', blob.ref.$link], ['staged', staged.ref.$link]] as const) {
    await s.procedure(`delete the record "${rkey}"`, 'com.atproto.repo.deleteRecord', { repo: account.did, collection: SEEDED_NOTE, rkey }, { auth: account })
    await s.eventually(`the blob of "${rkey}" to stop being served`, async () => {
      const probe = await s.query('probe', 'com.atproto.sync.getBlob', { did: account.did, cid }, { silent: true })
      return probe.status !== 200
    })
    refused(await s.query(`getBlob after "${rkey}" went`, 'com.atproto.sync.getBlob', { did: account.did, cid }), 'InvalidRequest')
  }
  const none = await s.query('listBlobs at the end', 'com.atproto.sync.listBlobs', { did: account.did })
  assert.deepEqual(none.json.cids, [])
  await checkRepo(s, 'after the blob writes', account)
})
