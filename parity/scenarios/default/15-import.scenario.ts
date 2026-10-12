// Repository import: SPEC 6.5, and 6.1 for a tree that references one node
// more than once (upstream #5584, carried as patch 104; SPEC 2.3, DD-6).
//
// The shared-node tree here is four levels deep, so that a build without the
// fix walks it in a moment. The same shape forty levels deep is the case the
// fix exists for, and it is not sent to any target.

import assert from 'node:assert/strict'
import { TID } from '@atproto/common-web'
import { Secp256k1Keypair } from '@atproto/crypto'
import { BlockMap, blocksToCarFile, signCommit } from '@atproto/repo'
import { refused, scenario } from '../../src/scenario.ts'
import type { Account, Scenario } from '../../src/scenario.ts'

const POST = 'app.bsky.feed.post'
const CAR = 'application/vnd.ipld.car'

type Cid = Awaited<ReturnType<BlockMap['add']>>

/**
 * A CAR whose tree has two nodes on each level, each pointing at both nodes
 * of the level below. It is upstream's construction in
 * `packages/pds/tests/import-repo.test.ts`.
 */
async function sharedNodeCar(did: string, depth: number): Promise<Uint8Array> {
  const blocks = new BlockMap()
  const text = new TextEncoder()
  const value = await blocks.add({ $type: 'com.example.dummy' })
  const entry = (key: string, tree: Cid | null) => ({ p: 0, k: text.encode(key), v: value, t: tree })
  let level: Cid[] = []
  for (let j = 0; j < 2; j++) level.push(await blocks.add({ l: null, e: [entry(`com.example.lvl0/${j}`, null)] }))
  for (let i = 1; i < depth; i++) {
    const next: Cid[] = []
    for (let j = 0; j < 2; j++) next.push(await blocks.add({ l: level[j]!, e: [entry(`com.example.lvl${i}/${j}`, level[(j + 1) % 2]!)] }))
    level = next
  }
  // The import does not check the signature (SPEC 6.5 names this), so any key will do.
  const keypair = await Secp256k1Keypair.create()
  const commit = await signCommit({ did, version: 3, rev: TID.nextStr(), prev: null, data: level[0]! }, keypair)
  const root = await blocks.add(commit)
  return blocksToCarFile(root, blocks)
}

const importRepo = (s: Scenario, step: string, account: Account | undefined, car: Uint8Array) =>
  s.http(step, { method: 'POST', path: '/xrpc/com.atproto.repo.importRepo', body: car, contentType: CAR, auth: account })

scenario('15-import', {}, async (s) => {
  const alice = await s.createAccount('alice')
  const written = await s.procedure('write a post', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa2', record: { $type: POST, text: 'before the import', createdAt: '2026-01-01T00:00:00.000Z' } }, { auth: alice, silent: true })
  assert.equal(written.status, 200)
  const before = await s.query('latest commit before', 'com.atproto.sync.getLatestCommit', { did: alice.did })

  refused(await importRepo(s, 'importRepo without a credential', undefined, new Uint8Array([0])), 'AuthMissing')

  // The account's own export imports cleanly, and a repeat does too (SPEC 6.5).
  const exported = await s.query('getRepo', 'com.atproto.sync.getRepo', { did: alice.did }, { silent: true })
  assert.equal((await importRepo(s, 'importRepo of the own export', alice, exported.bytes)).status, 200)
  assert.equal((await importRepo(s, 'importRepo of the own export again', alice, exported.bytes)).status, 200)

  refused(await importRepo(s, 'importRepo of bytes that are not a CAR', alice, new TextEncoder().encode('not a car')))

  // DD-6. The production Reference carries the fix and answers 400; what a
  // build without it answers is recorded by the transcript.
  const shared = await importRepo(s, 'importRepo of a tree that shares nodes', alice, await sharedNodeCar(alice.did, 4))
  s.note('shared-node import', { status: shared.status, error: shared.json?.error ?? null })

  // The server still serves. A build that accepted the import has replaced
  // the repository with the imported tree, so the post is gone there.
  const record = await s.query('the post after the imports', 'com.atproto.repo.getRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa2' })
  const after = await s.query('latest commit after', 'com.atproto.sync.getLatestCommit', { did: alice.did })
  assert.equal(after.status, 200)
  s.note('after the shared-node import', { postIsStillThere: record.status === 200, headMoved: after.json.cid !== before.json.cid })
  // SPEC 6.1: a server that refuses the tree keeps the repository as it was.
  if (shared.status !== 200) assert.equal(after.json.cid, before.json.cid)
})
