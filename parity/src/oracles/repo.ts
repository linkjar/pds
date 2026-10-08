// Oracle O2 (SPEC 17.3, 6.6): repository CIDs. The harness does not trust
// the server's word for its own root. It exports the repository, verifies
// the commit signature against the key in the PLC document, rebuilds the MST
// from the records with the SDK's implementation, and compares the root.

import assert from 'node:assert/strict'
import * as dagCbor from '@ipld/dag-cbor'
import { MST, MemoryBlockstore, readCarWithRoot, verifyRepoCar } from '@atproto/repo'
import type { Account, Scenario } from '../scenario.ts'
import { HOSTS } from '../stack/targets.ts'

export type RepoCheck = {
  /** CID of the signed commit. */
  commit: string
  /** MST root, the `data` field of the commit. */
  data: string
  rev: string
  records: number
}

/** The account's signing key as a did:key, from the PLC directory. */
export async function signingKey(s: Scenario, did: string): Promise<string> {
  const data = await s.http('plc data', { host: HOSTS.plc, path: `/${did}/data`, silent: true })
  assert.equal(data.status, 200, `PLC has no data for ${did}`)
  const key = data.json.verificationMethods?.atproto
  assert.ok(typeof key === 'string' && key.startsWith('did:key:'), 'PLC data carries the atproto signing key')
  return key
}

/**
 * Checks the repository of an account and records the outcome. Every write
 * scenario calls this after its writes.
 */
export async function checkRepo(
  s: Scenario,
  step: string,
  account: Pick<Account, 'did'>,
  /** False when the scenario let the server choose record keys, so the root cannot match across targets. */
  opts: { deterministic?: boolean } = {},
): Promise<RepoCheck> {
  const didKey = await signingKey(s, account.did)
  const exported = await s.http('export repo', { path: '/xrpc/com.atproto.sync.getRepo', query: { did: account.did }, silent: true })
  assert.equal(exported.status, 200, 'getRepo')
  assert.equal(exported.headers.get('content-type'), 'application/vnd.ipld.car')

  // Signature and structure: throws unless the commit is signed by the PLC
  // key and every MST node and record block is present.
  const verified = await verifyRepoCar(exported.bytes, account.did, didKey)

  const car = await readCarWithRoot(exported.bytes)
  const commitBytes = car.blocks.get(car.root)
  assert.ok(commitBytes, 'the CAR holds its root block')
  const commit = dagCbor.decode(commitBytes) as Record<string, unknown>
  assert.equal(commit.version, 3, 'commit version 3 (SPEC 6.1)')
  assert.equal(commit.did, account.did)
  assert.ok('prev' in commit && commit.prev === null, 'prev is present and null (SPEC 6.1)')
  assert.ok(commit.sig instanceof Uint8Array && commit.sig.byteLength === 64, 'a 64-byte compact signature')

  // Rebuild the tree from nothing but the record keys and CIDs.
  let tree = await MST.create(new MemoryBlockstore())
  const keys = verified.creates.map((create) => ({ key: `${create.collection}/${create.rkey}`, cid: create.cid }))
  for (const { key, cid } of keys) tree = await tree.add(key, cid)
  const rebuilt = (await tree.getPointer()).toString()
  assert.equal(rebuilt, String(commit.data), 'the MST root rebuilt from the records equals the signed root')

  const latest = await s.http('latest commit', {
    path: '/xrpc/com.atproto.sync.getLatestCommit',
    query: { did: account.did },
    silent: true,
  })
  assert.equal(latest.json.cid, car.root.toString(), 'getLatestCommit names the exported commit')
  assert.equal(latest.json.rev, commit.rev)

  const check: RepoCheck = { commit: car.root.toString(), data: rebuilt, rev: String(commit.rev), records: keys.length }
  // The commit CID differs between targets: it covers the DID, the `rev` and
  // the signature. The MST root covers only record keys and record CIDs, so
  // with explicit keys and fixed content it must be identical everywhere.
  s.note(step, {
    signatureValid: true,
    rootRebuilt: true,
    records: keys.map(({ key }) => key).sort(),
    commit: check.commit,
    rev: check.rev,
  })
  if (opts.deterministic !== false) s.exact(`${step}: MST root`, check.data)
  return check
}
