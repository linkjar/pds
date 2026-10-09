// The invitation hand-off (patch 103, SPEC 13): the LinkJar app sends a new
// member to the sign-up page with the invite code in the URL fragment,
// `#invite=<code>`. The page submits the code in place of the invite field,
// and the provider links carry it to the start request.

import assert from 'node:assert/strict'
import { launchBrowser, newDevice } from '../../src/lib/browser.ts'
import { makeAppClient } from '../../src/lib/oauth.ts'
import { driveAuthorization } from '../../src/oracles/oauth.ts'
import { scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

scenario('02-linkjar-invite-handoff', { linkjar: true, timeoutMs: 300_000 }, async (s) => {
  const browser = await launchBrowser(s.target)
  s.onCleanup(() => browser.close())
  const app = await makeAppClient(s)
  const invite = async () => (await s.procedure('createInviteCode', 'com.atproto.server.createInviteCode', { useCount: 1 }, { auth: 'admin', silent: true })).json.code as string
  const uses = async (code: string) => {
    const listed = await s.query('getInviteCodes', 'com.atproto.admin.getInviteCodes', { limit: 500 }, { auth: 'admin', silent: true })
    return listed.json.codes.find((c: { code: string }) => c.code === code)?.uses.length as number
  }

  const journey = async (name: string, opts: { fragment?: string; provider?: 'Google'; typedCode?: string }) => {
    const url = await app.client.authorize(`https://${HOSTS.pds}`, { scope: String(app.metadata.scope), state: `invite-${name}`, prompt: 'create' })
    const page = await (await newDevice(browser, { ip: s.ip })).newPage()
    const inviteFieldShownAtStart = await (async () => {
      await page.goto(String(url) + (opts.fragment ?? ''), { waitUntil: 'domcontentloaded' })
      await page.locator('input[name=email]').first().waitFor({ state: 'visible', timeout: 10_000 })
      return (await page.locator('input[name=inviteCode]').count()) > 0 && (await page.locator('input[name=inviteCode]').first().isVisible())
    })()
    const outcome = await driveAuthorization(page, page.url(), {
      redirectUri: app.redirectUri,
      provider: opts.provider,
      signUp: opts.provider ? undefined : { email: s.email(name), password: `${name}-password-1`, handle: `${name}-${s.tag}`, inviteCode: opts.typedCode },
      allowError: true,
      timeoutMs: 20_000,
    })
    const inviteFieldShownAtEnd = outcome.redirect ? null : (await page.locator('input[name=inviteCode]').count()) > 0 && (await page.locator('input[name=inviteCode]').first().isVisible())
    const text = outcome.redirect ? '' : await page.locator('body').innerText().catch(() => '')
    await page.close()
    return { ...outcome, inviteFieldShownAtStart, inviteFieldShownAtEnd, text }
  }

  // Without a hand-off the page asks for the code.
  const typed = await invite()
  const manual = await journey('typed', { typedCode: typed })
  s.note('sign-up with a typed invite code', { finished: Boolean(manual.redirect), inviteFieldShown: manual.inviteFieldShownAtStart, steps: manual.facts.steps })
  assert.equal(manual.inviteFieldShownAtStart, true)
  assert.ok(manual.redirect, manual.text.slice(0, 200))
  assert.equal(await uses(typed), 1)

  // With the hand-off the field is gone and the code is used.
  const handed = await invite()
  const handoff = await journey('handed', { fragment: `#invite=${handed}` })
  s.note('sign-up with the code in the fragment', { finished: Boolean(handoff.redirect), inviteFieldShown: handoff.inviteFieldShownAtStart, steps: handoff.facts.steps })
  assert.equal(handoff.inviteFieldShownAtStart, false, 'the page does not ask for a code it was handed')
  assert.ok(handoff.redirect, handoff.text.slice(0, 200))
  assert.equal(await uses(handed), 1)

  // A code the server refuses: the field comes back with the error.
  const refusedCode = await journey('refused', { fragment: '#invite=pds-linkjar-social-aaaaa-bbbbb' })
  s.note('sign-up with a refused code in the fragment', { finished: Boolean(refusedCode.redirect), inviteFieldShownAfterwards: refusedCode.inviteFieldShownAtEnd })
  assert.equal(refusedCode.redirect, undefined)
  assert.equal(refusedCode.inviteFieldShownAtEnd, true)

  // A provider sign-up carries the code too, so an invited person can use Apple or Google.
  const viaProvider = await invite()
  await s.fixtures.identity({ provider: 'google', subject: `google-${s.tag}-i1`, email: s.email('invited'), emailVerified: true, name: `Invited ${s.tag}` })
  const mark = await s.fixtures.cursor()
  const provider = await journey('provider', { fragment: `#invite=${viaProvider}`, provider: 'Google' })
  assert.ok(provider.redirect, provider.text.slice(0, 200))
  assert.equal(await uses(viaProvider), 1)
  s.note('provider sign-up with the code in the fragment', { finished: true, steps: provider.facts.steps, providerRequests: (await s.fixtures.requests('accounts.google.com', mark)).length })

  // A provider sign-up without a code is refused while invites are required.
  await s.fixtures.identity({ provider: 'google', subject: `google-${s.tag}-i2`, email: s.email('uninvited'), emailVerified: true, name: `Uninvited ${s.tag}` })
  const uninvited = await journey('uninvited', { provider: 'Google' })
  s.note('provider sign-up without a code', { finished: Boolean(uninvited.redirect) })
  assert.equal(uninvited.redirect, undefined)
})
