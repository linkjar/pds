// Flows of the LinkJar profile (SPEC 13) that several scenarios share:
// sign-up and sign-in through an external identity provider.

import assert from 'node:assert/strict'
import type { OAuthSession } from '@atproto/oauth-client-node'
import type { NextIdentity } from '../fixtures/identity-providers.ts'
import type { BrowserContext } from '../lib/browser.ts'
import type { TestClient } from '../lib/oauth.ts'
import type { Scenario } from '../scenario.ts'
import { HOSTS } from '../stack/targets.ts'
import { driveAuthorization } from './oauth.ts'
import type { PageFacts } from './oauth.ts'

const LABEL = { apple: 'Apple', google: 'Google', github: 'GitHub' } as const

export type ExternalOutcome = { session?: OAuthSession; did?: string; facts: PageFacts; redirect?: URL }

/**
 * Starts an authorization at the server (no account named), continues with a
 * provider, and finishes the flow when the server lets it finish.
 */
export async function externalSignIn(
  s: Scenario,
  device: BrowserContext,
  client: TestClient,
  identity: NextIdentity,
  opts: { prompt?: 'create' | 'login'; expectFailure?: boolean } = {},
): Promise<ExternalOutcome> {
  await s.fixtures.identity(identity)
  const url = await client.client.authorize(`https://${HOSTS.pds}`, {
    scope: String(client.metadata.scope),
    state: `ext-${identity.subject}`,
    prompt: opts.prompt ?? 'create',
  })
  const page = await device.newPage()
  try {
    const { redirect, facts } = await driveAuthorization(page, String(url), {
      redirectUri: client.redirectUri,
      provider: LABEL[identity.provider],
      clientId: client.clientId,
      clientName: String(client.metadata.client_name),
      allowError: opts.expectFailure,
      timeoutMs: opts.expectFailure ? 10_000 : 30_000,
    })
    if (!redirect || redirect.searchParams.has('error')) {
      assert.ok(opts.expectFailure, `the provider flow failed: ${JSON.stringify(facts)} ${redirect}`)
      return { facts, redirect }
    }
    const { session } = await client.client.callback(redirect.searchParams)
    return { session, did: session.did, facts, redirect }
  } finally {
    await page.close()
  }
}

/**
 * The account pages (`/account`), where a person manages sign-in methods
 * (patch 100, SPEC 12.5). The driver relies on these names: a "Sign in"
 * choice on the welcome page, the password form of the sign-in step, a link
 * to the account's `manage` page, and the controls "Link <Provider>" and
 * "Unlink <Provider>" in a section headed "Sign-in methods".
 */
export class AccountPage {
  readonly page: import('../lib/browser.ts').Page

  private constructor(page: import('../lib/browser.ts').Page) {
    this.page = page
  }

  /** Signs in on the account pages with a password, or with a provider identity queued by the caller. */
  static async open(device: BrowserContext, opts: { identifier?: string; password?: string; provider?: 'Apple' | 'Google' | 'GitHub' }): Promise<AccountPage> {
    const page = await device.newPage()
    page.setDefaultTimeout(10_000)
    await page.goto(`https://${HOSTS.pds}/account`, { waitUntil: 'domcontentloaded' })
    await page.getByRole('button', { name: 'Sign in', exact: true }).first().click()
    if (opts.provider) {
      await page.getByText(`Continue with ${opts.provider}`, { exact: true }).first().click()
    } else {
      const password = page.locator('input[type=password]').first()
      await password.waitFor({ state: 'attached' })
      if (!(await password.isVisible())) await page.getByText('Or use email').first().click()
      await page.locator('input[name=username]').fill(opts.identifier ?? '')
      await password.fill(opts.password ?? '')
      await page.locator('input[name=remember]').check({ force: true })
      await page.locator('button[type=submit]').first().click()
    }
    const manage = page.locator('a[href$="/manage"]').first()
    await manage.waitFor({ state: 'visible', timeout: 15_000 })
    await manage.click()
    await page.getByText('Sign-in methods').first().waitFor({ state: 'visible' })
    return new AccountPage(page)
  }

  /** The providers the page lists as linked, and those it offers to link. */
  async methods(): Promise<{ linked: string[]; offered: string[] }> {
    await this.page.waitForTimeout(600)
    const text = (await this.page.locator('body').innerText()).replace(/\n+/g, ' | ')
    const linked = [...text.matchAll(/Unlink (Apple|Google|GitHub)/g)].map((m) => m[1]!.toLowerCase()).sort()
    const offered = [...text.matchAll(/Link (Apple|Google|GitHub)/g)].map((m) => m[1]!.toLowerCase()).filter((p) => !linked.includes(p)).sort()
    return { linked, offered: [...new Set(offered)] }
  }

  /**
   * Clicks "Link <Provider>" and follows the round trip through the provider
   * back to the page. Returns when the page lists the method as linked, or
   * after the wait when the server refused.
   */
  async link(provider: 'Apple' | 'Google' | 'GitHub'): Promise<void> {
    const outbound = this.page.waitForRequest((request) => request.isNavigationRequest() && /\/oauth\/external\/[a-z]+\/start/.test(request.url()), { timeout: 10_000 })
    await this.page.getByText(`Link ${provider}`, { exact: true }).first().click()
    await outbound.catch(() => undefined)
    await this.page.waitForURL(/\/account\//, { timeout: 15_000 }).catch(() => undefined)
    await this.page.getByRole('button', { name: `Unlink ${provider}`, exact: true }).first().waitFor({ state: 'visible', timeout: 6_000 }).catch(() => undefined)
  }

  async unlink(provider: 'Apple' | 'Google' | 'GitHub'): Promise<void> {
    const button = this.page.getByRole('button', { name: `Unlink ${provider}`, exact: true }).first()
    await button.click()
    await button.waitFor({ state: 'detached', timeout: 6_000 }).catch(() => undefined)
  }

  /** Whether the page lets the person remove a method. It must not, for the last one (SPEC 12.5). */
  canUnlink(provider: 'Apple' | 'Google' | 'GitHub'): Promise<boolean> {
    return this.page.getByRole('button', { name: `Unlink ${provider}`, exact: true }).first().isEnabled()
  }

  /** Whether the page shows an error after the last action. */
  async errorShown(): Promise<boolean> {
    const alert = this.page.getByRole('alert')
    return (await alert.count()) > 0 && (await alert.first().isVisible())
  }

  /** What the page says, for a scenario that looks for a specific fact. */
  text(): Promise<string> {
    return this.page.locator('body').innerText()
  }

  close(): Promise<void> {
    return this.page.close()
  }
}
