// Account status: SPEC 10.1 (deactivation and reactivation), 6.3 and 6.4
// (what a deactivated repository refuses), 4.4 (the proxy needs an active
// account).

import assert from 'node:assert/strict'
import { checkRepo } from '../../src/oracles/repo.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { NOTE, surfaces } from '../../src/oracles/account-surfaces.ts'

scenario('12-lifecycle', {}, async (s) => {
  const alice = await s.createAccount('alice')
  const blob = (await s.http('uploadBlob', { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body: 'lifecycle blob', contentType: 'text/plain', auth: alice, silent: true })).json.blob
  await s.procedure('record with a blob', 'com.atproto.repo.createRecord', { repo: alice.did, collection: NOTE, rkey: 'one', record: { $type: NOTE, file: blob } }, { auth: alice, silent: true })

  const active = await surfaces(s, 'active', alice, blob.ref.$link)
  assert.equal(active.getRepo, 200)
  assert.equal(active.createRecord, 200)
  assert.equal(active.proxy, 200)
  assert.deepEqual(active.getRepoStatus, { active: true, status: null })

  // Deactivation.
  const deactivated = await s.procedure('deactivateAccount', 'com.atproto.server.deactivateAccount', { deleteAfter: '2030-01-01T00:00:00.000Z' }, { auth: alice })
  assert.equal(deactivated.status, 200)
  const session = await s.query('getSession while deactivated', 'com.atproto.server.getSession', {}, { auth: alice })
  assert.equal(session.json.active, false)
  assert.equal(session.json.status, 'deactivated')
  const status = await s.query('checkAccountStatus while deactivated', 'com.atproto.server.checkAccountStatus', {}, { auth: alice })
  assert.equal(status.json.activated, false)

  const inactive = await surfaces(s, 'deactivated', alice, blob.ref.$link)
  // SPEC 6.3: writes are refused. SPEC 6.4: blobs are not served to others.
  assert.equal(inactive.createRecord, '401 AccountDeactivated')
  assert.notEqual(inactive.getBlob, 200)
  assert.notEqual(inactive.getRepo, 200)
  // What the Reference still allows a deactivated account, because an account
  // that is migrating in is deactivated until it is activated: uploading
  // blobs, reading its own repository and blobs, preferences, minting service
  // tokens, and the proxy. The surface record above holds a Candidate to it.
  assert.equal(inactive.uploadBlob, 200)
  assert.equal(inactive.proxy, 200)
  assert.deepEqual(inactive.getRepoStatus, { active: false, status: 'deactivated' })
  refused(await s.query('getRepo of a deactivated account', 'com.atproto.sync.getRepo', { did: alice.did }), 'RepoDeactivated')
  refused(await s.procedure('createRecord while deactivated', 'com.atproto.repo.createRecord', { repo: alice.did, collection: NOTE, rkey: 'refused', record: { $type: NOTE } }, { auth: alice }))

  // Signing in still works, and says so.
  const signIn = await s.procedure('createSession while deactivated', 'com.atproto.server.createSession', { identifier: alice.handle, password: alice.password })
  assert.equal(signIn.status, 200)
  assert.equal(signIn.json.active, false)
  assert.equal(signIn.json.status, 'deactivated')
  const refreshed = await s.procedure('refreshSession while deactivated', 'com.atproto.server.refreshSession', undefined, { auth: signIn.json.refreshJwt })
  assert.equal(refreshed.status, 200)
  // The owner can still export the repository, which is what a migration needs.
  assert.equal(inactive.getRepoAsOwner, 200)

  // listRepos keeps the account and marks it.
  const repos = await s.query('listRepos', 'com.atproto.sync.listRepos', { limit: 1000 }, { silent: true })
  s.note('listRepos entry while deactivated', repos.json.repos.find((r: { did: string }) => r.did === alice.did))

  // Reactivation restores every surface.
  const activated = await s.procedure('activateAccount', 'com.atproto.server.activateAccount', undefined, { auth: alice })
  assert.equal(activated.status, 200)
  const restored = await surfaces(s, 'reactivated', alice, blob.ref.$link)
  assert.deepEqual(restored, active)
  await checkRepo(s, 'O2 after reactivation', alice)

  // Deactivating twice and activating twice are both accepted.
  await s.procedure('deactivateAccount again', 'com.atproto.server.deactivateAccount', {}, { auth: alice })
  await s.procedure('deactivateAccount a second time', 'com.atproto.server.deactivateAccount', {}, { auth: alice })
  await s.procedure('activateAccount again', 'com.atproto.server.activateAccount', undefined, { auth: alice })
  const twice = await s.procedure('activateAccount a second time', 'com.atproto.server.activateAccount', undefined, { auth: alice })
  assert.equal(twice.status, 200)
})
