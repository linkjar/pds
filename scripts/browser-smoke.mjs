import assert from 'node:assert/strict'
import { createHash, generateKeyPairSync, randomBytes } from 'node:crypto'
import { mkdir } from 'node:fs/promises'
import { chromium } from 'playwright'

const [unpatched, branded, production] = process.argv.slice(2)
assert(unpatched && branded, 'Supply unpatched and branding-smoke base URLs')
const browser = await chromium.launch({ headless: true })
async function authorizationUrl(base, extra = {}) {
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
      code_challenge_method: 'S256', dpop_jkt: thumbprint, ...extra,
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
  // 103-invite-handoff, against the production image, which requires invites.
  if (production) {
    const page = await browser.newPage({ locale: 'en-US' })
    page.setDefaultTimeout(20_000)
    const inviteField = page.getByLabel('Invite code', { exact: true })
    const email = page.getByLabel('Email', { exact: true })
    const focused = locator => locator.evaluate(element => element === document.activeElement)
    const googleStart = async () => new URL(await page.getByRole('link', { name: 'Continue with Google' }).getAttribute('href'), production)
    // Without a hand-off the page is unchanged: the invite field comes first.
    await page.goto(await authorizationUrl(production, { prompt: 'create' }), { waitUntil: 'networkidle' })
    await inviteField.waitFor()
    assert(await focused(inviteField))
    assert.equal((await googleStart()).searchParams.has('invite_code'), false)
    // The fragment hands off the code. The field is gone, the page drops the
    // code from the URL before its step routing, and a reload keeps the code.
    const code = 'smoke-handoff-unknown'
    await page.goto(`${await authorizationUrl(production, { prompt: 'create' })}#invite=${encodeURIComponent(code)}`, { waitUntil: 'networkidle' })
    await email.waitFor()
    assert.equal(await inviteField.count(), 0)
    assert(await focused(email))
    assert.equal(new URL(page.url()).hash, '#step=sign-up')
    assert.equal((await googleStart()).searchParams.get('invite_code'), code)
    await page.reload({ waitUntil: 'networkidle' })
    await email.waitFor()
    assert.equal(await inviteField.count(), 0)
    assert.equal(await page.locator('input[type="hidden"][name="inviteCode"]').inputValue(), code)
    await page.screenshot({ path: '.build/invite-handoff.png', fullPage: true })
    // The PDS refuses the unknown code. The wizard returns to its first step
    // with the field, focused and empty, and the server's error.
    await email.fill('handoff@example.com')
    await page.getByLabel('Password', { exact: true }).fill('correct-horse-battery')
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    await page.locator('input[name="handle"]').fill('handoffsmoke')
    const signUp = page.waitForResponse(response => response.url().endsWith('/sign-up'))
    await page.getByRole('button', { name: 'Sign up', exact: true }).click()
    const refused = await signUp
    assert.equal(refused.request().postDataJSON().inviteCode, code)
    assert.equal(refused.status(), 400)
    assert.match((await refused.json()).error_description, /^This invite code is invalid\./)
    await page.getByRole('alert').filter({ hasText: 'The invite code is not valid' }).waitFor()
    await inviteField.waitFor()
    assert(await focused(inviteField))
    assert.equal(await inviteField.inputValue(), '')
    assert.equal(await email.inputValue(), 'handoff@example.com')
    assert.equal((await googleStart()).searchParams.has('invite_code'), false)
    await page.screenshot({ path: '.build/invite-refused.png', fullPage: true })
    // The refused code is forgotten, so a reload shows the field again.
    await page.reload({ waitUntil: 'networkidle' })
    await inviteField.waitFor()
    await page.close()
  }
} finally {
  await browser.close()
}
