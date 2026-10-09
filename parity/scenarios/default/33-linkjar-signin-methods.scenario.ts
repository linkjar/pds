// Sign-in methods on the account page (patch 100, SPEC 12.5 and 12.6):
// linking and unlinking a provider, the protection of the last method, and
// the security notice each change sends (patch 101). Port of
// legacy/tests/signin-methods.mjs and signin-methods-ui.mjs.

import assert from 'node:assert/strict'
import { launchBrowser, newDevice } from '../../src/lib/browser.ts'
import { describeMail, mailsTo, nextMail } from '../../src/lib/mail.ts'
import { makeAppClient } from '../../src/lib/oauth.ts'
import { AccountPage, externalSignIn } from '../../src/oracles/linkjar.ts'
import { viaSession } from '../../src/oracles/oauth.ts'
import { scenario } from '../../src/scenario.ts'

scenario('33-linkjar-signin-methods', { linkjar: true, timeoutMs: 420_000 }, async (s) => {
  const browser = await launchBrowser(s.target)
  s.onCleanup(() => browser.close())
  const app = await makeAppClient(s)
  const device = () => newDevice(browser, { ip: s.ip })

  // A password account links Google.
  const alice = await s.createAccount('alice')
  const page = await AccountPage.open(await device(), { identifier: alice.handle, password: alice.password })
  assert.deepEqual(await page.methods(), { linked: [], offered: ['apple', 'github', 'google'] })
  const identity = { provider: 'google', subject: `google-${s.tag}-m1`, email: s.email('alice-google'), emailVerified: true, name: 'Alice' } as const
  await s.fixtures.identity(identity)
  await page.link('Google')
  assert.deepEqual((await page.methods()).linked, ['google'])

  // The linked identity signs in to the same account.
  const viaGoogle = await externalSignIn(s, await device(), app, identity, { prompt: 'login' })
  assert.equal(viaGoogle.did, alice.did)
  assert.equal((await viaSession(s, 'getSession through the linked identity', viaGoogle.session!, '/xrpc/com.atproto.server.getSession')).json.handle, alice.handle)

  // Linking the same identity again changes nothing and sends nothing.
  await s.fixtures.identity(identity)
  if ((await page.methods()).offered.includes('google')) await page.link('Google')
  assert.deepEqual((await page.methods()).linked, ['google'])

  // An identity that belongs to another account is refused.
  const bob = await s.createAccount('bob')
  const bobPage = await AccountPage.open(await device(), { identifier: bob.handle, password: bob.password })
  await s.fixtures.identity(identity)
  await bobPage.link('Google')
  const bobMethods = await bobPage.methods()
  const bobText = await bobPage.text()
  s.note('linking an identity that another account holds', { linked: bobMethods.linked, namesOtherAccount: bobText.includes(alice.handle), showsOtherEmail: bobText.includes(alice.email) })
  assert.deepEqual(bobMethods.linked, [], 'the identity is not linked to a second account')
  assert.equal(bobText.includes(alice.email), false, "the other account's email is not disclosed")
  await bobPage.close()

  // A second provider, then unlinking one: allowed, because a password and another provider remain.
  await s.fixtures.identity({ provider: 'github', subject: '5150001', email: s.email('alice-github'), emailVerified: true, name: 'Alice' })
  await page.link('GitHub')
  assert.deepEqual((await page.methods()).linked, ['github', 'google'])
  await page.unlink('Google')
  assert.deepEqual((await page.methods()).linked, ['github'])
  await page.close()

  // The last method cannot be removed. An account created through a provider
  // has no password, so its only identity must stay.
  const solo = { provider: 'google', subject: `google-${s.tag}-m2`, email: s.email('solo'), emailVerified: true, name: `Solo ${s.tag}` } as const
  const created = await externalSignIn(s, await device(), app, solo)
  assert.ok(created.did)
  await s.fixtures.identity(solo)
  const soloPage = await AccountPage.open(await device(), { provider: 'Google' })
  assert.deepEqual((await soloPage.methods()).linked, ['google'])
  s.note('the only sign-in method can be unlinked', await soloPage.canUnlink('Google'))
  assert.equal(await soloPage.canUnlink('Google'), false, 'the page does not offer to remove the last method')
  // With a second method linked, the first can go.
  await s.fixtures.identity({ provider: 'apple', subject: `apple-${s.tag}-m2`, email: s.email('solo-apple'), emailVerified: 'true' })
  await soloPage.link('Apple')
  assert.deepEqual((await soloPage.methods()).linked, ['apple', 'google'])
  assert.equal(await soloPage.canUnlink('Google'), true)
  await soloPage.unlink('Google')
  assert.deepEqual((await soloPage.methods()).linked, ['apple'])
  await soloPage.close()

  // SPEC 12.6: every change sent a notice, through an outbox that the server
  // drains once a minute. Three changes for alice: Google linked, GitHub
  // linked, Google unlinked. The refused link and the idempotent link sent nothing.
  await nextMail(s, alice.email, 3, 150_000)
  const notices = (await mailsTo(s, alice.email)).reverse()
  s.note('sign-in notices to the password account', notices.map(describeMail))
  assert.equal(notices.length, 3)
  for (const notice of notices) {
    assert.match(notice.subject, /sign-in method changed/)
    assert.equal(notice.alternative, true, 'HTML with a text alternative')
    assert.ok(notice.headers['Message-Id'] ?? notice.headers['Message-ID'], 'a Message-ID')
  }
  assert.equal((await mailsTo(s, bob.email)).length, 0, 'a refused link sends nothing')
  await nextMail(s, solo.email, 2, 150_000)
  s.note('sign-in notices to the provider-created account', (await mailsTo(s, solo.email)).reverse().map(describeMail))
})
