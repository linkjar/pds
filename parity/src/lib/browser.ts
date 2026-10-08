// A headless Chromium whose every hostname resolves to the stack's edge, so
// that pages load from `https://pds.linkjar.social` and the provider and
// client fixtures by their real names. The certificate check is off in this
// browser only; the harness CA is never installed anywhere.

import { chromium } from 'playwright'
import type { Browser, BrowserContext, Page } from 'playwright'
import type { Target } from './target.ts'

export type { BrowserContext, Page }

export async function launchBrowser(target: Target): Promise<Browser> {
  return chromium.launch({
    headless: true,
    args: [`--host-resolver-rules=MAP * 127.0.0.1:${target.state.edgePort}`],
  })
}

/** One browser profile: its own cookies, which is one device session on the authorization server. */
export async function newDevice(browser: Browser, opts: { ip?: string; width?: number } = {}): Promise<BrowserContext> {
  return browser.newContext({
    ignoreHTTPSErrors: true,
    locale: 'en-US',
    viewport: { width: opts.width ?? 1280, height: 900 },
    extraHTTPHeaders: opts.ip ? { 'x-forwarded-for': opts.ip } : undefined,
  })
}
