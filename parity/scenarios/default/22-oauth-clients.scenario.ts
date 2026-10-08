// OAuth client kinds and account states: SPEC 11.2 and the rest of oracle
// O4: a confidential client and what happens when its key is removed, a
// loopback client, how a trusted client is shown against an untrusted one,
// and a deactivated account.

import assert from 'node:assert/strict'
import { JoseKey } from '@atproto/oauth-client-node'
import { launchBrowser, newDevice } from '../../src/lib/browser.ts'
import { decodeJwt } from '../../src/lib/jwt.ts'
import { makeClient, makeLoopbackClient } from '../../src/lib/oauth.ts'
import { signIn, viaSession } from '../../src/oracles/oauth.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const NOTE = 'com.example.parity.note'

scenario('22-oauth-clients', { timeoutMs: 300_000 }, async (s) => {
  const alice = await s.createAccount('alice')
  const bob = await s.createAccount('bob')
  const browser = await launchBrowser(s.target)
  s.onCleanup(() => browser.close())
  const device = () => newDevice(browser, { ip: s.ip })

  // A confidential client: private_key_jwt, with the session bound to the assertion key (SPEC 11.3).
  const confidential = await makeClient(s, 'conf', 'confidential')
  const c = await signIn(s, await device(), confidential, alice, { label: 'alice signs in through a confidential client' })
  assert.equal((await viaSession(s, 'getSession through the confidential client', c.session, '/xrpc/com.atproto.server.getSession')).status, 200)
  const refreshed = await confidential.client.restore(alice.did, true)
  assert.equal((await viaSession(s, 'getSession after a confidential refresh', refreshed, '/xrpc/com.atproto.server.getSession')).status, 200)

  // O4: the client removes its key from its metadata. The session was bound
  // to that key, so the next refresh must fail once the server sees the new document.
  const replacement = await JoseKey.generate(['ES256'], 'conf-key-2')
  await s.fixtures.serve({ host: HOSTS.client, path: new URL(confidential.clientId).pathname, body: JSON.stringify({ ...confidential.metadata, jwks: { keys: [replacement.publicJwk] } }) })
  // The Reference keeps client metadata in a memory cache, so the session
  // survives until the cache lets go. A restart empties the cache on any target.
  const outcome: Record<string, unknown> = {}
  const tryRefresh = () => confidential.client.restore(alice.did, true).then(() => 'refreshed', (error: Error) => `${error.name}: ${error.message}`)
  outcome.whileCached = await tryRefresh()
  await s.target.restart()
  outcome.afterRestart = await tryRefresh()
  s.note('refresh after the confidential client removed its key', outcome)
  assert.notEqual(outcome.afterRestart, 'refreshed', 'a session bound to a removed key does not refresh')

  // A loopback client (SPEC 11.2): synthesized metadata, a public native client.
  const loopback = makeLoopbackClient(s)
  const l = await signIn(s, await device(), loopback, bob, { label: 'bob signs in through a loopback client' })
  assert.equal(l.redirect.origin, 'http://127.0.0.1')
  assert.equal((await viaSession(s, 'getSession through the loopback client', l.session, '/xrpc/com.atproto.server.getSession')).status, 200)
  const loopbackToken = decodeJwt(loopback.sessions.get(bob.did)!.tokenSet.access_token)
  assert.equal(loopbackToken.claims.client_id, loopback.clientId)

  // A client on the trusted list (PDS_OAUTH_TRUSTED_CLIENTS) against one that is not.
  const trusted = await makeClient(s, 'app', 'public', { host: HOSTS.app, path: '/client-metadata.json', metadata: { client_name: 'LinkJar' } })
  const t = await signIn(s, await device(), trusted, alice, { label: 'alice signs in through a trusted client' })
  const untrusted = await makeClient(s, 'other', 'public')
  const u = await signIn(s, await device(), untrusted, alice, { label: 'alice signs in through an untrusted client' })
  s.note('how each client is shown', {
    trusted: { steps: t.facts.steps, showsClientName: t.facts.showsClientName ?? null },
    untrusted: { steps: u.facts.steps, showsClientId: u.facts.showsClientId, showsClientName: u.facts.showsClientName },
  })
  assert.equal(u.facts.showsClientId, true)
  assert.equal(u.facts.showsClientName, false)

  // A deactivated account (O4). Its OAuth session cannot write, and it can still be read.
  const victim = await signIn(s, await device(), untrusted, bob, { label: 'bob signs in before deactivating' })
  await s.procedure('deactivateAccount', 'com.atproto.server.deactivateAccount', {}, { auth: bob })
  refused(await viaSession(s, 'createRecord through OAuth while deactivated', victim.session, '/xrpc/com.atproto.repo.createRecord', { json: { repo: bob.did, collection: NOTE, rkey: 'x', record: { $type: NOTE } } }), 'AccountDeactivated')
  const stillReads = await viaSession(s, 'getSession through OAuth while deactivated', victim.session, '/xrpc/com.atproto.server.getSession')
  s.note('getSession through OAuth while deactivated', { status: stillReads.status, active: stillReads.json?.active, accountStatus: stillReads.json?.status })
  const refreshWhileOff = await untrusted.client.restore(bob.did, true).then(() => 'refreshed', (error: Error) => error.name)
  s.note('refresh while deactivated', refreshWhileOff)
  await s.procedure('activateAccount', 'com.atproto.server.activateAccount', undefined, { auth: bob })

  // A taken-down account: sessions end.
  await s.procedure('take bob down', 'com.atproto.admin.updateSubjectStatus', { subject: { $type: 'com.atproto.admin.defs#repoRef', did: bob.did }, takedown: { applied: true } }, { auth: 'admin' })
  const afterTakedown = await viaSession(s, 'getSession through OAuth after a takedown', l.session, '/xrpc/com.atproto.server.getSession')
  refused(afterTakedown)
  const refreshAfterTakedown = await loopback.client.restore(bob.did, true).then(() => 'refreshed', (error: Error) => error.name)
  s.note('refresh after a takedown', refreshAfterTakedown)
})
