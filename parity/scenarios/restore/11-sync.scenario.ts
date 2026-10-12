// com.atproto.sync reads on a restored data directory (Appendix A), with
// the account statuses of SPEC 10.1 and the credentials that may read
// through them.

import assert from 'node:assert/strict'
import { readCarWithRoot } from '@atproto/repo'
import { loadSeed } from '../../src/lib/seed.ts'
import { get, raw } from '../../src/lib/reads.ts'
import { mintLegacyToken } from '../../src/lib/jwt.ts'
import { scenario } from '../../src/scenario.ts'

const POST = 'app.bsky.feed.post'
const NOTE = 'com.example.parity.note'
const UNKNOWN = 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa'
const ABSENT_CID = 'bafyreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku'

scenario('11-sync', { timeoutMs: 300_000 }, async (s) => {
  const seed = loadSeed(s.target.name)
  const { alice, big, deactivated, takendown, moderated } = seed.accounts
  const sync = (name: string) => `com.atproto.sync.${name}`

  // The export, whole and since each revision. The bytes must be the Reference's.
  const full = await get(s, 'getRepo', sync('getRepo'), { did: alice.did })
  assert.equal(full.headers.get('content-type'), 'application/vnd.ipld.car')
  const fullCar = await readCarWithRoot(full.bytes)
  for (const [i, rev] of seed.revs.entries()) await get(s, `getRepo since revision ${i + 1}`, sync('getRepo'), { did: alice.did, since: rev })
  await get(s, 'getRepo since a revision before the repository', sync('getRepo'), { did: alice.did, since: '3aaaaaaaaaaa2' })
  await get(s, 'getRepo since a revision after it', sync('getRepo'), { did: alice.did, since: '3zzzzzzzzzzzz' })
  const bigCar = await get(s, 'getRepo of a repository of several pages', sync('getRepo'), { did: big.did })
  assert.ok((await readCarWithRoot(bigCar.bytes)).blocks.size > seed.bigCount, 'every record and the tree above them')
  await get(s, 'getRepo with gzip refused', sync('getRepo'), { did: big.did }, { headers: { 'accept-encoding': 'identity' } })
  await get(s, 'getCheckout', sync('getCheckout'), { did: alice.did })

  const latest = await get(s, 'getLatestCommit', sync('getLatestCommit'), { did: alice.did })
  assert.equal(latest.json.cid, fullCar.root.toString())
  await get(s, 'getHead', sync('getHead'), { did: alice.did })
  await get(s, 'getRepoStatus', sync('getRepoStatus'), { did: alice.did })

  // A record with its proof, and proofs of absence.
  for (const rkey of seed.posts) await get(s, `getRecord ${rkey}`, sync('getRecord'), { did: alice.did, collection: POST, rkey })
  await get(s, 'getRecord of a deleted record', sync('getRecord'), { did: alice.did, collection: POST, rkey: seed.deletedPost })
  await get(s, 'getRecord below every key', sync('getRecord'), { did: alice.did, collection: 'a.b.c', rkey: 'x' })
  await get(s, 'getRecord above every key', sync('getRecord'), { did: alice.did, collection: 'zz.zz.zz', rkey: 'x' })
  for (const i of [0, 349, 699]) await get(s, `getRecord ${i} of the large repository`, sync('getRecord'), { did: big.did, collection: NOTE, rkey: `r${String(i).padStart(4, '0')}` })
  await get(s, 'getRecord absent from the large repository', sync('getRecord'), { did: big.did, collection: NOTE, rkey: 'r0350x' })
  await get(s, 'getRecord of a record that is taken down', sync('getRecord'), { did: moderated.did, collection: NOTE, rkey: seed.moderatedRecord.takenDown })

  // Blocks by CID, in the order asked and the other way round.
  const record = await s.query('a record CID', 'com.atproto.repo.getRecord', { repo: alice.did, collection: POST, rkey: seed.posts[0] }, { silent: true })
  const cids = [latest.json.cid, record.json.cid]
  await get(s, 'getBlocks', sync('getBlocks'), { did: alice.did, cids })
  await get(s, 'getBlocks, the other order', sync('getBlocks'), { did: alice.did, cids: [...cids].reverse() })
  await get(s, 'getBlocks, one CID twice', sync('getBlocks'), { did: alice.did, cids: [cids[1], cids[1]] })
  await get(s, 'getBlocks with a block the repository lacks', sync('getBlocks'), { did: alice.did, cids: [cids[1], ABSENT_CID] })
  await get(s, 'getBlocks with two blocks it lacks', sync('getBlocks'), { did: alice.did, cids: [ABSENT_CID, 'bafyreigyuveqaqy7kzdnhdaqzoztl4bq3fqrxfmlrizv2pi6ugs7j6zfmu'] })

  // Listings.
  const repos = await get(s, 'listRepos', sync('listRepos'))
  assert.equal(repos.json.repos.length, 6)
  const page = await get(s, 'listRepos, limit 2', sync('listRepos'), { limit: 2 })
  await get(s, 'listRepos, the next page', sync('listRepos'), { limit: 2, cursor: page.json.cursor })
  await get(s, 'listRepos past the end', sync('listRepos'), { cursor: '99999999999999::did:plc:zzzzzzzzzzzzzzzzzzzzzzzz' })
  for (const cursor of ['zzz', 'abc::did:plc:x', '1::', '::did:plc:x', '1::a::b', '5x::did:plc:x', '9999999999999999999::did:plc:x', '-1::did:plc:x', ' 12::did:plc:x']) await get(s, `listRepos, cursor ${JSON.stringify(cursor)}`, sync('listRepos'), { cursor })

  await get(s, 'listBlobs', sync('listBlobs'), { did: alice.did })
  await get(s, 'listBlobs, limit 1', sync('listBlobs'), { did: alice.did, limit: 1 })
  await get(s, 'listBlobs after the first', sync('listBlobs'), { did: alice.did, cursor: [seed.blobs.text, seed.blobs.png].sort()[0] })
  await get(s, 'listBlobs since the first blob record', sync('listBlobs'), { did: alice.did, since: seed.revs.at(-3) })
  await get(s, 'listBlobs since the last revision', sync('listBlobs'), { did: alice.did, since: seed.revs.at(-1) })
  await get(s, 'listBlobs of a repository without blobs', sync('listBlobs'), { did: big.did })
  await get(s, 'listBlobs with a blob that is taken down', sync('listBlobs'), { did: moderated.did })

  // Blobs, with the headers of SPEC 6.4.
  const text = await get(s, 'getBlob, text', sync('getBlob'), { did: alice.did, cid: seed.blobs.text })
  assert.equal(text.headers.get('content-security-policy'), "default-src 'none'; sandbox")
  assert.equal(text.headers.get('x-content-type-options'), 'nosniff')
  s.note('headers of a text blob', { length: text.headers.get('content-length'), disposition: text.headers.get('content-disposition') })
  await get(s, 'getBlob, an image', sync('getBlob'), { did: alice.did, cid: seed.blobs.png })
  await get(s, 'getBlob of another account', sync('getBlob'), { did: big.did, cid: seed.blobs.text })
  await get(s, 'getBlob with the CID of a record', sync('getBlob'), { did: alice.did, cid: record.json.cid })
  await get(s, 'getBlob that is taken down', sync('getBlob'), { did: moderated.did, cid: seed.blobs.moderatedGone })
  await get(s, 'getBlob beside one that is taken down', sync('getBlob'), { did: moderated.did, cid: seed.blobs.moderatedKept })

  // An account nobody hosts.
  for (const name of ['getRepo', 'getCheckout', 'getLatestCommit', 'getHead', 'getRepoStatus', 'listBlobs']) await get(s, `${name} for an unknown DID`, sync(name), { did: UNKNOWN })
  await get(s, 'getRecord for an unknown DID', sync('getRecord'), { did: UNKNOWN, collection: POST, rkey: 'x' })
  await get(s, 'getBlocks for an unknown DID', sync('getBlocks'), { did: UNKNOWN, cids: [ABSENT_CID] })
  await get(s, 'getBlob for an unknown DID', sync('getBlob'), { did: UNKNOWN, cid: seed.blobs.text })

  // Status: what anyone reads, what the owner reads, what an admin reads.
  const other = { auth: alice } as const
  for (const [label, account, blob] of [['deactivated', deactivated, seed.blobs.deactivated], ['taken down', takendown, seed.blobs.takendown]] as const) {
    const requests: [string, string, Record<string, string | string[]>][] = [
      ['getRepo', 'getRepo', { did: account.did }],
      ['getCheckout', 'getCheckout', { did: account.did }],
      ['getLatestCommit', 'getLatestCommit', { did: account.did }],
      ['getHead', 'getHead', { did: account.did }],
      ['getRepoStatus', 'getRepoStatus', { did: account.did }],
      ['getRecord', 'getRecord', { did: account.did, collection: NOTE, rkey: 'one' }],
      ['listBlobs', 'listBlobs', { did: account.did }],
      ['getBlob', 'getBlob', { did: account.did, cid: blob }],
    ]
    for (const [step, name, params] of requests) {
      await get(s, `${label}: ${step}, no credential`, sync(name), params)
      await get(s, `${label}: ${step}, another account`, sync(name), params, other)
      await get(s, `${label}: ${step}, the owner`, sync(name), params, { auth: account })
      await get(s, `${label}: ${step}, an admin`, sync(name), params, { auth: 'admin' })
    }
  }
  // A token with the scope a taken-down account is given: three methods accept it.
  const limited = mintLegacyToken(s.target.state.secrets.jwtSecret, { scope: 'com.atproto.takendown', sub: takendown.did, aud: s.target.serviceDid })
  for (const name of ['getRepo', 'listBlobs', 'getLatestCommit', 'getHead', 'getCheckout']) await get(s, `taken down: ${name}, the takendown scope`, sync(name), { did: takendown.did }, { auth: limited })
  await get(s, 'taken down: getBlob, the takendown scope', sync('getBlob'), { did: takendown.did, cid: seed.blobs.takendown }, { auth: limited })

  // Credentials that do not verify.
  const did = { did: alice.did }
  await get(s, 'a token that is not a JWT', sync('getLatestCommit'), did, { auth: 'abc' })
  await get(s, 'a token signed with another secret', sync('getLatestCommit'), did, { auth: mintLegacyToken('another secret', { scope: 'com.atproto.access', sub: alice.did, aud: s.target.serviceDid }) })
  await get(s, 'an expired token', sync('getLatestCommit'), did, { auth: mintLegacyToken(s.target.state.secrets.jwtSecret, { scope: 'com.atproto.access', sub: alice.did, aud: s.target.serviceDid, expiresInSeconds: -60 }) })
  await get(s, 'a token for another audience', sync('getLatestCommit'), did, { auth: mintLegacyToken(s.target.state.secrets.jwtSecret, { scope: 'com.atproto.access', sub: alice.did, aud: 'did:web:elsewhere.test' }) })
  await get(s, 'a refresh token', sync('getLatestCommit'), did, { auth: alice.refreshJwt })
  await get(s, 'a token with the refresh scope and the access type', sync('getLatestCommit'), did, { auth: mintLegacyToken(s.target.state.secrets.jwtSecret, { scope: 'com.atproto.refresh', sub: alice.did, aud: s.target.serviceDid }) })
  await get(s, 'an app-password token', sync('getLatestCommit'), did, { auth: mintLegacyToken(s.target.state.secrets.jwtSecret, { scope: 'com.atproto.appPass', sub: alice.did, aud: s.target.serviceDid }) })
  await get(s, 'a wrong admin password', sync('getLatestCommit'), did, { headers: { authorization: `Basic ${Buffer.from('admin:wrong').toString('base64')}` } })
  await get(s, 'basic credentials of another user', sync('getLatestCommit'), did, { headers: { authorization: `Basic ${Buffer.from(`root:${s.target.state.secrets.adminPassword}`).toString('base64')}` } })
  await get(s, 'basic credentials that are not base64', sync('getLatestCommit'), did, { headers: { authorization: 'Basic ***' } })
  await get(s, 'an authorization type nobody knows', sync('getLatestCommit'), did, { headers: { authorization: 'Weird abc' } })
  await get(s, 'an authorization header of three parts', sync('getLatestCommit'), did, { headers: { authorization: 'Bearer a b' } })
  await get(s, 'an authorization header of one part', sync('getLatestCommit'), did, { headers: { authorization: 'Bearer' } })
  // Parameters are checked before the credential.
  await raw(s, 'a bad parameter with a bad token', '/xrpc/com.atproto.sync.getLatestCommit?did=nope', { auth: 'abc' })
})
