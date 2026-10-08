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
