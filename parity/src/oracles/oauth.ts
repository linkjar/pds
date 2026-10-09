// Oracle O4 (SPEC 17.3): OAuth flows driven by the official client and a
// real browser. The page driver states the contract the authorization pages
// must keep for any target: a password field and a submit button on the
// sign-in step, a button named "Authorize" and one named "Deny access" on
// the consent step. HTML is otherwise outside the comparison (C6); what a
// page shows is recorded as facts, not as markup.

import assert from 'node:assert/strict'
import type { OAuthSession } from '@atproto/oauth-client-node'
import type { BrowserContext, Page } from '../lib/browser.ts'
import type { TestClient } from '../lib/oauth.ts'
import type { Account, Result, Scenario } from '../scenario.ts'

export type PageFacts = {
  /** The steps the page went through, in order. */
  steps: string[]
  /** Whether the consent step named the client by its full client_id, and whether it showed the client's own name. */
  showsClientId?: boolean
  showsClientName?: boolean
  /** Error text the page showed, if it stopped on one. */
  error?: string
  /** The handle the sign-up step proposed (SPEC 12.4). */
  suggestedHandle?: string
}

export type DriveOptions = {
  redirectUri: string
  password?: string
  /** Typed into the identifier field when the page has not filled it from a login hint. */
  identifier?: string
  decision?: 'authorize' | 'deny'
  clientId?: string
  clientName?: string
  /** Continue with an external identity provider instead of a password (SPEC 12.3). */
  provider?: 'Apple' | 'Google' | 'GitHub'
  /** Create an account with email and password on the sign-up page. */
  signUp?: { email: string; password: string; handle: string; inviteCode?: string }
  /** Tick "remember this account" at sign-in, so that the device keeps the account. */
  remember?: boolean
  /** The handle to pick when the device offers accounts it remembers. */
  account?: string
  /** Stop and report instead of failing when the page shows an error. */
  allowError?: boolean
  timeoutMs?: number
}

/** Drives the authorization page to the redirect back to the client and returns the redirect URL. */
export async function driveAuthorization(page: Page, url: string, opts: DriveOptions): Promise<{ redirect?: URL; facts: PageFacts }> {
  const facts: PageFacts = { steps: [] }
  let redirect: URL | undefined
  // The redirect is read off the request, so a client host that serves nothing still works (a loopback client).
  page.on('request', (request) => {
    if (request.isNavigationRequest() && request.url().startsWith(opts.redirectUri)) redirect = new URL(request.url())
  })
  await page.goto(url, { waitUntil: 'domcontentloaded' }).catch(() => undefined)
  const deadline = Date.now() + (opts.timeoutMs ?? 30_000)
  const password = page.locator('input[type=password]').first()
  const authorize = page.getByRole('button', { name: 'Authorize', exact: true })
  const deny = page.getByRole('button', { name: 'Deny access', exact: true })
  let signedIn = false
  let decided = false
  let providerChosen = false
  let signUpStep = 0
  let welcomed = false
  while (!redirect && Date.now() < deadline) {
    // An authorization that names no account and no prompt starts on a choice between signing in and signing up.
    if (!welcomed && !signedIn && !decided) {
      const choice = page.getByRole('button', { name: opts.signUp ? 'Create a new account' : 'Sign in', exact: true })
      const welcome = page.getByText('Please authenticate to continue')
      if ((await welcome.count()) > 0 && (await choice.count()) > 0 && (await choice.first().isVisible())) {
        welcomed = true
        facts.steps.push(opts.signUp ? 'welcome:sign-up' : 'welcome:sign-in')
        await choice.first().click({ timeout: 5_000 }).catch(() => undefined)
        await page.waitForTimeout(500)
        continue
      }
    }
    if (opts.provider && !providerChosen) {
      const button = page.getByText(`Continue with ${opts.provider}`, { exact: true })
      if ((await button.count()) > 0 && (await button.first().isVisible())) {
        providerChosen = true
        facts.steps.push(`provider:${opts.provider.toLowerCase()}`)
        await button.first().click({ timeout: 5_000 }).catch(() => undefined)
        await page.waitForTimeout(600)
        continue
      }
    }
    if (opts.signUp && !decided) {
      // Step 1 of the LinkJar journey: email and password. Later steps are whatever the page asks next.
      const email = page.locator('input[name=email]')
      if (signUpStep === 0 && (await email.count()) > 0 && (await email.first().isVisible())) {
        const invite = page.locator('input[name=inviteCode]')
        if (opts.signUp.inviteCode !== undefined && (await invite.count()) > 0 && (await invite.first().isVisible())) await invite.first().fill(opts.signUp.inviteCode)
        await email.first().fill(opts.signUp.email)
        await page.locator('input[type=password]').first().fill(opts.signUp.password)
        facts.steps.push('sign-up:credentials')
        signUpStep = 1
        await page.locator('button[type=submit]').first().click({ timeout: 5_000 }).catch(() => undefined)
        await page.waitForTimeout(800)
        continue
      }
      const handle = page.locator('input[name=handle]')
      if (signUpStep === 1 && (await handle.count()) > 0 && (await handle.first().isVisible())) {
        facts.steps.push('sign-up:handle')
        facts.suggestedHandle = await handle.first().inputValue()
        await handle.first().fill(opts.signUp.handle)
        signUpStep = 2
        await page.locator('button[type=submit]').first().click({ timeout: 5_000 }).catch(() => undefined)
        await page.waitForTimeout(800)
        continue
      }
    }
    if (!signedIn && opts.password !== undefined && (await password.count()) > 0) {
      // The LinkJar journey keeps the password form behind a disclosure.
      if (!(await password.isVisible())) {
        const disclosure = page.getByText('Or use email')
        if ((await disclosure.count()) > 0) await disclosure.first().click()
      }
      if (await password.isVisible()) {
        const identifier = page.locator('input[name=username]')
        if (opts.identifier && (await identifier.count()) > 0 && (await identifier.inputValue()) === '') await identifier.fill(opts.identifier)
        await password.fill(opts.password)
        if (opts.remember) await page.locator('input[name=remember]').check({ force: true }).catch(() => undefined)
        await page.locator('button[type=submit]').first().click()
        facts.steps.push('sign-in')
        signedIn = true
        await page.waitForTimeout(400)
        continue
      }
    }
    if (!decided && (await authorize.count()) > 0 && (await authorize.first().isEnabled().catch(() => false))) {
      const text = await page.locator('body').innerText()
      if (opts.clientId) facts.showsClientId = text.includes(opts.clientId)
      if (opts.clientName) facts.showsClientName = text.includes(opts.clientName)
      facts.steps.push('consent')
      decided = true
      // The page navigates away as soon as the decision is posted.
      await (opts.decision === 'deny' ? deny : authorize).first().click({ timeout: 5_000 }).catch(() => undefined)
      continue
    }
    // A device that remembers accounts offers them before anything else.
    if (!signedIn && !decided && opts.account && !facts.steps.includes('choose-account')) {
      const choice = page.getByRole('button', { name: new RegExp(opts.account.replace(/[.]/g, '\\.')) })
      if ((await choice.count()) > 0 && (await choice.first().isVisible())) {
        facts.steps.push('choose-account')
        await choice.first().click({ timeout: 5_000 }).catch(() => undefined)
        await page.waitForTimeout(400)
        continue
      }
    }
    const alert = page.getByRole('alert')
    if ((await alert.count()) > 0 && (await alert.first().isVisible())) {
      facts.error = (await alert.first().innerText()).trim()
      if (opts.allowError) break
    }
    // A provider failure ends on a page of the server's own, outside the authorization page.
    if (opts.allowError && providerChosen && /\/oauth\/external\//.test(page.url()) && (await page.locator('body').innerText().catch(() => '')).trim() !== '') {
      await page.waitForTimeout(500)
      facts.error ??= (await page.locator('body').innerText()).trim().slice(0, 300)
      break
    }
    await page.waitForTimeout(150)
  }
  if (!redirect && !opts.allowError) {
    const text = await page.locator('body').innerText().catch(() => '')
    throw new Error(`The authorization page did not return to the client. Steps: ${facts.steps.join(', ') || 'none'}. Page says: ${text.slice(0, 300)}`)
  }
  return { redirect, facts }
}

