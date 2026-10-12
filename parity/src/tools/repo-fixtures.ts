// Fixtures for the Rust repository engine, written by the Reference's own
// library: `@atproto/repo` at the pin, the engine behind oracle O2. The Rust
// tests replay each commit and must produce the same tree root, the same
// commit CID, the same sets of blocks and the same export bytes.
//
//   node src/tools/repo-fixtures.ts ../crates/pds-repo/tests/fixtures
//
// Everything is derived from fixed inputs. One value is not: `rev` comes
// from the library's clock, so the fixture records the `rev` of each commit
// and the Rust side signs with the same one. Signatures are deterministic
// (RFC 6979), so a fixed key gives fixed commit CIDs for a given `rev`.

import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { P256Keypair, Secp256k1Keypair } from '@atproto/crypto'
import {
  BlockMap,
  MST,
  MemoryBlockstore,
  Repo,
  WriteOpAction,
  blocksToCarFile,
  cidForRecord,
  concatBytesAsync,
  getFullRepo,
  signCommit,
} from '@atproto/repo'
import type { CommitData, RecordWriteOp } from '@atproto/repo'

const argument = process.argv[2]
if (!argument) throw new Error('usage: node src/tools/repo-fixtures.ts <output directory>')
const out: string = argument
mkdirSync(out, { recursive: true })

const DID = 'did:plc:ewvi7nxzyoun6zhxrhs64oiz'
const KEY_HEX = '9085d2bef69286a6cbb51623c8fa258629945cd55ca705cc4e66700396894e0c'
const hex = (bytes: Uint8Array) => Buffer.from(bytes).toString('hex')
const sha256 = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex')
const pkg = (name: string) => JSON.parse(readFileSync(new URL(`../../node_modules/${name}/package.json`, import.meta.url), 'utf8')).version as string

