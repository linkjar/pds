// Repository and sync reads: Appendix A read methods, the deprecated methods
// that the pin still serves (VP-5), and `getRepo` with `since`.

import assert from 'node:assert/strict'
import { readCar, readCarWithRoot, verifyRecords } from '@atproto/repo'
import { signingKey } from '../../src/oracles/repo.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const POST = 'app.bsky.feed.post'
const keys = ['3aaaaaaaaaaa2', '3aaaaaaaaaaa3', '3aaaaaaaaaaa4', '3aaaaaaaaaaa5', '3aaaaaaaaaaa6']

scenario('04-repo-reads', {}, async (s) => {
  const alice = await s.createAccount('alice')
  const revs: string[] = []
  for (const [i, rkey] of keys.entries()) {
    const written = await s.procedure(`write post ${i + 1}`, 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey, record: { $type: POST, text: `post ${i + 1}`, createdAt: '2026-01-01T00:00:00.000Z' } }, { auth: alice, silent: true })
    assert.equal(written.status, 200)
    revs.push(written.json.commit.rev)
  }

  // com.atproto.repo reads need no credential.
  const record = await s.query('getRecord', 'com.atproto.repo.getRecord', { repo: alice.did, collection: POST, rkey: keys[0] })
  assert.equal(record.status, 200)
  assert.equal(record.json.value.text, 'post 1')
  const byHandle = await s.query('getRecord by handle', 'com.atproto.repo.getRecord', { repo: alice.handle, collection: POST, rkey: keys[0] })
  assert.equal(byHandle.json.cid, record.json.cid)
  const pinned = await s.query('getRecord with the matching cid', 'com.atproto.repo.getRecord', { repo: alice.did, collection: POST, rkey: keys[0], cid: record.json.cid })
  assert.equal(pinned.status, 200)
  refused(await s.query('getRecord with another cid', 'com.atproto.repo.getRecord', { repo: alice.did, collection: POST, rkey: keys[0], cid: 'bafyreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku' }), 'RecordNotFound')
  refused(await s.query('getRecord that does not exist', 'com.atproto.repo.getRecord', { repo: alice.did, collection: POST, rkey: '3zzzzzzzzzzzz' }), 'RecordNotFound')
  // A repository this server does not host: the Reference forwards the read
  // to the AppView, without a credential and without service auth.
  const mark = await s.fixtures.cursor()
  const elsewhere = await s.query('getRecord in a repository hosted elsewhere', 'com.atproto.repo.getRecord', { repo: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa', collection: POST, rkey: keys[0] })
  assert.equal(elsewhere.status, 200)
  const forwarded = await s.fixtures.requests(HOSTS.appview, mark)
  assert.equal(forwarded.length, 1)
  s.note('forwarded getRecord', { path: forwarded[0]!.path, query: forwarded[0]!.query, authorization: forwarded[0]!.headers.authorization ?? null })

  const all = await s.query('listRecords', 'com.atproto.repo.listRecords', { repo: alice.did, collection: POST })
  assert.deepEqual(all.json.records.map((r: { uri: string }) => r.uri.split('/').at(-1)), [...keys].reverse(), 'newest first by default')
  const page = await s.query('listRecords, limit 2', 'com.atproto.repo.listRecords', { repo: alice.did, collection: POST, limit: 2 })
  assert.equal(page.json.records.length, 2)
  assert.ok(page.json.cursor)
  const next = await s.query('listRecords, second page', 'com.atproto.repo.listRecords', { repo: alice.did, collection: POST, limit: 2, cursor: page.json.cursor })
  assert.deepEqual(next.json.records.map((r: { uri: string }) => r.uri.split('/').at(-1)), [keys[2], keys[1]])
  const reversed = await s.query('listRecords, reverse', 'com.atproto.repo.listRecords', { repo: alice.did, collection: POST, reverse: true, limit: 2 })
  assert.deepEqual(reversed.json.records.map((r: { uri: string }) => r.uri.split('/').at(-1)), [keys[0], keys[1]])
  const emptyCollection = await s.query('listRecords of an empty collection', 'com.atproto.repo.listRecords', { repo: alice.did, collection: 'app.bsky.feed.like' })
  assert.deepEqual(emptyCollection.json.records, [])
  refused(await s.query('listRecords, limit 101', 'com.atproto.repo.listRecords', { repo: alice.did, collection: POST, limit: 101 }), 'InvalidRequest')

  const described = await s.query('describeRepo', 'com.atproto.repo.describeRepo', { repo: alice.did })
  assert.equal(described.status, 200)
  assert.equal(described.json.handle, alice.handle)
  assert.equal(described.json.handleIsCorrect, true)
  assert.deepEqual(described.json.collections, [POST])
  assert.equal(described.json.didDoc.id, alice.did)

  // com.atproto.sync reads.
  const latest = await s.query('getLatestCommit', 'com.atproto.sync.getLatestCommit', { did: alice.did })
  assert.equal(latest.json.rev, revs.at(-1))
  refused(await s.query('getLatestCommit for an unknown DID', 'com.atproto.sync.getLatestCommit', { did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' }), 'RepoNotFound')
  const status = await s.query('getRepoStatus', 'com.atproto.sync.getRepoStatus', { did: alice.did })
  assert.deepEqual(status.json, { did: alice.did, active: true, rev: revs.at(-1) })
  refused(await s.query('getRepoStatus for an unknown DID', 'com.atproto.sync.getRepoStatus', { did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' }), 'RepoNotFound')

  const repos = await s.query('listRepos', 'com.atproto.sync.listRepos', { limit: 1000 }, { silent: true })
  const mine = repos.json.repos.find((r: { did: string }) => r.did === alice.did)
  s.note('listRepos entry', mine)
  assert.deepEqual(mine, { did: alice.did, head: latest.json.cid, rev: revs.at(-1), active: true })

  // A record with its proof, checked with the SDK against the PLC key.
  const didKey = await signingKey(s, alice.did)
  const proof = await s.query('sync.getRecord', 'com.atproto.sync.getRecord', { did: alice.did, collection: POST, rkey: keys[2] })
  assert.equal(proof.headers.get('content-type'), 'application/vnd.ipld.car')
  const claims = await verifyRecords(proof.bytes, alice.did, didKey)
  assert.equal(claims.length, 1)
  assert.equal((claims[0]!.record as { text: string }).text, 'post 3')
  // A proof of absence is a CAR as well, and proves nothing present.
  const absent = await s.query('sync.getRecord for a key that is absent', 'com.atproto.sync.getRecord', { did: alice.did, collection: POST, rkey: '3zzzzzzzzzzzz' })
  s.note('proof of absence', { status: absent.status, records: absent.status === 200 ? (await verifyRecords(absent.bytes, alice.did, didKey)).length : null })

  // Blocks by CID.
  const blocks = await s.query('getBlocks', 'com.atproto.sync.getBlocks', { did: alice.did, cids: [latest.json.cid, record.json.cid] })
  assert.equal(blocks.status, 200)
  // The CAR of getBlocks has no root.
  const car = await readCar(blocks.bytes)
  assert.equal(car.roots.length, 0)
  assert.deepEqual(car.blocks.entries().map((e) => e.cid.toString()).sort(), [latest.json.cid, record.json.cid].sort())
  refused(await s.query('getBlocks for a block the repository lacks', 'com.atproto.sync.getBlocks', { did: alice.did, cids: ['bafyreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku'] }))

  // getRepo with `since`: only what changed after that revision.
  const full = await s.query('getRepo', 'com.atproto.sync.getRepo', { did: alice.did })
  const diff = await s.query('getRepo since the third write', 'com.atproto.sync.getRepo', { did: alice.did, since: revs[2] })
  const fullCar = await readCarWithRoot(full.bytes)
  const diffCar = await readCarWithRoot(diff.bytes)
  assert.equal(diffCar.root.toString(), fullCar.root.toString())
  assert.ok(diffCar.blocks.size < fullCar.blocks.size, 'the diff is smaller than the export')
  const fullCids = new Set(fullCar.blocks.entries().map((e) => e.cid.toString()))
  assert.ok(diffCar.blocks.entries().every((e) => fullCids.has(e.cid.toString())), 'every diff block is in the export')
  s.note('getRepo block counts', { full: fullCar.blocks.size, since: diffCar.blocks.size })
  refused(await s.query('getRepo for an unknown DID', 'com.atproto.sync.getRepo', { did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' }), 'RepoNotFound')

  // VP-5: the deprecated methods are still served at the pin.
  const head = await s.query('VP-5 getHead', 'com.atproto.sync.getHead', { did: alice.did })
  assert.equal(head.status, 200)
  assert.deepEqual(head.json, { root: latest.json.cid })
  const checkout = await s.query('VP-5 getCheckout', 'com.atproto.sync.getCheckout', { did: alice.did })
  assert.equal(checkout.status, 200)
  assert.equal((await readCarWithRoot(checkout.bytes)).blocks.size, fullCar.blocks.size)
})
