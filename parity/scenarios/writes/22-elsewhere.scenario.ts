// Data from elsewhere: what each server answers where the Candidate checks
// more than the Reference at the pin: the import checks of SPEC 2.3 (a
// malformed tree, record blocks that are not canonical DAG-CBOR, a commit
// that another key signed or that names another account) and a blob reference
// with size zero or an empty MIME type.
//
// Every case runs on an account of its own, because a server that accepts
// an import has replaced the repository. A CAR is signed with the account's
// own signing key unless the case is about the signature, so that the
// Candidate's refusal is about the thing the case names.

import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { TID } from '@atproto/common-web'
import { Secp256k1Keypair } from '@atproto/crypto'
import { BlockMap, blocksToCarFile, signCommit } from '@atproto/repo'
import { CID } from 'multiformats/cid'
import { sha256 } from 'multiformats/hashes/sha2'
import { scenario } from '../../src/scenario.ts'
import type { Account, Scenario } from '../../src/scenario.ts'

const NOTE = 'com.example.parity.note'
const CAR = 'application/vnd.ipld.car'
const DAG_CBOR = 0x71

type Cid = Awaited<ReturnType<BlockMap['add']>>
type Keypair = Awaited<ReturnType<typeof Secp256k1Keypair.create>>

/** The layer of a key in the tree: half the leading zero bits of its SHA-256. */
function layerOf(key: string): number {
  const digest = createHash('sha256').update(key).digest()
  let zeros = 0
  for (const byte of digest) {
    if (byte === 0) {
      zeros += 8
      continue
    }
    zeros += Math.clz32(byte) - 24
    break
  }
  return Math.floor(zeros / 2)
}

/** The first key `<NOTE>/<prefix><n>` at `layer`. */
function keyAt(layer: number, prefix: string): string {
  for (let n = 0; n < 1_000_000; n++) {
    const key = `${NOTE}/${prefix}${n}`
    if (layerOf(key) === layer) return key
  }
  throw new Error(`no key at layer ${layer}`)
}

/** The account's signing key, read from its actor store. */
async function signingKeypair(s: Scenario, did: string): Promise<Keypair> {
  const shard = createHash('sha256').update(did).digest('hex').slice(0, 2)
  const read = s.target.exec(['od', '-An', '-v', '-tx1', `/app/data/actors/${shard}/${did}/key`])
  assert.equal(read.status, 0, read.stderr)
  const hex = read.stdout.replace(/\s+/g, '')
  assert.equal(hex.length, 64, 'a 32-byte key')
  return Secp256k1Keypair.import(hex)
}

const text = new TextEncoder()
const entry = (key: string, value: Cid, tree: Cid | null = null) => ({ p: 0, k: text.encode(key), v: value, t: tree })

async function carOf(blocks: BlockMap, did: string, data: Cid, keypair: Keypair): Promise<Uint8Array> {
  const commit = await signCommit({ did, version: 3, rev: TID.nextStr(), prev: null, data }, keypair)
  return blocksToCarFile(await blocks.add(commit), blocks)
}

/** Adds a block of bytes that the library would not write itself. */
async function addRaw(blocks: BlockMap, bytes: Uint8Array): Promise<Cid> {
  const cid = CID.createV1(DAG_CBOR, await sha256.digest(bytes)) as unknown as Cid
  blocks.set(cid, bytes)
  return cid
}

/** Imports `car` into a fresh account's repository and records what the server did. */
async function importCase(s: Scenario, step: string, build: (account: Account, keypair: Keypair) => Promise<Uint8Array>): Promise<void> {
  const account = await s.createAccount(step.replace(/\W+/g, '-'))
  const keypair = await signingKeypair(s, account.did)
  const before = await s.http('head before', { path: '/xrpc/com.atproto.sync.getLatestCommit', query: { did: account.did }, silent: true })
  const imported = await s.http(step, {
    method: 'POST',
    path: '/xrpc/com.atproto.repo.importRepo',
    body: await build(account, keypair),
    contentType: CAR,
    auth: account,
    silent: true,
  })
  const after = await s.http('head after', { path: '/xrpc/com.atproto.sync.getLatestCommit', query: { did: account.did }, silent: true })
  s.note(step, { status: imported.status, error: imported.json?.error ?? null, headMoved: after.json?.cid !== before.json?.cid })
  // A server that refuses the import keeps the repository as it was.
  if (imported.status !== 200) assert.equal(after.json.cid, before.json.cid, `${step}: refused and unchanged`)
}

