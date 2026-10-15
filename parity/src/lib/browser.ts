// A headless Chromium whose every hostname resolves to the stack's edge, so
// that pages load from `https://pds.linkjar.social` and the provider and
// client fixtures by their real names. The certificate check is off in this
// browser only; the harness CA is never installed anywhere.

import { chromium } from 'playwright'
import type { Browser, BrowserContext, Page } from 'playwright'
import { bound, describe, TimeoutError } from './bound.ts'
import type { Target } from './target.ts'

export type { BrowserContext, Page }

/**
 * How long `browser.close()` waits for Chromium to go before killing it.
 * Playwright kills a browser that ignores its close after 30 s of its own;
 * this is shorter, so that a scenario's cleanup bound (scenario.ts) is never
 * what gives up on the browser.
 */
export const BROWSER_CLOSE_TIMEOUT_MS = 15_000
/** How long the kill may take: the process group is sent SIGKILL and its temporary profile removed. */
const KILL_TIMEOUT_MS = 5_000

export type LaunchOptions = { closeTimeoutMs?: number }

/**
 * Launches the browser. Its `close()` is bounded: when Chromium has not gone
 * within `closeTimeoutMs`, its process is killed and `close()` returns.
 */
export async function launchBrowser(target: Target, opts: LaunchOptions = {}): Promise<Browser> {
  const browser = await chromium.launch({
    headless: true,
    args: [`--host-resolver-rules=MAP * 127.0.0.1:${target.state.edgePort}`],
  })
  const close = browser.close.bind(browser)
  const timeoutMs = opts.closeTimeoutMs ?? BROWSER_CLOSE_TIMEOUT_MS
  browser.close = async (options) => {
    try {
      await bound(close(options), timeoutMs, 'browser.close()')
    } catch (error) {
      if (!(error instanceof TimeoutError)) throw error
      console.error(`${error.message}; killing Chromium (pid ${browserPid(browser) ?? 'unknown'})`)
      await killBrowser(browser)
    }
  }
  return browser
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

// The handle on the browser process is Playwright's own, behind the client
// object (playwright-core, `toImpl` and `browserProcess.kill`): not public
// API. When a Playwright bump moves it, the kill is skipped with a line in
// the log, and the runner's --test-force-exit (run.ts) still ends the
// scenario process, with the browser, once the test has ended.
type Internals = {
  _connection?: {
    toImpl?: (object: unknown) => { options?: { browserProcess?: BrowserProcess } } | undefined
  }
}
type BrowserProcess = { process?: { pid?: number }; kill?: () => Promise<void> }

function browserProcess(browser: Browser): BrowserProcess | undefined {
  return (browser as unknown as Internals)._connection?.toImpl?.(browser)?.options?.browserProcess
}

/** The process id of the launched Chromium, when Playwright exposes it. */
export function browserPid(browser: Browser): number | undefined {
  return browserProcess(browser)?.process?.pid
}

async function killBrowser(browser: Browser): Promise<void> {
  const process = browserProcess(browser)
  if (!process?.kill) {
    console.error('no handle on the Chromium process; it stays until the scenario process exits')
    return
  }
  try {
    await bound(process.kill(), KILL_TIMEOUT_MS, 'killing Chromium')
  } catch (error) {
    console.error(`killing Chromium: ${describe(error)}`)
  }
}
