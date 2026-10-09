// The sign-up journey with email (patches 102 and 099b, SPEC 12.2 and 12.8):
// the page the LinkJar app hands new members to. Email first, the providers
// under it, then the username.

import assert from 'node:assert/strict'
import { launchBrowser, newDevice } from '../../src/lib/browser.ts'
import { mailsTo } from '../../src/lib/mail.ts'
import { makeAppClient, makeClient } from '../../src/lib/oauth.ts'
import { driveAuthorization, viaSession } from '../../src/oracles/oauth.ts'
import { checkRepo } from '../../src/oracles/repo.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

scenario('32-linkjar-signup-journey', { linkjar: true, timeoutMs: 300_000 }, async (s) => {
  const browser = await launchBrowser(s.target)
  s.onCleanup(() => browser.close())
  const app = await makeAppClient(s)

  const signUp = async (name: string, client = app, handle = `${name}-${s.tag}`) => {
    const url = await client.client.authorize(`https://${HOSTS.pds}`, { scope: String(client.metadata.scope), state: `signup-${name}`, prompt: 'create' })
    const page = await (await newDevice(browser, { ip: s.ip })).newPage()
    const outcome = await driveAuthorization(page, String(url), {
      redirectUri: client.redirectUri,
      signUp: { email: s.email(name), password: `${name}-password-1`, handle },
      clientId: client.clientId,
      clientName: String(client.metadata.client_name),
      allowError: true,
      timeoutMs: 20_000,
    })
    const pageText = outcome.redirect ? '' : await page.locator('body').innerText().catch(() => '')
    await page.close()
    return { ...outcome, pageText }
  }

  // Sign-up through the trusted client.
  const first = await signUp('member')
  assert.ok(first.redirect, `sign-up did not finish: ${first.facts.error ?? first.pageText.slice(0, 200)}`)
  s.note('email sign-up, trusted client', first.facts)
  assert.deepEqual(first.facts.steps.slice(0, 2), ['sign-up:credentials', 'sign-up:handle'])
  assert.equal(first.facts.suggestedHandle, '', 'an email sign-up chooses its own username')
  const { session } = await app.client.callback(first.redirect.searchParams)
  const me = await viaSession(s, 'getSession of the new member', session, '/xrpc/com.atproto.server.getSession')
  assert.equal(me.json.handle, s.handle('member'))
  await checkRepo(s, 'O2 for an account created on the sign-up page', { did: session.did })

  // The session that created the account holds a receipt (patch 099b).
  const receipt = await viaSession(s, 'getSignupReceipt after an email sign-up', session, '/xrpc/io.linkjar.account.getSignupReceipt')
  assert.deepEqual(receipt.json, { created: true, did: session.did, handle: s.handle('member') })

  // The password works on the legacy endpoint, and a legacy session has no receipt.
  const legacy = await s.procedure('createSession with the password chosen on the page', 'com.atproto.server.createSession', { identifier: s.handle('member'), password: 'member-password-1' })
  assert.equal(legacy.status, 200)
  assert.equal(legacy.json.emailConfirmed, false)
  refused(await s.query('getSignupReceipt with a legacy session', 'io.linkjar.account.getSignupReceipt', {}, { auth: legacy.json.accessJwt }))
  s.note('mail sent at sign-up', (await mailsTo(s, s.email('member'))).map((mail) => mail.subject))

  // What the page refuses.
  const taken = await signUp('second', app, `member-${s.tag}`)
  s.note('sign-up with a username that is taken', { finished: Boolean(taken.redirect), errorShown: Boolean(taken.facts.error) || /taken|available|already/i.test(taken.pageText) })
  assert.equal(taken.redirect, undefined)
  const reserved = await signUp('third', app, 'support')
  s.note('sign-up with a reserved username', { finished: Boolean(reserved.redirect) })
  assert.equal(reserved.redirect, undefined)

  // Through a client that is not trusted, the same journey ends on the consent step.
  const other = await makeClient(s, 'other', 'public')
  const viaOther = await signUp('guest', other)
  assert.ok(viaOther.redirect)
  s.note('email sign-up, untrusted client', viaOther.facts)
  assert.ok(viaOther.facts.steps.includes('consent'))
})