scenario('22-elsewhere', { timeoutMs: 300_000 }, async (s) => {
  // The control: a small valid repository, signed with the account's key.
  await importCase(s, 'import of a valid repository', async (account, keypair) => {
    const blocks = new BlockMap()
    const record = await blocks.add({ $type: NOTE, text: 'valid' })
    return carOf(blocks, account.did, await blocks.add({ l: null, e: [entry(keyAt(0, 'a'), record)] }), keypair)
  })

  // A tree that is not a Merkle search tree.
  await importCase(s, 'a key at the wrong layer', async (account, keypair) => {
    const blocks = new BlockMap()
    const record = await blocks.add({ $type: NOTE, text: 'layer' })
    // Two keys of different layers in one node, in key order.
    const keys = [keyAt(0, 'a'), keyAt(1, 'b')].sort()
    const node = { l: null, e: [entry(keys[0]!, record), { ...entry(keys[1]!, record), p: 0 }] }
    return carOf(blocks, account.did, await blocks.add(node), keypair)
  })
  await importCase(s, 'keys out of order across nodes', async (account, keypair) => {
    const blocks = new BlockMap()
    const record = await blocks.add({ $type: NOTE, text: 'order' })
    // The left subtree holds a key that sorts after the key above it.
    const above = keyAt(1, 'a')
    const below = keyAt(0, 'z')
    assert.ok(below > above)
    const left = await blocks.add({ l: null, e: [entry(below, record)] })
    return carOf(blocks, account.did, await blocks.add({ l: left, e: [entry(above, record)] }), keypair)
  })
  await importCase(s, 'an empty node below the root', async (account, keypair) => {
    const blocks = new BlockMap()
    const record = await blocks.add({ $type: NOTE, text: 'empty' })
    const empty = await blocks.add({ l: null, e: [] })
    return carOf(blocks, account.did, await blocks.add({ l: empty, e: [entry(keyAt(1, 'a'), record)] }), keypair)
  })

  // A record block that is not canonical DAG-CBOR.
  const withRecord = (bytes: number[]) => async (account: Account, keypair: Keypair) => {
    const blocks = new BlockMap()
    const record = await addRaw(blocks, Uint8Array.from(bytes))
    return carOf(blocks, account.did, await blocks.add({ l: null, e: [entry(keyAt(0, 'a'), record)] }), keypair)
  }
  const type = [0x65, ...text.encode('$type'), 0x77, ...text.encode(NOTE)]
  // {"$type": NOTE, "a": 1} with the longer key first.
  await importCase(s, 'map keys out of canonical order', withRecord([0xa2, ...type, 0x61, 0x61, 0x01]))
  // {"a": 1.0, "$type": NOTE}: a float with a whole value.
  await importCase(s, 'a float with a whole value', withRecord([0xa2, 0x61, 0x61, 0xfb, 0x3f, 0xf0, 0, 0, 0, 0, 0, 0, ...type]))
  // {"a": undefined, "$type": NOTE}.
  await importCase(s, 'undefined', withRecord([0xa2, 0x61, 0x61, 0xf7, ...type]))

  // The commit.
  const valid = async (did: string, keypair: Keypair) => {
    const blocks = new BlockMap()
    const record = await blocks.add({ $type: NOTE, text: 'commit' })
    return carOf(blocks, did, await blocks.add({ l: null, e: [entry(keyAt(0, 'a'), record)] }), keypair)
  }
  await importCase(s, 'a commit that another key signed', async (account) => valid(account.did, await Secp256k1Keypair.create()))
  await importCase(s, 'a commit for another account', async (_account, keypair) => valid('did:plc:aaaaaaaaaaaaaaaaaaaaaaaa', keypair))

  // A blob reference with a size of zero. An empty upload is accepted (SPEC
  // 6.4); a record that references it states size 0.
  const owner = await s.createAccount('blob-owner')
  const empty = await s.http('upload of zero bytes', { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body: new Uint8Array(0), contentType: 'text/plain', auth: owner, silent: true })
  assert.equal(empty.status, 200, empty.text)
  const zero = await s.procedure('record referencing an empty blob', 'com.atproto.repo.createRecord', { repo: owner.did, collection: NOTE, rkey: 'zero', record: { $type: NOTE, file: empty.json.blob } }, { auth: owner, silent: true })
  s.note('a blob reference with size zero', { status: zero.status, error: zero.json?.error ?? null })
  const noType = await s.procedure('record with an empty mimeType', 'com.atproto.repo.createRecord', { repo: owner.did, collection: NOTE, rkey: 'untyped', record: { $type: NOTE, file: { ...empty.json.blob, mimeType: '' } } }, { auth: owner, silent: true })
  s.note('a blob reference with an empty mimeType', { status: noType.status, error: noType.json?.error ?? null })
})
