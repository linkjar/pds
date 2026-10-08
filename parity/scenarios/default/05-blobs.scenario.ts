// Blobs: SPEC 6.4, and VP-10 (what happens to a blob when its last
// reference goes).

import assert from 'node:assert/strict'
import { createHash, randomBytes } from 'node:crypto'
import { checkRepo } from '../../src/oracles/repo.ts'
import { refused, scenario } from '../../src/scenario.ts'
import type { Account, Scenario } from '../../src/scenario.ts'

const NOTE = 'com.example.parity.note'
const text = Buffer.from('hello, blob\n')
// The smallest valid PNG: one transparent pixel.
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==', 'base64')

const upload = (s: Scenario, step: string, account: Account, body: Uint8Array, contentType?: string) =>
  s.http(step, { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body, contentType, auth: account })

scenario('05-blobs', {}, async (s) => {
  const alice = await s.createAccount('alice')
  const bob = await s.createAccount('bob')

  const uploaded = await upload(s, 'uploadBlob', alice, text, 'text/plain')
  assert.equal(uploaded.status, 200)
  const blob = uploaded.json.blob
  assert.equal(blob.$type, 'blob')
  assert.equal(blob.mimeType, 'text/plain')
  assert.equal(blob.size, text.byteLength)
  // A blob CID is the raw-codec SHA-256 of the bytes: identical on every target.
  s.exact('blob CID', blob.ref.$link)
  assert.equal(createHash('sha256').update(text).digest('hex').length, 64)

  refused(await s.http('uploadBlob without a credential', { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body: text, contentType: 'text/plain' }), 'AuthMissing')

  // SPEC 6.4: an unreferenced upload is not downloadable and is absent from listBlobs.
  refused(await s.query('getBlob before any record references it', 'com.atproto.sync.getBlob', { did: alice.did, cid: blob.ref.$link }), 'InvalidRequest')
  const none = await s.query('listBlobs before any record references it', 'com.atproto.sync.listBlobs', { did: alice.did })
  assert.deepEqual(none.json.cids, [])

  // A blob belongs to the account that uploaded it.
  refused(
    await s.procedure("record referencing another account's upload", 'com.atproto.repo.createRecord', { repo: bob.did, collection: NOTE, rkey: 'stolen', record: { $type: NOTE, file: blob } }, { auth: bob }),
    'BlobNotFound',
  )

  const first = await s.procedure('record referencing the blob', 'com.atproto.repo.createRecord', { repo: alice.did, collection: NOTE, rkey: 'one', record: { $type: NOTE, file: blob } }, { auth: alice })
  assert.equal(first.status, 200)

  const served = await s.query('getBlob', 'com.atproto.sync.getBlob', { did: alice.did, cid: blob.ref.$link })
  assert.equal(served.status, 200)
  assert.deepEqual(Buffer.from(served.bytes), text)
  assert.match(served.headers.get('content-type') ?? '', /^text\/plain/)
  assert.equal(served.headers.get('content-length'), String(text.byteLength))
  assert.equal(served.headers.get('content-security-policy'), "default-src 'none'; sandbox")
  assert.equal(served.headers.get('x-content-type-options'), 'nosniff')
  s.note('getBlob content-length', served.headers.get('content-length'))

  const listed = await s.query('listBlobs', 'com.atproto.sync.listBlobs', { did: alice.did })
  assert.deepEqual(listed.json.cids, [blob.ref.$link])
  const since = await s.query('listBlobs since the referencing commit', 'com.atproto.sync.listBlobs', { did: alice.did, since: first.json.commit.rev })
  assert.deepEqual(since.json.cids, [])
  const missing = await s.query('listMissingBlobs', 'com.atproto.repo.listMissingBlobs', {}, { auth: alice })
  assert.deepEqual(missing.json.blobs, [])
  refused(await s.query('getBlob with a CID the account does not hold', 'com.atproto.sync.getBlob', { did: alice.did, cid: 'bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku' }), 'InvalidRequest')
  refused(await s.query('getBlob for an unknown DID', 'com.atproto.sync.getBlob', { did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa', cid: blob.ref.$link }))

  // The declared type against the content.
  const sniffed = await upload(s, 'uploadBlob of a PNG declared as text', alice, png, 'text/plain')
  s.note('type of a PNG declared as text', sniffed.json?.blob?.mimeType)
  const untyped = await upload(s, 'uploadBlob without Content-Type', alice, png)
  s.note('uploadBlob without Content-Type', { status: untyped.status, mimeType: untyped.json?.blob?.mimeType })
  const empty = await upload(s, 'uploadBlob of zero bytes', alice, new Uint8Array(0), 'text/plain')
  s.note('uploadBlob of zero bytes', empty.status)

  // SPEC 6.4: the upload limit, 5 MiB by default.
  const limit = 5 * 1024 * 1024
  const atLimit = await upload(s, 'uploadBlob at the limit', alice, randomBytes(limit), 'application/octet-stream')
  assert.equal(atLimit.status, 200)
  assert.equal(atLimit.json.blob.size, limit)
  const overLimit = await upload(s, 'uploadBlob one byte over the limit', alice, randomBytes(limit + 1), 'application/octet-stream')
  refused(overLimit)
  s.note('over the limit', { status: overLimit.status, error: overLimit.json?.error })

  // A second reference keeps the blob alive when the first goes.
  const second = await s.procedure('second record referencing the blob', 'com.atproto.repo.createRecord', { repo: alice.did, collection: NOTE, rkey: 'two', record: { $type: NOTE, file: blob } }, { auth: alice })
  assert.equal(second.status, 200)
  await s.procedure('delete the first reference', 'com.atproto.repo.deleteRecord', { repo: alice.did, collection: NOTE, rkey: 'one' }, { auth: alice })
  await new Promise((resolve) => setTimeout(resolve, 500))
  const stillThere = await s.query('getBlob while one reference remains', 'com.atproto.sync.getBlob', { did: alice.did, cid: blob.ref.$link })
  assert.equal(stillThere.status, 200)

  // VP-10: the Reference deletes a blob when the commit that removes its
  // last reference lands. There is no grace period. The harness measures
  // how long the blob stays downloadable.
  const started = Date.now()
  await s.procedure('delete the last reference', 'com.atproto.repo.deleteRecord', { repo: alice.did, collection: NOTE, rkey: 'two' }, { auth: alice })
  await s.eventually('the blob to stop being served', async () => {
    const probe = await s.query('probe', 'com.atproto.sync.getBlob', { did: alice.did, cid: blob.ref.$link }, { silent: true })
    return probe.status !== 200
  })
  const gone = await s.query('getBlob after the last reference went', 'com.atproto.sync.getBlob', { did: alice.did, cid: blob.ref.$link })
  refused(gone, 'InvalidRequest')
  const elapsed = Date.now() - started
  assert.ok(elapsed < 5_000, `VP-10: gone within seconds, took ${elapsed} ms`)
  s.note('VP-10 blob removed within five seconds of the last dereference', true)
  const afterDelete = await s.query('listBlobs after the last reference went', 'com.atproto.sync.listBlobs', { did: alice.did })
  assert.deepEqual(afterDelete.json.cids, [])

  // Replacing a record's blob through an update dereferences the old one too.
  const other = await upload(s, 'uploadBlob of a second file', alice, png, 'image/png')
  await s.procedure('record referencing the first file again', 'com.atproto.repo.createRecord', { repo: alice.did, collection: NOTE, rkey: 'three', record: { $type: NOTE, file: (await upload(s, 'uploadBlob again', alice, text, 'text/plain')).json.blob } }, { auth: alice })
  await s.procedure('update the record to the second file', 'com.atproto.repo.putRecord', { repo: alice.did, collection: NOTE, rkey: 'three', record: { $type: NOTE, file: other.json.blob } }, { auth: alice })
  await s.eventually('the replaced blob to stop being served', async () => {
    const probe = await s.query('probe', 'com.atproto.sync.getBlob', { did: alice.did, cid: blob.ref.$link }, { silent: true })
    return probe.status !== 200
  })
  const finalList = await s.query('listBlobs at the end', 'com.atproto.sync.listBlobs', { did: alice.did })
  assert.deepEqual(finalList.json.cids, [other.json.blob.ref.$link])

  await checkRepo(s, 'O2 after the blob writes', alice)
})
