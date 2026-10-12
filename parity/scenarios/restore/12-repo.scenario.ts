// com.atproto.repo reads on a restored data directory, describeServer and
// resolveHandle (Appendix A; SPEC 4.4 for a repository hosted elsewhere).

import assert from 'node:assert/strict'
import { loadSeed } from '../../src/lib/seed.ts'
import { get } from '../../src/lib/reads.ts'
import { scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const POST = 'app.bsky.feed.post'
const NOTE = 'com.example.parity.note'
const UNKNOWN = 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa'
const ABSENT_CID = 'bafyreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku'

scenario('12-repo', { timeoutMs: 300_000 }, async (s) => {
  const seed = loadSeed(s.target.name)
  const { alice, big, deactivated, takendown, moderated, renamed } = seed.accounts
  const repo = (name: string) => `com.atproto.repo.${name}`

  // getRecord.
  const first = await get(s, 'getRecord', repo('getRecord'), { repo: alice.did, collection: POST, rkey: seed.posts[0] })
  assert.equal(first.json.value.text, 'post 1')
  await get(s, 'getRecord by handle', repo('getRecord'), { repo: alice.handle, collection: POST, rkey: seed.posts[0] })
  await get(s, 'getRecord by a handle in capitals', repo('getRecord'), { repo: alice.handle.toUpperCase(), collection: POST, rkey: seed.posts[0] })
  await get(s, 'getRecord with the matching cid', repo('getRecord'), { repo: alice.did, collection: POST, rkey: seed.posts[0], cid: first.json.cid })
  await get(s, 'getRecord with another cid', repo('getRecord'), { repo: alice.did, collection: POST, rkey: seed.posts[0], cid: ABSENT_CID })
  await get(s, 'getRecord that was replaced', repo('getRecord'), { repo: alice.did, collection: POST, rkey: seed.posts[1] })
  await get(s, 'getRecord that was deleted', repo('getRecord'), { repo: alice.did, collection: POST, rkey: seed.deletedPost })
  await get(s, 'getRecord with a blob', repo('getRecord'), { repo: alice.did, collection: NOTE, rkey: 'text' })
  await get(s, 'getRecord with nested data and bytes', repo('getRecord'), { repo: alice.did, collection: NOTE, rkey: 'image' })
  await get(s, 'getRecord that is taken down', repo('getRecord'), { repo: moderated.did, collection: NOTE, rkey: seed.moderatedRecord.takenDown })
  await get(s, 'getRecord beside one that is taken down', repo('getRecord'), { repo: moderated.did, collection: NOTE, rkey: seed.moderatedRecord.kept })
  await get(s, 'getRecord that is taken down, as an admin', repo('getRecord'), { repo: moderated.did, collection: NOTE, rkey: seed.moderatedRecord.takenDown }, { auth: 'admin' })
  // A repository this server does not serve goes to the AppView, without a credential (SPEC 4.4).
  const mark = await s.fixtures.cursor()
  await get(s, 'getRecord in a repository hosted elsewhere', repo('getRecord'), { repo: UNKNOWN, collection: POST, rkey: seed.posts[0] })
  await get(s, 'getRecord of a deactivated account', repo('getRecord'), { repo: deactivated.did, collection: NOTE, rkey: 'one' })
  await get(s, 'getRecord of an account that is taken down', repo('getRecord'), { repo: takendown.did, collection: NOTE, rkey: 'one' })
  await get(s, 'getRecord of a deactivated account, as its owner', repo('getRecord'), { repo: deactivated.did, collection: NOTE, rkey: 'one' }, { auth: deactivated })
  // What the AppView answers comes back as it is: a record, a refusal, a failure.
  const elsewhere = { repo: UNKNOWN, collection: POST, rkey: seed.posts[0] }
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: repo('getRecord'), body: { uri: `at://${UNKNOWN}/${POST}/${seed.posts[0]}`, cid: ABSENT_CID, value: { $type: POST, text: 'from the AppView' } } })
  await get(s, 'getRecord that the AppView has', repo('getRecord'), elsewhere)
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: repo('getRecord'), status: 400, body: { error: 'RecordNotFound', message: 'Could not locate record' } })
  await get(s, 'getRecord that the AppView lacks', repo('getRecord'), elsewhere)
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: repo('getRecord'), status: 500, body: { error: 'InternalServerError', message: 'Internal Server Error' } })
  await get(s, 'getRecord when the AppView fails', repo('getRecord'), elsewhere)
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: repo('getRecord'), body: {} })
  const forwarded = await s.fixtures.requests(HOSTS.appview, mark)
  s.note('requests forwarded to the AppView', forwarded.map((request) => ({ path: request.path, query: request.query, authorization: request.headers.authorization ?? null })))

  // listRecords.
  const all = await get(s, 'listRecords', repo('listRecords'), { repo: alice.did, collection: POST })
  assert.deepEqual(all.json.records.map((r: { uri: string }) => r.uri.split('/').at(-1)), [...seed.posts].reverse())
  const page = await get(s, 'listRecords, limit 2', repo('listRecords'), { repo: alice.did, collection: POST, limit: 2 })
  await get(s, 'listRecords, the next page', repo('listRecords'), { repo: alice.did, collection: POST, limit: 2, cursor: page.json.cursor })
  await get(s, 'listRecords, reverse', repo('listRecords'), { repo: alice.did, collection: POST, reverse: true, limit: 2 })
  await get(s, 'listRecords, reverse with a cursor', repo('listRecords'), { repo: alice.did, collection: POST, reverse: true, cursor: seed.posts[0] })
  await get(s, 'listRecords, a cursor past the end', repo('listRecords'), { repo: alice.did, collection: POST, cursor: '0' })
  await get(s, 'listRecords of an empty collection', repo('listRecords'), { repo: alice.did, collection: 'app.bsky.feed.repost' })
  await get(s, 'listRecords by handle', repo('listRecords'), { repo: alice.handle, collection: NOTE })
  await get(s, 'listRecords, limit 100 of the large repository', repo('listRecords'), { repo: big.did, collection: NOTE, limit: 100 })
  await get(s, 'listRecords with a record that is taken down', repo('listRecords'), { repo: moderated.did, collection: NOTE })
  await get(s, 'listRecords of an unknown repository', repo('listRecords'), { repo: UNKNOWN, collection: POST })
  await get(s, 'listRecords of a handle nobody has', repo('listRecords'), { repo: `nobody${HOSTS.handleDomain}`, collection: POST })
  await get(s, 'listRecords of a deactivated account', repo('listRecords'), { repo: deactivated.did, collection: NOTE })
  await get(s, 'listRecords of an account that is taken down', repo('listRecords'), { repo: takendown.did, collection: NOTE })
  // The deprecated range parameters are still in the lexicon.
  await get(s, 'listRecords with rkeyStart and rkeyEnd', repo('listRecords'), { repo: alice.did, collection: POST, rkeyStart: seed.posts[0], rkeyEnd: seed.posts[3] })

  // describeRepo.
  const described = await get(s, 'describeRepo', repo('describeRepo'), { repo: alice.did })
  assert.equal(described.json.handleIsCorrect, true)
  assert.deepEqual(described.json.collections, ['app.bsky.feed.like', POST, NOTE])
  await get(s, 'describeRepo by handle', repo('describeRepo'), { repo: alice.handle })
  await get(s, 'describeRepo of a repository without records', repo('describeRepo'), { repo: renamed.did })
  await get(s, 'describeRepo by the new handle', repo('describeRepo'), { repo: renamed.handle })
  await get(s, 'describeRepo by the old handle', repo('describeRepo'), { repo: seed.renamedFrom })
  await get(s, 'describeRepo of a deactivated account', repo('describeRepo'), { repo: deactivated.did })
  await get(s, 'describeRepo of an account that is taken down', repo('describeRepo'), { repo: takendown.did })
  await get(s, 'describeRepo of an unknown DID', repo('describeRepo'), { repo: UNKNOWN })

  // describeServer.
  const server = await get(s, 'describeServer', 'com.atproto.server.describeServer')
  assert.equal(server.json.did, s.target.serviceDid)
  await get(s, 'describeServer with a parameter it does not have', 'com.atproto.server.describeServer', { x: 1 })

  // resolveHandle: a hosted handle from the database, any other from the AppView (SPEC 9.2).
  const resolve = 'com.atproto.identity.resolveHandle'
  assert.equal((await get(s, 'resolveHandle', resolve, { handle: alice.handle })).json.did, alice.did)
  await get(s, 'resolveHandle in capitals', resolve, { handle: alice.handle.toUpperCase() })
  await get(s, 'resolveHandle of a renamed account', resolve, { handle: renamed.handle })
  await get(s, 'resolveHandle of the handle it had before', resolve, { handle: seed.renamedFrom })
  await get(s, 'resolveHandle of a deactivated account', resolve, { handle: deactivated.handle })
  await get(s, 'resolveHandle of an account that is taken down', resolve, { handle: takendown.handle })
  await get(s, 'resolveHandle of a hosted handle nobody has', resolve, { handle: `nobody${HOSTS.handleDomain}` })
  await get(s, 'resolveHandle of the bare hosted domain', resolve, { handle: HOSTS.handleDomain.slice(1) })
  // Any other handle is the AppView's to resolve. The server passes on what
  // the AppView says: a DID, a refusal, or an answer without a DID.
  const foreign = { handle: 'someone.example.com' }
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: resolve, body: { did: UNKNOWN } })
  assert.equal((await get(s, 'resolveHandle of a handle hosted elsewhere', resolve, foreign)).json.did, UNKNOWN)
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: resolve, status: 400, body: { error: 'InvalidRequest', message: 'Unable to resolve handle' } })
  await get(s, 'resolveHandle that the AppView refuses', resolve, foreign)
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: resolve, status: 500, body: { error: 'InternalServerError', message: 'Internal Server Error' } })
  await get(s, 'resolveHandle when the AppView fails', resolve, foreign)
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: resolve, body: {} })
  await get(s, 'resolveHandle when the AppView answers without a DID', resolve, foreign)
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: resolve, body: { did: 'not a did' } })
  await get(s, 'resolveHandle when the AppView answers with a value that is not a DID', resolve, foreign)
})
