import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import { chromium } from 'playwright'

const browser = await chromium.launch({ headless: true })
const base = process.env.EXTERNAL_UI_BASE
assert(base, 'Supply the compiled fixture origin')
await mkdir('.build', { recursive: true })
try {
  for (const colorScheme of ['light', 'dark']) {
    const context = await browser.newContext({ colorScheme, locale: 'en-US', viewport: { width: 390, height: 844 } })
    // The actual app logo, pinned with this fixture; no public-network dependency.
    await context.route('https://linkjar.io/logo/linkjar-128.png', route => route.fulfill({ path: 'tests/fixtures/linkjar-128.png' }))
    const page = await context.newPage()
    const errors = []
    page.on('pageerror', error => errors.push(error.message))
    await page.goto(`${base}/oauth/authorize?prompt=create`, { waitUntil: 'networkidle' })
    const logo = page.locator('img[src="https://linkjar.io/logo/linkjar-128.png"]')
    await logo.waitFor()
    assert(await logo.evaluate(image => image.complete && image.naturalWidth > 0))
    assert.match(await page.locator('body').innerText(), /LinkJar/)
    for (const [label, path] of [['Terms of Service', 'terms'], ['Privacy Policy', 'privacy'], ['Support', 'support']]) {
      const links = page.getByRole('link', { name: label, exact: true })
      assert(await links.count() > 0, label)
      for (const link of await links.all()) {
        // Upstream adds its public-host referral parameters to external links.
        const target = new URL(await link.getAttribute('href'))
        assert.equal(`${target.origin}${target.pathname}`, `https://policy.invalid/${path}`)
        assert(target.searchParams.keys().every(key => key.startsWith('utm_')))
      }
    }
    const colors = await page.evaluate(() => {
      const css = getComputedStyle(document.documentElement)
      return Object.fromEntries(['primary', 'error', 'warning', 'info', 'success', 'primary-contrast'].map(name => [name, css.getPropertyValue(`--branding-color-${name}`).trim()]))
    })
    assert.deepEqual(colors, { primary: '29 95 236', error: '255 56 60', warning: '245 165 36', info: '29 95 236', success: '5 150 105', 'primary-contrast': '255 255 255' })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
    // 102-signup-journey: one dark page in both schemes, Geist served from the
    // assets route under the page CSP, and the light pill as the email action.
    const journey = await page.evaluate(async () => {
      await document.fonts.ready
      const css = getComputedStyle(document.body)
      return {
        background: css.backgroundColor,
        geist: [...document.fonts].some(font => font.family === 'Geist' && font.status === 'loaded'),
        font: css.fontFamily.split(',')[0],
      }
    })
    assert.deepEqual(journey, { background: 'rgb(5, 7, 11)', geist: true, font: 'Geist' })
    const submit = page.locator('form button[type="submit"]')
    assert.equal(await submit.textContent(), 'Continue')
    const pill = await submit.evaluate(button => {
      const css = getComputedStyle(button)
      return [css.backgroundImage.startsWith('linear-gradient('), css.color, css.borderRadius, css.height]
    })
    assert.deepEqual(pill, [true, 'rgb(18, 27, 42)', '99px', '58px'])
    await page.screenshot({ path: `.build/provider-${colorScheme}.png`, fullPage: true })
    await page.goto(`${base}/account`, { waitUntil: 'networkidle' })
    await logo.waitFor()
    assert.match(await page.locator('body').innerText(), /LinkJar/)
    await page.screenshot({ path: `.build/provider-account-${colorScheme}.png`, fullPage: true })
    assert.deepEqual(errors, [])
    await context.close()
    console.log(`PASS compiled ${colorScheme} signup/account branding, palette, journey font and pill, logo and fixture policy links`)
  }
} finally {
  await browser.close()
}