/** A small fixed generator (mulberry32), so that the fixture is the same on every run. */
function prng(seed: number) {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const COLLECTIONS = ['app.bsky.feed.post', 'app.bsky.feed.like', 'com.example.parity.note']
const B32 = '234567abcdefghijklmnopqrstuvwxyz'

function rkey(random: () => number): string {
  let s = '3'
  for (let i = 0; i < 12; i++) s += B32[Math.floor(random() * 32)]
  return s
}

function record(collection: string, n: number, random: () => number): Record<string, unknown> {
  // Shapes that exercise the encoder: nesting, an empty list, null, a
  // negative and a large integer, a key longer than 23 bytes, non-ASCII text.
  return {
    $type: collection,
    n,
    text: `record ${n} of a repository that grows · ${'é'.repeat(n % 5)}`,
    tags: n % 3 === 0 ? [] : ['a', `tag-${n}`],
    nested: { deep: { value: -n, big: 9007199254740991 - n }, flag: n % 2 === 0, none: null },
    aKeyThatIsLongerThanTwentyThreeBytes: Math.floor(random() * 1_000_000),
  }
}

type FixtureWrite = { action: 'create' | 'update' | 'delete'; key: string; record?: unknown; cid?: string }

async function main() {
  const keypair = await Secp256k1Keypair.import(KEY_HEX, { exportable: true })

  // --- signatures: both curves, fixed keys and messages -------------------
  const p256 = await P256Keypair.import(KEY_HEX, { exportable: true })
  const messages = ['', 'hello world', 'a'.repeat(1000)].map((m) => new TextEncoder().encode(m))
  const signatures = [] as unknown[]
  for (const message of messages) {
    signatures.push({
      messageHex: hex(message),
      secp256k1: hex(await keypair.sign(message)),
      p256: hex(await p256.sign(message)),
    })
  }

  // --- trees: roots for sets of keys -------------------------------------
  const trees = [] as unknown[]
  for (const [name, count, seed] of [
    ['empty', 0, 1],
    ['one', 1, 2],
    ['small', 25, 3],
    ['medium', 300, 4],
    ['large', 2000, 5],
  ] as const) {
    const random = prng(seed)
    const entries: [string, string][] = []
    let tree = await MST.create(new MemoryBlockstore())
    for (let i = 0; i < count; i++) {
      const collection = COLLECTIONS[i % COLLECTIONS.length]!
      const key = `${collection}/${rkey(random)}`
      const cid = await cidForRecord({ $type: collection, i })
      tree = await tree.add(key, cid)
      entries.push([key, cid.toString()])
    }
    trees.push({ name, entries, root: (await tree.getPointer()).toString(), nodes: (await tree.allNodes()).filter((n) => n.isTree()).length })
  }

  // --- commits: one repository, a sequence of batches ---------------------
  const random = prng(42)
  const storage = new MemoryBlockstore()
  const live = new Map<string, number>()
  let counter = 0
  const create = (collection: string): { op: RecordWriteOp; fx: FixtureWrite } => {
    const n = counter++
    const key = rkey(random)
    const value = record(collection, n, random)
    live.set(`${collection}/${key}`, n)
    return { op: { action: WriteOpAction.Create, collection, rkey: key, record: value } as RecordWriteOp, fx: { action: 'create', key: `${collection}/${key}`, record: value } }
  }
  const pick = (): string => [...live.keys()].sort()[Math.floor(random() * live.size)]!
  const update = (): { op: RecordWriteOp; fx: FixtureWrite } => {
    const path = pick()
    const [collection, key] = path.split('/') as [string, string]
    const value = record(collection, counter++, random)
    return { op: { action: WriteOpAction.Update, collection, rkey: key, record: value } as RecordWriteOp, fx: { action: 'update', key: path, record: value } }
  }
  const remove = (): { op: RecordWriteOp; fx: FixtureWrite } => {
    const path = pick()
    const [collection, key] = path.split('/') as [string, string]
    live.delete(path)
    return { op: { action: WriteOpAction.Delete, collection, rkey: key } as RecordWriteOp, fx: { action: 'delete', key: path } }
  }
  const many = <T>(count: number, fn: () => T): T[] => Array.from({ length: count }, fn)
  const coll = () => COLLECTIONS[Math.floor(random() * COLLECTIONS.length)]!

  const plans: { name: string; writes: () => { op: RecordWriteOp; fx: FixtureWrite }[] }[] = [
    { name: 'one create', writes: () => [create(coll())] },
    { name: 'a second create', writes: () => [create(coll())] },
    { name: 'batch of 60 creates', writes: () => many(60, () => create(coll())) },
    { name: 'one update', writes: () => [update()] },
    { name: 'one delete', writes: () => [remove()] },
    { name: 'batch of 200 creates', writes: () => many(200, () => create(coll())) },
    { name: 'mixed batch', writes: () => [...many(20, () => create(coll())), ...many(15, update), ...many(15, remove)] },
    { name: 'batch of 40 deletes', writes: () => many(40, remove) },
    { name: 'batch of 200 creates in one collection', writes: () => many(200, () => create(COLLECTIONS[2]!)) },
    { name: 'create and delete of the same key', writes: () => {
        const made = create(coll())
        const [collection, key] = made.fx.key.split('/') as [string, string]
        live.delete(made.fx.key)
        return [made, { op: { action: WriteOpAction.Delete, collection, rkey: key } as RecordWriteOp, fx: { action: 'delete' as const, key: made.fx.key } }]
      } },
    { name: 'batch of 120 deletes', writes: () => many(120, remove) },
    { name: 'one create after the deletes', writes: () => [create(coll())] },
  ]

  const describe = async (commit: CommitData, fx: FixtureWrite[]) => ({
    rev: commit.rev,
    since: commit.since,
    prev: commit.prev?.toString() ?? null,
    writes: fx,
    cid: commit.cid.toString(),
    newBlocks: commit.newBlocks.cids().map(String).sort(),
    relevantBlocks: commit.relevantBlocks.cids().map(String),
    removedCids: commit.removedCids.toList().map(String).sort(),
    relevantCarSha256: sha256(await blocksToCarFile(commit.cid, commit.relevantBlocks)),
    relevantCarLength: (await blocksToCarFile(commit.cid, commit.relevantBlocks)).byteLength,
  })
  const withCids = async (fx: FixtureWrite[]) => {
    for (const write of fx) if (write.record) write.cid = (await cidForRecord(write.record as never)).toString()
    return fx
  }

  const commits = [] as unknown[]
  const first = [create(COLLECTIONS[0]!)]
  const init = await Repo.formatInitCommit(storage, DID, keypair, first.map((w) => w.op) as never)
  let repo = await Repo.createFromCommit(storage, init)
  commits.push({ name: 'the first commit', ...(await describe(init, await withCids(first.map((w) => w.fx)))), data: repo.commit.data.toString(), records: live.size })
  for (const plan of plans) {
    const writes = plan.writes()
    const commit = await repo.formatCommit(writes.map((w) => w.op), keypair)
    repo = await repo.applyCommit(commit)
    commits.push({ name: plan.name, ...(await describe(commit, await withCids(writes.map((w) => w.fx)))), data: repo.commit.data.toString(), records: live.size })
  }
  const exported = await concatBytesAsync(getFullRepo(storage, repo.cid))
  writeFileSync(join(out, 'export.car'), exported)

  // --- trees that reference one node more than once -----------------------
  // The shapes of upstream's packages/repo/tests/mst-dag.test.ts at
  // a7c8604d (0.5.37), built with the library at the pin. Their blocks are
  // what an attacker would upload; the pin walks them without a visited set.
  const enc = new TextEncoder()
  const leafEntry = (key: string, value: unknown, t: unknown = null) => ({ p: 0, k: enc.encode(key), v: value, t })
  const commitFor = async (blocks: BlockMap, data: unknown) =>
    blocks.add((await signCommit({ did: DID, version: 3, rev: '3jqfcqzm3fo2j', prev: null, data } as never, keypair)) as never)
  const dags = [] as unknown[]
  for (const depth of [4, 10, 14, 18, 40]) {
    const blocks = new BlockMap()
    const value = await blocks.add({ $type: 'com.example.dummy' })
    let level: unknown[] = []
    for (let j = 0; j < 2; j++) level.push(await blocks.add({ l: null, e: [leafEntry(`com.example.lvl0/${j}`, value)] } as never))
    for (let i = 1; i < depth; i++) {
      const next: unknown[] = []
      for (let j = 0; j < 2; j++) next.push(await blocks.add({ l: level[j], e: [leafEntry(`com.example.lvl${i}/${j}`, value, level[(j + 1) % 2])] } as never))
      level = next
    }
    const commit = await commitFor(blocks, level[0])
    const file = `dag-diamond-${depth}.car`
    writeFileSync(join(out, file), await blocksToCarFile(commit, blocks))
    dags.push({ file, shape: 'diamond', depth, blocks: blocks.size, leafVisitsWithoutAGuard: 2 ** depth })
  }
  {
    const [n, m] = [50, 50]
    const blocks = new BlockMap()
    const value = await blocks.add({ $type: 'com.example.dummy' })
    let chain = await blocks.add({ l: null, e: [leafEntry('com.example.bottom/a', value)] } as never)
    for (let i = 0; i < m; i++) chain = await blocks.add({ l: chain, e: [] } as never)
    const e = Array.from({ length: n }, (_, i) => leafEntry(`com.example.wide/${String(i).padStart(6, '0')}`, value, chain))
    const root = await blocks.add({ l: null, e } as never)
    const commit = await commitFor(blocks, root)
    const file = `dag-shared-chain-${n}x${m}.car`
    writeFileSync(join(out, file), await blocksToCarFile(commit, blocks))
    dags.push({ file, shape: 'shared chain', n, m, blocks: blocks.size, nodeVisitsWithoutAGuard: n * m })
  }

  writeFileSync(
    join(out, 'reference.json'),
    // One line: the file is data for a test, and it is long.
    `${JSON.stringify(
      {
        source: {
          note: 'Written by parity/src/tools/repo-fixtures.ts. Do not edit by hand.',
          atprotoRepo: pkg('@atproto/repo'),
          atprotoCrypto: pkg('@atproto/crypto'),
          upstreamCommit: '7ca16cc6989f8247637615aca17c5abb911b8fb1',
        },
        did: DID,
        privateKeyHex: KEY_HEX,
        didKey: keypair.did(),
        p256DidKey: p256.did(),
        signatures,
        trees,
        commits,
        export: { file: 'export.car', commit: repo.cid.toString(), records: live.size, sha256: sha256(exported), length: exported.byteLength },
        dags,
      },
    )}\n`,
  )
  console.log(`wrote ${commits.length} commits, ${trees.length} trees, ${dags.length} shared-node CARs and export.car (${exported.byteLength} bytes) to ${out}`)
}

await main()
