import assert from 'node:assert/strict'
import { createHash, generateKeyPairSync, randomBytes } from 'node:crypto'
import { mkdir } from 'node:fs/promises'
import { chromium } from 'playwright'

const [unpatched, branded] = process.argv.slice(2)
assert(unpatched && branded, 'Supply unpatched and branding-smoke base URLs')
const browser = await chromium.launch({ headless: true })
async function authorizationUrl(base) {
  const redirect = 'http://127.0.0.1/callback'
  const clientId = `http://localhost?${new URLSearchParams({ redirect_uri: redirect, scope: 'atproto' })}`
  const jwk = generateKeyPairSync('ec', { namedCurve: 'P-256' }).publicKey.export({ format: 'jwk' })
  const thumbprint = createHash('sha256').update(JSON.stringify({ crv: jwk.crv, kty: jwk.kty, x: jwk.x, y: jwk.y })).digest('base64url')
  const response = await fetch(`${base}/oauth/par`, {
    method: 'POST',
    body: new URLSearchParams({
      client_id: clientId, redirect_uri: redirect, scope: 'atproto', response_type: 'code',
      state: randomBytes(24).toString('base64url'),
      code_challenge: createHash('sha256').update(randomBytes(32).toString('base64url')).digest('base64url'),
      code_challenge_method: 'S256', dpop_jkt: thumbprint,
    }),
  })
  const result = await response.json()
  assert.equal(response.status, 201, JSON.stringify(result))
  assert(result.request_uri)
  return `${base}/oauth/authorize?${new URLSearchParams({ client_id: clientId, request_uri: result.request_uri })}`
}
try {
  for (const [base, title, name] of [
    [unpatched, 'Sign in', 'unpatched'],
    [branded, 'LinkJar staging sign in', 'branding-smoke'],
  ]) {
    const page = await browser.newPage()
    page.setDefaultTimeout(20_000)
    await page.goto(await authorizationUrl(base), { waitUntil: 'networkidle' })
    await page.getByRole('button', { name: 'Sign in', exact: true }).click()
    // Upstream CardTitle is a div, not a semantic heading.
    const cardTitle = page.locator('[data-slot="card-title"]').filter({ hasText: title })
    await cardTitle.waitFor()
    assert.equal(await cardTitle.textContent(), title)
    assert.equal(await page.title(), title)
    assert.equal(await page.getByLabel('Password', { exact: true }).count(), 1)
    await mkdir('.build', { recursive: true })
    await page.screenshot({ path: `.build/${name}.png`, fullPage: true })
    await page.close()
  }
} finally {
  await browser.close()
}