export type SignIn = { session: OAuthSession; facts: PageFacts; redirect: URL }

/** A full authorization-code flow with the official client: PAR, the page, the token exchange. */
export async function signIn(
  s: Scenario,
  device: BrowserContext,
  client: TestClient,
  account: Account,
  opts: { scope?: string; password?: string | null; label?: string; remember?: boolean } = {},
): Promise<SignIn> {
  const label = opts.label ?? `${account.name} signs in`
  const url = await client.client.authorize(account.handle, { scope: opts.scope ?? String(client.metadata.scope), state: `state-${account.name}` })
  const page = await device.newPage()
  try {
    const { redirect, facts } = await driveAuthorization(page, String(url), {
      redirectUri: client.redirectUri,
      password: opts.password === null ? undefined : (opts.password ?? account.password),
      remember: opts.remember,
      account: account.handle,
      clientId: client.clientId,
      clientName: String(client.metadata.client_name),
    })
    assert.ok(redirect)
    // SPEC 11.1: the response names the issuer.
    assert.equal(redirect.searchParams.get('iss'), new URL(String(url)).origin, 'the authorization response carries iss')
    const { session, state } = await client.client.callback(redirect.searchParams)
    assert.equal(state, `state-${account.name}`)
    assert.equal(session.did, account.did)
    s.note(label, { page: facts, redirectParameters: [...redirect.searchParams.keys()].sort(), token: await describeSession(s, client, account.did) })
    return { session, facts, redirect }
  } finally {
    await page.close()
  }
}

/** The comparable part of the tokens the client holds for an account. */
export async function describeSession(s: Scenario, client: TestClient, did: string): Promise<Record<string, unknown>> {
  const saved = client.sessions.get(did)
  assert.ok(saved, 'the client holds a session')
  const { tokenSet } = saved
  return {
    fields: Object.keys(tokenSet).sort(),
    token_type: tokenSet.token_type,
    scope: tokenSet.scope,
    sub: tokenSet.sub,
    iss: tokenSet.iss,
    aud: tokenSet.aud,
    access_token: tokenSet.access_token,
    hasRefreshToken: Boolean(tokenSet.refresh_token),
  }
}

/** Calls the resource server through an OAuth session and records the exchange. */
export async function viaSession(
  s: Scenario,
  step: string,
  session: OAuthSession,
  path: string,
  init: { method?: string; json?: unknown } = {},
): Promise<Result> {
  const response = await session.fetchHandler(path, {
    method: init.method ?? (init.json === undefined ? 'GET' : 'POST'),
    headers: init.json === undefined ? undefined : { 'content-type': 'application/json' },
    body: init.json === undefined ? undefined : JSON.stringify(init.json),
  })
  const bytes = new Uint8Array(await response.arrayBuffer())
  const text = Buffer.from(bytes).toString('utf8')
  let json: any
  try {
    json = text ? JSON.parse(text) : undefined
  } catch {
    json = undefined
  }
  const result: Result = { status: response.status, headers: response.headers, text, bytes, json }
  s.record(step, init.method ?? (init.json === undefined ? 'GET' : 'POST'), `${path.split('?')[0]} (OAuth session)`, result)
  return result
}
