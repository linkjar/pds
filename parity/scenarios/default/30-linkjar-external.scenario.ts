// External sign-in (patch 099, SPEC 12.3), the handle a new account is
// given (patch 093, SPEC 12.4) and the creation receipt (patch 099b, SPEC
// 12.8), end to end through the authorization page.
//
// Port of legacy/tests/external-accounts.mjs, external-ui.mjs and
// signup-receipt.mjs. Apple, Google and GitHub are the fixture server's
// stand-ins: the patch fixes the provider endpoints, and the stack resolves
// those names to the fixtures.

import assert from 'node:assert/strict'
import { launchBrowser, newDevice } from '../../src/lib/browser.ts'
import { makeAppClient, makeClient } from '../../src/lib/oauth.ts'
import { externalSignIn } from '../../src/oracles/linkjar.ts'
import { viaSession } from '../../src/oracles/oauth.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const RECEIPT = '/xrpc/io.linkjar.account.getSignupReceipt'
const ACKNOWLEDGE = '/xrpc/io.linkjar.account.acknowledgeSignupReceipt'

scenario('30-linkjar-external', { linkjar: true, timeoutMs: 420_000 }, async (s) => {
  const browser = await launchBrowser(s.target)
  s.onCleanup(() => browser.close())
  const device = () => newDevice(browser, { ip: s.ip })
  // The LinkJar web app is on the trusted list; the other client is not.
  const app = await makeAppClient(s)
  const other = await makeClient(s, 'other', 'public')
  const name = `Nova ${s.tag}`
  const derived = `nova-${s.tag}${HOSTS.handleDomain}`

  // A new Google identity through the trusted client: the account is created
  // and the client is authorized without a consent step.
  const google = { provider: 'google', subject: `google-${s.tag}-1`, email: s.email('nova'), emailVerified: true, name } as const
  const created = await externalSignIn(s, await device(), app, google)
  assert.ok(created.session && created.did)
  s.note('new Google identity, trusted client', created.facts)
  assert.deepEqual(created.facts.steps, ['provider:google'], 'fresh sign-up is pre-authorized for a trusted client')
  const session = await viaSession(s, 'getSession of the new account', created.session, '/xrpc/com.atproto.server.getSession')
  assert.equal(session.status, 200)
  // SPEC 13, patch 093: the handle comes from the display name: ASCII, lower case, 3 to 18 characters.
  assert.equal(session.json.handle, derived)
  const document = await s.http('PLC document of the new account', { host: HOSTS.plc, path: `/${created.did}` })
  assert.deepEqual(document.json.alsoKnownAs, [`at://${derived}`])
  const info = await s.query('getAccountInfo of the new account', 'com.atproto.admin.getAccountInfo', { did: created.did }, { auth: 'admin' })
  assert.equal(info.json.email, google.email)
  assert.ok(info.json.emailConfirmedAt, 'an address the provider verified is confirmed')

  // A passwordless account has no password to sign in with.
  refused(await s.procedure('createSession for a passwordless account', 'com.atproto.server.createSession', { identifier: derived, password: '!external-identity' }), 'AuthenticationRequired')
  refused(await s.procedure('createSession for a passwordless account, empty password', 'com.atproto.server.createSession', { identifier: derived, password: '' }))

  // The creation receipt (patch 099b): bound to the session that created the account.
  const receipt = await viaSession(s, 'getSignupReceipt', created.session, RECEIPT)
  assert.equal(receipt.status, 200)
  assert.deepEqual(receipt.json, { created: true, did: created.did, handle: derived })
  assert.equal(receipt.headers.get('cache-control'), 'no-store')
  const again = await viaSession(s, 'getSignupReceipt, read twice', created.session, RECEIPT)
  assert.equal(again.json.created, true, 'reading does not consume the receipt')
  // It survives a token rotation.
  const rotated = await app.client.restore(created.did, true)
  assert.equal((await viaSession(s, 'getSignupReceipt after a refresh', rotated, RECEIPT)).json.created, true)
  refused(await s.http('getSignupReceipt without a credential', { path: RECEIPT }))

  // The same identity again, through the other client: the same account, a
  // consent step, and no receipt, because this session created nothing.
  const returning = await externalSignIn(s, await device(), other, google, { prompt: 'login' })
  assert.equal(returning.did, created.did)
  s.note('returning Google identity, untrusted client', returning.facts)
  assert.ok(returning.facts.steps.includes('consent'))
  const noReceipt = await viaSession(s, 'getSignupReceipt of a returning sign-in', returning.session!, RECEIPT)
  assert.deepEqual(noReceipt.json, { created: false, did: created.did })
  // Another session cannot acknowledge it.
  const foreignAck = await viaSession(s, 'acknowledgeSignupReceipt from another session', returning.session!, ACKNOWLEDGE, { json: {} })
  assert.deepEqual(foreignAck.json, { created: false, did: created.did })
  assert.equal((await viaSession(s, 'getSignupReceipt after a foreign acknowledgment', rotated, RECEIPT)).json.created, true)

  // Acknowledgment clears it for good.
  const acknowledged = await viaSession(s, 'acknowledgeSignupReceipt', rotated, ACKNOWLEDGE, { json: {} })
  assert.deepEqual(acknowledged.json, { created: false, did: created.did })
  assert.equal(acknowledged.headers.get('cache-control'), 'no-store')
  assert.deepEqual((await viaSession(s, 'acknowledgeSignupReceipt again', rotated, ACKNOWLEDGE, { json: {} })).json, { created: false, did: created.did })
  const afterAck = await app.client.restore(created.did, true)
  assert.equal((await viaSession(s, 'getSignupReceipt after acknowledgment and a refresh', afterAck, RECEIPT)).json.created, false)

  // A second person with the same display name: the handle gets a six-hex suffix.
  const twin = await externalSignIn(s, await device(), app, { provider: 'google', subject: `google-${s.tag}-2`, email: s.email('twin'), emailVerified: true, name })
  // The suffix is random, so the exchange stays out of the transcript and the shape of the handle goes in.
  const twinSession = await twin.session!.fetchHandler('/xrpc/com.atproto.server.getSession')
  const twinHandle = ((await twinSession.json()) as { handle: string }).handle
  assert.match(twinHandle, new RegExp(`^nova-${s.tag}-[0-9a-f]{6}\\.linkjar\\.social$`))
  s.note('handle on a collision', twinHandle.replace(/-[0-9a-f]{6}\./, '-<six hex>.'))

  // No display name: the handle comes from the local part of the email.
  const nameless = await externalSignIn(s, await device(), app, { provider: 'google', subject: `google-${s.tag}-3`, email: `quiet.${s.tag}@example.com`, emailVerified: true })
  const namelessHandle = (await viaSession(s, 'getSession of an account without a display name', nameless.session!, '/xrpc/com.atproto.server.getSession')).json.handle
  s.note('handle from the email local part', namelessHandle)
  assert.equal(namelessHandle, `quiet-${s.tag}${HOSTS.handleDomain}`)

  // GitHub: the numeric id is the subject and only a verified primary email counts.
  const github = await externalSignIn(s, await device(), app, { provider: 'github', subject: '4242001', email: s.email('octo'), emailVerified: true, name: `Octo ${s.tag}` })
  assert.ok(github.did)
  assert.equal((await viaSession(s, 'getSession of a GitHub account', github.session!, '/xrpc/com.atproto.server.getSession')).json.handle, `octo-${s.tag}${HOSTS.handleDomain}`)
  const unverified = await externalSignIn(s, await device(), app, { provider: 'github', subject: '4242002', email: s.email('octo2'), emailVerified: false, name: 'Unverified' }, { expectFailure: true })
  s.note('GitHub identity whose primary email is not verified', { finished: Boolean(unverified.session), errorShown: Boolean(unverified.redirect?.searchParams.get('error') ?? unverified.facts.error) })
  assert.equal(unverified.session, undefined)

  // Apple: a form post back to the server, the name in the first response only.
  const apple = await externalSignIn(s, await device(), app, { provider: 'apple', subject: `apple-${s.tag}-1`, email: s.email('pomme'), emailVerified: 'true', name: `Pomme ${s.tag}` })
  assert.ok(apple.did)
  assert.equal((await viaSession(s, 'getSession of an Apple account', apple.session!, '/xrpc/com.atproto.server.getSession')).json.handle, `pomme-${s.tag}${HOSTS.handleDomain}`)

  // An email that an existing account holds is refused, never linked.
  const local = await s.createAccount('local')
  const collision = await externalSignIn(s, await device(), app, { provider: 'google', subject: `google-${s.tag}-4`, email: local.email, emailVerified: true, name: 'Collision' }, { expectFailure: true })
  s.note('provider identity whose email belongs to an existing account', { finished: Boolean(collision.session), errorShown: Boolean(collision.redirect?.searchParams.get('error') ?? collision.facts.error) })
  assert.equal(collision.session, undefined)
  const untouched = await s.query('the existing account is untouched', 'com.atproto.admin.getAccountInfo', { did: local.did }, { auth: 'admin' })
  assert.equal(untouched.json.email, local.email)

  // What the provider or the token can get wrong.
  for (const [label, identity] of [
    ['the person refuses at the provider', { provider: 'google', subject: `google-${s.tag}-5`, email: s.email('no'), deny: true }],
    ['an ID token for another audience', { provider: 'google', subject: `google-${s.tag}-6`, email: s.email('aud'), claims: { aud: 'another-client' } }],
    ['an ID token from another issuer', { provider: 'google', subject: `google-${s.tag}-7`, email: s.email('iss'), claims: { iss: 'https://attacker.example.com' } }],
    ['an ID token with another nonce', { provider: 'google', subject: `google-${s.tag}-8`, email: s.email('nonce'), claims: { nonce: 'another-request' } }],
    ['an ID token whose email is not verified', { provider: 'google', subject: `google-${s.tag}-9`, email: s.email('unv'), emailVerified: false }],
    ['an expired ID token', { provider: 'google', subject: `google-${s.tag}-10`, email: s.email('exp'), claims: { iat: 1, exp: 2 } }],
  ] as const) {
    const outcome = await externalSignIn(s, await device(), app, identity as never, { expectFailure: true })
    // The wording of the page is the target's own; that it stopped on an error is what counts.
    s.note(label, { finished: Boolean(outcome.session), errorShown: Boolean(outcome.redirect?.searchParams.get('error') ?? outcome.facts.error) })
    assert.equal(outcome.session, undefined, label)
  }
})
