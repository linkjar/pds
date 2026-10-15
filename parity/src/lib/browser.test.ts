import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { test } from 'node:test'
import { chromium } from 'playwright'
import { browserPid, launchBrowser } from './browser.ts'
import type { Target } from './target.ts'

// The browser resolves every name to a closed port; the test opens no page.
const target = { state: { edgePort: 1 } } as unknown as Target

test('browser.close() kills a Chromium that does not answer', { timeout: 60_000 }, async (t) => {
  if (!existsSync(chromium.executablePath())) return t.skip('Chromium is not installed: pnpm exec playwright install chromium')
  const browser = await launchBrowser(target, { closeTimeoutMs: 1_000 })
  const pid = browserPid(browser)
  assert.ok(pid, 'Playwright exposes the browser process')
  // A stopped browser answers nothing, as a wedged one does.
  process.kill(pid, 'SIGSTOP')
  const started = Date.now()
  await browser.close()
  assert.ok(Date.now() - started < 10_000, `close() returned after ${Date.now() - started} ms`)
  assert.throws(() => process.kill(pid, 0), { code: 'ESRCH' }, 'the browser process is gone')
})
