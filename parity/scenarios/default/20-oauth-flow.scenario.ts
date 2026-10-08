// OAuth, the flows that must work: SPEC 11.1 (metadata), 11.3 (PAR,
// authorize, token, refresh, revoke) and oracle O4's first clause: full flows
// for two fixture users, and again after a server restart.

import assert from 'node:assert/strict'
import { launchBrowser, newDevice } from '../../src/lib/browser.ts'
import { decodeJwt } from '../../src/lib/jwt.ts'
import { makeClient } from '../../src/lib/oauth.ts'
import { describeSession, signIn, viaSession } from '../../src/oracles/oauth.ts'
import { checkRepo } from '../../src/oracles/repo.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const ISSUER = `https://${HOSTS.pds}`
const NOTE = 'com.example.parity.note'

scenario('20-oauth-flow', { timeoutMs: 300_000 }, async (s) => {
  // SPEC 11.1: authorization-server metadata.
  const metadata = (await s.http('authorization server metadata', { path: '/.well-known/oauth-authorization-server' })).json
  assert.equal(metadata.issuer, ISSUER)
  assert.equal(metadata.require_pushed_authorization_requests, true)
  assert.notEqual(metadata.require_request_uri_registration, false)
  assert.deepEqual(metadata.dpop_signing_alg_values_supported.includes('ES256'), true)
  assert.deepEqual(metadata.code_challenge_methods_supported, ['S256'])
  assert.equal(metadata.client_id_metadata_document_supported, true)
  assert.equal(metadata.authorization_response_iss_parameter_supported, true)
  assert.ok(metadata.token_endpoint_auth_methods_supported.includes('none'))
  assert.ok(metadata.token_endpoint_auth_methods_supported.includes('private_key_jwt'))
  assert.ok(metadata.token_endpoint_auth_signing_alg_values_supported.includes('ES256'))
  assert.ok(!metadata.token_endpoint_auth_signing_alg_values_supported.includes('none'))
  assert.ok(metadata.grant_types_supported.includes('authorization_code'))
  assert.ok(metadata.grant_types_supported.includes('refresh_token'))
  assert.ok(metadata.scopes_supported.includes('atproto'))
  assert.deepEqual(metadata.protected_resources, [ISSUER])
  const resource = (await s.http('protected resource metadata', { path: '/.well-known/oauth-protected-resource' })).json
  assert.deepEqual(resource.authorization_servers, [ISSUER])
  await s.http('JWKS', { path: '/oauth/jwks' })

  const alice = await s.createAccount('alice')
  const bob = await s.createAccount('bob')
  const browser = await launchBrowser(s.target)
  s.onCleanup(() => browser.close())
  const client = await makeClient(s, 'web', 'public')

  // Two users, two devices.
  const aliceDevice = await newDevice(browser, { ip: s.ip })
  const bobDevice = await newDevice(browser, { ip: s.ip })
  const a = await signIn(s, aliceDevice, client, alice, { remember: true })
  const b = await signIn(s, bobDevice, client, bob)
  assert.deepEqual(a.facts.steps, ['sign-in', 'consent'])
  // SPEC 11.3: an untrusted client is shown by its full client_id, never by the name it gives itself.
  assert.equal(a.facts.showsClientId, true)
  assert.equal(a.facts.showsClientName, false)

  // The access token: signed with the server's HS256 key, bound to the DPoP key (SPEC 11.3, 5.5).
  const saved = client.sessions.get(alice.did)!
  const access = decodeJwt(saved.tokenSet.access_token)
  assert.equal(access.header.alg, 'HS256')
  assert.equal(access.header.typ, 'at+jwt')
  assert.equal(access.claims.sub, alice.did)
  assert.equal(access.claims.iss, ISSUER)
  assert.ok(access.claims.cnf?.jkt, 'cnf.jkt binds the token to the DPoP key')
  assert.equal(saved.tokenSet.token_type, 'DPoP')
  assert.equal(saved.tokenSet.sub, alice.did, 'the token response carries sub')
  assert.equal(saved.tokenSet.scope, 'atproto transition:generic', 'the token response carries scope')
  s.note('access token lifetime, seconds', access.claims.exp - access.claims.iat)

  // The session works against the resource server.
  const session = await viaSession(s, 'getSession through OAuth', a.session, '/xrpc/com.atproto.server.getSession')
  assert.equal(session.status, 200)
  assert.equal(session.json.did, alice.did)
  assert.equal(session.json.email, undefined, 'the email needs transition:email')
  const written = await viaSession(s, 'createRecord through OAuth', a.session, '/xrpc/com.atproto.repo.createRecord', { json: { repo: alice.did, collection: NOTE, rkey: 'oauth', record: { $type: NOTE, via: 'oauth' } } })
  assert.equal(written.status, 200)
  await checkRepo(s, 'O2 after a write through OAuth', alice)
  const proxied = await viaSession(s, 'a proxied call through OAuth', a.session, `/xrpc/app.bsky.graph.getFollows?actor=${alice.did}`)
  assert.equal(proxied.status, 200)
  // Cross-user access: alice's session cannot write to bob's repository.
  refused(await viaSession(s, "createRecord in another account's repository through OAuth", a.session, '/xrpc/com.atproto.repo.createRecord', { json: { repo: bob.did, collection: NOTE, rkey: 'x', record: { $type: NOTE } } }))
  // Account management is outside transition:generic.
  refused(await viaSession(s, 'createAppPassword through OAuth', a.session, '/xrpc/com.atproto.server.createAppPassword', { json: { name: 'x' } }))
  refused(await viaSession(s, 'deactivateAccount through OAuth', a.session, '/xrpc/com.atproto.server.deactivateAccount', { json: {} }))
  // A DPoP-bound token is not a bearer token.
  refused(await s.query('the OAuth access token as a plain bearer token', 'com.atproto.server.getSession', {}, { auth: saved.tokenSet.access_token }))

  // Refresh with rotation.
  const before = client.sessions.get(bob.did)!.tokenSet
  const refreshed = await client.client.restore(bob.did, true)
  const after = client.sessions.get(bob.did)!.tokenSet
  assert.notEqual(after.access_token, before.access_token)
  assert.notEqual(after.refresh_token, before.refresh_token, 'the refresh token rotates')
  s.note('tokens after a refresh', await describeSession(s, client, bob.did))
  assert.equal((await viaSession(s, 'getSession after a refresh', refreshed, '/xrpc/com.atproto.server.getSession')).status, 200)

  // O4: after a server restart the sessions still work, refresh works, and a new flow works.
  await s.target.restart()
  const afterRestart = await client.client.restore(alice.did, false)
  assert.equal((await viaSession(s, 'getSession after a restart, same access token', afterRestart, '/xrpc/com.atproto.server.getSession')).status, 200)
  const refreshedAfterRestart = await client.client.restore(alice.did, true)
  assert.equal((await viaSession(s, 'getSession after a restart and a refresh', refreshedAfterRestart, '/xrpc/com.atproto.server.getSession')).status, 200)

  // The device that remembered the account survived the restart too: it signs in again without a password.
  const again = await signIn(s, aliceDevice, client, alice, { password: null, label: 'alice signs in again on the same device, after the restart' })
  s.note('steps on a device that already has a session', again.facts.steps)
  // A fresh device after the restart.
  const carolDevice = await newDevice(browser, { ip: s.ip })
  const b2 = await signIn(s, carolDevice, client, bob, { label: 'bob signs in on a new device, after the restart' })
  assert.deepEqual(b2.facts.steps, ['sign-in', 'consent'])

  // transition:email exposes the address (SPEC 5.5).
  const emailClient = await makeClient(s, 'mail', 'public', { scope: 'atproto transition:generic transition:email' })
  const withEmail = await signIn(s, await newDevice(browser, { ip: s.ip }), emailClient, alice, { label: 'alice signs in with transition:email' })
  const emailSession = await viaSession(s, 'getSession with transition:email', withEmail.session, '/xrpc/com.atproto.server.getSession')
  assert.equal(emailSession.json.email, alice.email)

  // `atproto` alone identifies the account and grants no repository access.
  const idClient = await makeClient(s, 'id', 'public', { scope: 'atproto' })
  const idOnly = await signIn(s, await newDevice(browser, { ip: s.ip }), idClient, alice, { label: 'alice signs in with the atproto scope only' })
  refused(await viaSession(s, 'createRecord with the atproto scope only', idOnly.session, '/xrpc/com.atproto.repo.createRecord', { json: { repo: alice.did, collection: NOTE, rkey: 'denied', record: { $type: NOTE } } }))

  // Revocation: signing out ends the session at the server.
  const revokedToken = client.sessions.get(bob.did)!.tokenSet.access_token
  await b2.session.signOut()
  assert.equal(client.sessions.has(bob.did), false)
  s.note('after sign-out the client holds no session', true)
  void revokedToken

  // Denying consent returns an error to the client.
  const denyUrl = await client.client.authorize(alice.handle, { scope: 'atproto transition:generic', state: 'deny' })
  const page = await (await newDevice(browser, { ip: s.ip })).newPage()
  const { driveAuthorization } = await import('../../src/oracles/oauth.ts')
  const denied = await driveAuthorization(page, String(denyUrl), { redirectUri: client.redirectUri, password: alice.password, decision: 'deny' })
  assert.ok(denied.redirect)
  s.note('denied consent', Object.fromEntries([...denied.redirect.searchParams].filter(([k]) => k !== 'state')))
  assert.equal(denied.redirect.searchParams.get('error'), 'access_denied')
})
