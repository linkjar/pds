import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import { chromium } from 'playwright'

process.env.LINKJAR_METHODS_TEST = '1'
const fixture = process.env.EXTERNAL_UI_BASE ? undefined : await import('./external-ui-server.mjs')
const origin = process.env.EXTERNAL_UI_BASE || fixture.origin
const browser = await chromium.launch()
const did = 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa'
const api = '/@atproto/oauth-provider/~api'
try {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } })
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto(`${origin}/account/u/${did}/manage`, { waitUntil: 'networkidle' })
  await page.getByRole('heading', { name: 'Sign-in methods' }).waitFor()
  const google = page.getByRole('button', { name: 'Unlink Google', exact: true })
  assert(await google.isDisabled())
  assert.equal(await page.getByRole('link', { name: 'Link Apple', exact: true }).getAttribute('href'), `/oauth/external/apple/start?link_did=${encodeURIComponent(did)}`)
  await page.getByText('Set a password or link another method first.', { exact: true }).waitFor()
  await mkdir('.build', { recursive: true })
  await page.screenshot({ path: '.build/signin-methods-last.png', fullPage: true })

  // Exercise the real API middleware's account-page, device and CSRF boundary.
  for (const variant of ['csrf', 'account', 'bearer', 'authorization-page']) {
    const result = await page.evaluate(async ({ api, did, variant }) => {
      const csrf = decodeURIComponent(document.cookie.split('; ').find(row => row.startsWith('csrf-token='))?.slice(11) || '')
      const headers = { 'content-type': 'application/json', 'x-csrf-token': variant === 'csrf' ? 'wrong' : csrf }
      if (variant === 'bearer') headers.authorization = 'Bearer fixture-ephemeral'
      const response = await fetch(`${api}/unlink-sign-in-method`, {
        method: 'POST', mode: 'same-origin', headers,
        ...(variant === 'authorization-page' ? { referrer: `${location.origin}/oauth/authorize?request_uri=urn:ietf:params:oauth:request_uri:req-01234567890123456789012345678901` } : {}),
        body: JSON.stringify({ did: variant === 'account' ? 'did:plc:bbbbbbbbbbbbbbbbbbbbbbbb' : did, provider: 'google', subject: 'google-a' }),
      })
      return { status: response.status, body: await response.text() }
    }, { api, did, variant })
    assert(result.status >= 400 && result.status < 500, `${variant}: ${result.status}`)
    assert.match(result.body, variant === 'csrf' ? /csrf/i : variant === 'account' ? /not authenticated/ : /Manage sign-in methods from the account page/)
  }
  await page.reload({ waitUntil: 'networkidle' })
  assert(await google.isDisabled())
  await page.request.get(`${origin}/fixture/two-methods`)
  await page.reload({ waitUntil: 'networkidle' })
  await google.waitFor()
  assert.equal(await google.isDisabled(), false)
  await google.click()
  await page.getByRole('link', { name: 'Link Google', exact: true }).waitFor()
  assert(await page.getByRole('button', { name: 'Unlink Apple', exact: true }).isDisabled())
  await page.screenshot({ path: '.build/signin-methods-unlinked.png', fullPage: true })
  assert.deepEqual(errors, [])
  console.log('PASS compiled account methods UI, sole-method protection, refresh after unlink and real CSRF/device/account-page API guards')
  await context.close()
} finally {
  await browser.close()
  await fixture?.close()
}
