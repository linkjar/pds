// Writes the data directory of a restore run, on the Reference: accounts in
// every status a read can meet, a repository large enough to page, records
// that were deleted and replaced, and blobs.

import assert from 'node:assert/strict'
import { saveSeed } from '../../src/lib/seed.ts'
import { scenario } from '../../src/scenario.ts'
import type { Account, Scenario } from '../../src/scenario.ts'

const POST = 'app.bsky.feed.post'
const NOTE = 'com.example.parity.note'
const CREATED = '2026-01-01T00:00:00.000Z'
// The smallest valid PNG: one transparent pixel.
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==', 'base64')

async function upload(s: Scenario, account: Account, body: Uint8Array | string, contentType: string): Promise<{ $type: string; ref: { $link: string }; mimeType: string; size: number }> {
  const result = await s.http('upload', { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body, contentType, auth: account, silent: true })
  assert.equal(result.status, 200, result.text)
  return result.json.blob
}

async function write(s: Scenario, account: Account, collection: string, rkey: string, record: Record<string, unknown>): Promise<{ rev: string; cid: string }> {
  const result = await s.procedure('write', 'com.atproto.repo.createRecord', { repo: account.did, collection, rkey, record: { $type: collection, ...record } }, { auth: account, silent: true })
  assert.equal(result.status, 200, result.text)
  return { rev: result.json.commit.rev, cid: result.json.cid }
}

scenario('00-seed', { timeoutMs: 600_000 }, async (s) => {
  const admin = { auth: 'admin', silent: true } as const

  // alice: posts, one deleted and one replaced, a like, and two blobs.
  const alice = await s.createAccount('alice')
  const revs: string[] = []
  const keys = ['3aaaaaaaaaaa2', '3aaaaaaaaaaa3', '3aaaaaaaaaaa4', '3aaaaaaaaaaa5', '3aaaaaaaaaaa6']
  for (const [i, rkey] of keys.entries()) revs.push((await write(s, alice, POST, rkey, { text: `post ${i + 1}`, createdAt: CREATED })).rev)
  const removed = await s.procedure('delete', 'com.atproto.repo.deleteRecord', { repo: alice.did, collection: POST, rkey: keys[1] }, { auth: alice, silent: true })
  assert.equal(removed.status, 200)
  revs.push(removed.json.commit.rev)
  const replaced = await s.procedure('put', 'com.atproto.repo.putRecord', { repo: alice.did, collection: POST, rkey: keys[2], record: { $type: POST, text: 'post 3, replaced', createdAt: CREATED } }, { auth: alice, silent: true })
  assert.equal(replaced.status, 200)
  revs.push(replaced.json.commit.rev)
  const textBlob = await upload(s, alice, 'a text blob', 'text/plain')
  const pngBlob = await upload(s, alice, png, 'image/png')
  revs.push((await write(s, alice, NOTE, 'text', { file: textBlob })).rev)
  revs.push((await write(s, alice, NOTE, 'image', { file: pngBlob, nested: { list: [1, 'two', null, true], bytes: { $bytes: 'aGVsbG8' } } })).rev)
  revs.push((await write(s, alice, 'app.bsky.feed.like', '3aaaaaaaaaaa7', { subject: { uri: `at://${alice.did}/${POST}/${keys[0]}`, cid: 'bafyreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku' }, createdAt: CREATED })).rev)

  // big: more blocks than one page of the export holds.
  const big = await s.createAccount('big')
  const bigCount = 700
  for (let start = 0; start < bigCount; start += 200) {
    const writes = []
    for (let i = start; i < Math.min(start + 200, bigCount); i++) {
      writes.push({ $type: 'com.atproto.repo.applyWrites#create', collection: NOTE, rkey: `r${String(i).padStart(4, '0')}`, value: { $type: NOTE, text: `big ${i}` } })
    }
    const applied = await s.procedure('applyWrites', 'com.atproto.repo.applyWrites', { repo: big.did, writes }, { auth: big, silent: true })
    assert.equal(applied.status, 200, applied.text)
  }

  // deactivated: by its owner.
  const deactivated = await s.createAccount('dana')
  const deactivatedBlob = await upload(s, deactivated, 'the blob of a deactivated account', 'text/plain')
  await write(s, deactivated, NOTE, 'one', { file: deactivatedBlob })
  assert.equal((await s.procedure('deactivate', 'com.atproto.server.deactivateAccount', {}, { auth: deactivated, silent: true })).status, 200)

  // takendown: by an admin.
  const takendown = await s.createAccount('tara')
  const takendownBlob = await upload(s, takendown, 'the blob of an account that is taken down', 'text/plain')
  await write(s, takendown, NOTE, 'one', { file: takendownBlob })
  assert.equal((await s.procedure('takedown', 'com.atproto.admin.updateSubjectStatus', { subject: { $type: 'com.atproto.admin.defs#repoRef', did: takendown.did }, takedown: { applied: true, ref: 'restore-1' } }, admin)).status, 200)

  // moderated: an active account with one record and one blob taken down.
  const moderated = await s.createAccount('mona')
  const goneBlob = await upload(s, moderated, 'a blob that is taken down', 'text/plain')
  const keptBlob = await upload(s, moderated, 'a blob that stays', 'text/plain')
  const gone = await write(s, moderated, NOTE, 'gone', { text: 'a record that is taken down' })
  await write(s, moderated, NOTE, 'kept', { text: 'a record that stays', files: [goneBlob, keptBlob] })
  assert.equal((await s.procedure('record takedown', 'com.atproto.admin.updateSubjectStatus', { subject: { $type: 'com.atproto.repo.strongRef', uri: `at://${moderated.did}/${NOTE}/gone`, cid: gone.cid }, takedown: { applied: true, ref: 'restore-2' } }, admin)).status, 200)
  assert.equal((await s.procedure('blob takedown', 'com.atproto.admin.updateSubjectStatus', { subject: { $type: 'com.atproto.admin.defs#repoBlobRef', did: moderated.did, cid: goneBlob.ref.$link }, takedown: { applied: true, ref: 'restore-3' } }, admin)).status, 200)

  // renamed: the handle in the account database and in the DID document moved together.
  const renamed = await s.createAccount('rene')
  const renamedFrom = renamed.handle
  const newHandle = s.handle('renata')
  assert.equal((await s.procedure('updateHandle', 'com.atproto.identity.updateHandle', { handle: newHandle }, { auth: renamed, silent: true })).status, 200)
  renamed.handle = newHandle

  saveSeed(s.target.name, {
    accounts: { alice, big, deactivated, takendown, moderated, renamed },
    revs,
    posts: [keys[0]!, keys[2]!, keys[3]!, keys[4]!],
    deletedPost: keys[1]!,
    blobs: { text: textBlob.ref.$link, png: pngBlob.ref.$link, deactivated: deactivatedBlob.ref.$link, takendown: takendownBlob.ref.$link, moderatedGone: goneBlob.ref.$link, moderatedKept: keptBlob.ref.$link },
    moderatedRecord: { takenDown: 'gone', kept: 'kept' },
    renamedFrom,
    bigCount,
  })
  s.note('seeded', { accounts: 6, bigCount })
})
