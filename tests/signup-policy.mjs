import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import { stripTypeScriptTypes } from 'node:module'
import { test } from 'node:test'

const pds = '/app/packages/pds'
if (process.env.LINKJAR_POLICY_SOURCE) {
  for (const file of ['config/config', 'api/com/atproto/server/createAccount']) {
    writeFileSync(`${pds}/dist/${file}.js`, stripTypeScriptTypes(
      readFileSync(`/patch-src/packages/pds/src/${file}.ts`, 'utf8'),
      { mode: 'transform' },
    ))
  }
}
const { envToCfg } = await import(`${pds}/dist/config/config.js`)
const { default: register } = await import(`${pds}/dist/api/com/atproto/server/createAccount.js`)
const captcha = { hcaptchaSiteKey: 'fixture-site', hcaptchaSecretKey: 'fixture-secret', hcaptchaTokenSalt: 'fixture-salt' }
const base = { hostname: 'pds.linkjar.social', blobstoreDiskLocation: '/tmp/unused', inviteRequired: false }

test('partial hCaptcha configuration fails closed without disclosing credentials', () => {
  const entries = Object.entries(captcha)
  for (let mask = 1; mask < 7; mask++) {
    const partial = Object.fromEntries(entries.filter((_, i) => mask & (1 << i)))
    assert.throws(() => envToCfg({ ...base, ...partial }), (error) => {
      assert.match(error.message, /Partial hCaptcha config/)
      assert.doesNotMatch(error.message, /fixture-/)
      return true
    })
  }
})
test('absent hCaptcha stays optional and complete configuration enables it', () => {
  assert.equal(envToCfg(base).oauth.provider.hcaptcha, undefined)
  assert.deepEqual(envToCfg({ ...base, ...captcha }).oauth.provider.hcaptcha,
    { siteKey: captcha.hcaptchaSiteKey, secretKey: captcha.hcaptchaSecretKey, tokenSalt: captcha.hcaptchaTokenSalt })
})

// Capture the real registered XRPC handler. The incomplete body deliberately
// stops normal validation before any database, key or PLC operation can occur.
function handler(enabled = true) {
  let result
  register({ add(_lexicon, route) { result = route.handler } }, {
    cfg: envToCfg({ ...base, ...(enabled ? captcha : {}) }),
    authVerifier: { userServiceAuthOptional: () => {} },
  })
  return result
}
function invoke(run, body = {}, did) {
  return run({ input: { body }, auth: { credentials: did ? { did } : null }, req: {} })
}
test('captcha-enabled legacy signup is rejected before account allocation', async () => {
  for (const body of [{}, { inviteCode: 'guess' }, { did: 'did:plc:claimed' }, { deactivated: true }]) {
    await assert.rejects(invoke(handler(), body), /Create an account through the OAuth signup page/)
  }
  await assert.rejects(invoke(handler(), { did: 'did:plc:other' }, 'did:plc:owner'), /Create an account through the OAuth signup page/)
  await assert.rejects(invoke(handler(), {}, 'did:plc:owner'), /Create an account through the OAuth signup page/)
})
test('authenticated import of the same DID retains ordinary validation', async () => {
  await assert.rejects(invoke(handler(), { did: 'did:plc:owner' }, 'did:plc:owner'), /Email is required/)
})
test('without hCaptcha the stock legacy validation remains available', async () => {
  await assert.rejects(invoke(handler(false)), /Email is required/)
})
