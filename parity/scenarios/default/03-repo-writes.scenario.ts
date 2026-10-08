// Repository writes: SPEC 6.3 (writes, swaps, validation modes), 6.2 (record
// keys) and 6.6 (determinism, oracle O2). Record keys and content are fixed
// so that every target must arrive at the same MST root.

import assert from 'node:assert/strict'
import { checkRepo } from '../../src/oracles/repo.ts'
import { refused, scenario } from '../../src/scenario.ts'

const POST = 'app.bsky.feed.post'
const post = (text: string) => ({ $type: POST, text, createdAt: '2026-01-01T00:00:00.000Z' })

scenario('03-repo-writes', {}, async (s) => {
  const alice = await s.createAccount('alice')
  const bob = await s.createAccount('bob')
  const empty = await checkRepo(s, 'O2 after account creation', alice)
  assert.equal(empty.records, 0)

  // createRecord with an explicit key.
  const first = await s.procedure('createRecord', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa2', record: post('first') }, { auth: alice })
  assert.equal(first.status, 200)
  assert.equal(first.json.uri, `at://${alice.did}/${POST}/3aaaaaaaaaaa2`)
  assert.equal(first.json.validationStatus, 'valid')
  assert.ok(first.json.commit.cid && first.json.commit.rev)
  // The record CID covers content only, so it is the same on every target.
  s.exact('record CID of the first post', first.json.cid)
  const afterFirst = await checkRepo(s, 'O2 after createRecord', alice)
  assert.equal(afterFirst.commit, first.json.commit.cid)
  assert.equal(afterFirst.rev, first.json.commit.rev)
  assert.ok(afterFirst.rev > empty.rev, 'rev increases (SPEC 6.2)')

  // The handle works as the repo identifier.
  const byHandle = await s.procedure('createRecord by handle', 'com.atproto.repo.createRecord', { repo: alice.handle, collection: POST, rkey: '3aaaaaaaaaaa3', record: post('second') }, { auth: alice })
  assert.equal(byHandle.status, 200)

  // A key that exists.
  const clash = await s.procedure('createRecord with a key that exists', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa2', record: post('clash') }, { auth: alice })
  refused(clash)

  // Another account's repository.
  const foreign = await s.procedure("createRecord in another account's repository", 'com.atproto.repo.createRecord', { repo: bob.did, collection: POST, record: post('x') }, { auth: alice })
  refused(foreign, 'AuthenticationRequired')
  const anonymous = await s.procedure('createRecord without a credential', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, record: post('x') })
  assert.equal(anonymous.status, 401)

  // putRecord: update, then create through put.
  const updated = await s.procedure('putRecord updating a record', 'com.atproto.repo.putRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa2', record: post('first, edited') }, { auth: alice })
  assert.equal(updated.status, 200)
  assert.notEqual(updated.json.cid, first.json.cid)
  const putNew = await s.procedure('putRecord creating a record', 'com.atproto.repo.putRecord', { repo: alice.did, collection: 'app.bsky.actor.profile', rkey: 'self', record: { $type: 'app.bsky.actor.profile', displayName: 'Alice' } }, { auth: alice })
  assert.equal(putNew.status, 200)

  // SPEC 6.3: a write that changes nothing makes no commit.
  const before = await checkRepo(s, 'O2 before a no-op put', alice)
  const noop = await s.procedure('putRecord with identical content', 'com.atproto.repo.putRecord', { repo: alice.did, collection: 'app.bsky.actor.profile', rkey: 'self', record: { $type: 'app.bsky.actor.profile', displayName: 'Alice' } }, { auth: alice })
  assert.equal(noop.status, 200)
  const after = await checkRepo(s, 'O2 after a no-op put', alice)
  assert.equal(after.commit, before.commit, 'no new commit')

  // Swaps.
  const staleCommit = await s.procedure('createRecord with a stale swapCommit', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa4', record: post('x'), swapCommit: first.json.commit.cid }, { auth: alice })
  refused(staleCommit, 'InvalidSwap')
  const staleRecord = await s.procedure('putRecord with a stale swapRecord', 'com.atproto.repo.putRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa2', record: post('y'), swapRecord: first.json.cid }, { auth: alice })
  refused(staleRecord, 'InvalidSwap')
  const goodSwap = await s.procedure('putRecord with the current swapRecord and swapCommit', 'com.atproto.repo.putRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa2', record: post('first, edited twice'), swapRecord: updated.json.cid, swapCommit: after.commit }, { auth: alice })
  assert.equal(goodSwap.status, 200)
  const staleDelete = await s.procedure('deleteRecord with a stale swapRecord', 'com.atproto.repo.deleteRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa2', swapRecord: first.json.cid }, { auth: alice })
  refused(staleDelete, 'InvalidSwap')

  // Validation modes (SPEC 6.3).
  const unknownType = { $type: 'com.example.parity.note', text: 'unknown lexicon' }
  const unknownDefault = await s.procedure('unknown lexicon, validate unset', 'com.atproto.repo.createRecord', { repo: alice.did, collection: 'com.example.parity.note', rkey: 'one', record: unknownType }, { auth: alice })
  assert.equal(unknownDefault.status, 200)
  assert.equal(unknownDefault.json.validationStatus, 'unknown')
  const unknownStrict = await s.procedure('unknown lexicon, validate true', 'com.atproto.repo.createRecord', { repo: alice.did, collection: 'com.example.parity.note', rkey: 'two', record: unknownType, validate: true }, { auth: alice })
  refused(unknownStrict)
  const invalidPost = { $type: POST, createdAt: '2026-01-01T00:00:00.000Z' }
  const knownInvalid = await s.procedure('known lexicon, invalid record', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa5', record: invalidPost }, { auth: alice })
  refused(knownInvalid, 'InvalidRequest')
  const knownInvalidLoose = await s.procedure('known lexicon, invalid record, validate false', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa5', record: invalidPost, validate: false }, { auth: alice })
  assert.equal(knownInvalidLoose.status, 200)
  assert.equal(knownInvalidLoose.json.validationStatus, undefined)
  const wrongType = await s.procedure('$type that does not match the collection', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa6', record: { $type: 'app.bsky.feed.like', text: 'x' } }, { auth: alice })
  refused(wrongType)

  // Data-model rules that hold in every mode (SPEC 6.3).
  const float = await s.procedure('record with a float', 'com.atproto.repo.createRecord', { repo: alice.did, collection: 'com.example.parity.note', rkey: 'float', record: { $type: 'com.example.parity.note', ratio: 1.5 }, validate: false }, { auth: alice })
  refused(float)
  const missingBlob = await s.procedure('record that references a blob the server does not hold', 'com.atproto.repo.createRecord', {
    repo: alice.did,
    collection: 'com.example.parity.note',
    rkey: 'blob',
    record: { $type: 'com.example.parity.note', file: { $type: 'blob', ref: { $link: 'bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku' }, mimeType: 'text/plain', size: 11 } },
  }, { auth: alice })
  refused(missingBlob)

  // Record keys (SPEC 6.2).
  for (const [label, rkey] of [['a dot', '.'], ['two dots', '..'], ['a slash', 'a/b'], ['a percent sign', 'a%20b'], ['513 characters', 'k'.repeat(513)]] as const) {
    const bad = await s.procedure(`record key that is ${label}`, 'com.atproto.repo.createRecord', { repo: alice.did, collection: 'com.example.parity.note', rkey, record: unknownType }, { auth: alice })
    refused(bad)
  }
  const longKey = await s.procedure('record key of 512 characters', 'com.atproto.repo.createRecord', { repo: alice.did, collection: 'com.example.parity.note', rkey: 'k'.repeat(512), record: unknownType }, { auth: alice })
  assert.equal(longKey.status, 200)
  const oddKey = await s.procedure('record key with every allowed punctuation mark', 'com.atproto.repo.createRecord', { repo: alice.did, collection: 'com.example.parity.note', rkey: 'a.b_c:d~e-f', record: unknownType }, { auth: alice })
  assert.equal(oddKey.status, 200)

  // SPEC 6.2: the key type the lexicon declares is enforced. A post is keyed by TID.
  const notATid = await s.procedure('post with a key that is not a TID', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey: 'not-a-tid', record: post('x') }, { auth: alice })
  refused(notATid, 'InvalidRequest')
  const profileKey = await s.procedure('profile with a key other than self', 'com.atproto.repo.createRecord', { repo: alice.did, collection: 'app.bsky.actor.profile', rkey: 'other', record: { $type: 'app.bsky.actor.profile' } }, { auth: alice })
  refused(profileKey, 'InvalidRequest')

  // applyWrites: create, update and delete in one commit.
  const batch = await s.procedure('applyWrites', 'com.atproto.repo.applyWrites', {
    repo: alice.did,
    writes: [
      { $type: 'com.atproto.repo.applyWrites#create', collection: POST, rkey: '3aaaaaaaaaaa7', value: post('batch create') },
      { $type: 'com.atproto.repo.applyWrites#update', collection: POST, rkey: '3aaaaaaaaaaa3', value: post('second, edited') },
      { $type: 'com.atproto.repo.applyWrites#delete', collection: 'com.example.parity.note', rkey: 'one' },
    ],
  }, { auth: alice })
  assert.equal(batch.status, 200)
  assert.equal(batch.json.results.length, 3)
  assert.deepEqual(batch.json.results.map((r: { $type: string }) => r.$type), [
    'com.atproto.repo.applyWrites#createResult',
    'com.atproto.repo.applyWrites#updateResult',
    'com.atproto.repo.applyWrites#deleteResult',
  ])
  const atomic = await s.procedure('applyWrites with one invalid operation', 'com.atproto.repo.applyWrites', {
    repo: alice.did,
    writes: [
      { $type: 'com.atproto.repo.applyWrites#create', collection: POST, rkey: '3aaaaaaaaaaab', value: post('never written') },
      { $type: 'com.atproto.repo.applyWrites#create', collection: POST, rkey: '3aaaaaaaaaaa7', value: post('key exists') },
    ],
  }, { auth: alice })
  refused(atomic)
  const absent = await s.query('the valid half of a failed batch was not written', 'com.atproto.repo.getRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaab' })
  refused(absent, 'RecordNotFound')

  // SPEC 6.3: at most 200 operations.
  const writes = (count: number, prefix: string) =>
    Array.from({ length: count }, (_, i) => ({ $type: 'com.atproto.repo.applyWrites#create', collection: 'com.example.parity.note', rkey: `${prefix}${String(i).padStart(3, '0')}`, value: unknownType }))
  const tooMany = await s.procedure('applyWrites with 201 operations', 'com.atproto.repo.applyWrites', { repo: alice.did, writes: writes(201, 'x') }, { auth: alice })
  refused(tooMany)
  const full = await s.procedure('applyWrites with 200 operations', 'com.atproto.repo.applyWrites', { repo: alice.did, writes: writes(200, 'n') }, { auth: alice })
  assert.equal(full.status, 200)

  // deleteRecord, and deleting what is not there.
  const deleted = await s.procedure('deleteRecord', 'com.atproto.repo.deleteRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa7' }, { auth: alice })
  assert.equal(deleted.status, 200)
  const deletedAgain = await s.procedure('deleteRecord of a record that is gone', 'com.atproto.repo.deleteRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa7' }, { auth: alice })
  assert.equal(deletedAgain.status, 200)

  const final = await checkRepo(s, 'O2 after every write', alice)
  assert.equal(final.records, 206)
  await checkRepo(s, 'O2 on the untouched repository', bob)
})
