// Signup policy with hCaptcha configured: SPEC 12.2, and the invariants of
// patch 099c in SPEC 13. Port of legacy/tests/signup-policy.mjs.
//
// The scenario runs on every target. A stock build accepts a fresh legacy
// createAccount here and starts with a partial hCaptcha configuration; both
// are recorded differences (differences/reference-vs-stock.json).

import assert from 'node:assert/strict'
import { scenario } from '../../src/scenario.ts'
import { HCAPTCHA_SITE_KEY } from '../../src/stack/env.ts'
import { HOSTS } from '../../src/stack/targets.ts'

scenario('01-signup-policy', { timeoutMs: 240_000 }, async (s) => {
  const { secrets } = s.target.state
  const captcha = { PDS_HCAPTCHA_SITE_KEY: HCAPTCHA_SITE_KEY, PDS_HCAPTCHA_SECRET_KEY: secrets.hcaptchaSecretKey, PDS_HCAPTCHA_TOKEN_SALT: secrets.hcaptchaTokenSalt }

  // The stack runs with all three variables: the server is up.
  const health = await s.http('health with hCaptcha configured', { path: '/xrpc/_health' })
  assert.equal(health.status, 200)

  // Partial configuration: each variable removed in turn. The LinkJar build refuses to start
  // and its message names no secret.
  const partial: Record<string, unknown> = {}
  for (const missing of Object.keys(captcha)) {
    const boot = s.target.boot({ [missing]: '' })
    partial[`without ${missing}`] = { started: boot.started, saysPartial: /Partial hCaptcha config/.test(boot.output) }
    assert.ok(!boot.output.includes(secrets.hcaptchaSecretKey), 'the message does not disclose the secret key')
    assert.ok(!boot.output.includes(secrets.hcaptchaTokenSalt), 'the message does not disclose the token salt')
    if (s.target.linkjar) {
      assert.equal(boot.started, false, `starts without ${missing}`)
      assert.match(boot.output, /Partial hCaptcha config/)
    }
  }
  s.note('start with a partial hCaptcha configuration', partial)

  // A fresh legacy createAccount would bypass the CAPTCHA of the OAuth sign-up page.
  const account = (step: string, body: Record<string, unknown>, extra = {}) => s.procedure(step, 'com.atproto.server.createAccount', body, extra)
  const fresh = await account('legacy createAccount, fresh', { handle: s.handle('fresh'), email: s.email('fresh'), password: 'fresh-password-1' })
  const withInvite = await account('legacy createAccount with an invite code', { handle: s.handle('invited'), email: s.email('invited'), password: 'fresh-password-1', inviteCode: 'pds-linkjar-social-aaaaa-bbbbb' })
  const claimingDid = await account('legacy createAccount naming a DID, without service auth', { handle: s.handle('claimed'), email: s.email('claimed'), password: 'fresh-password-1', did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' })
  if (s.target.linkjar) {
    for (const result of [fresh, withInvite, claimingDid]) {
      assert.ok(result.status >= 400)
      assert.match(result.json.message, /Create an account through the OAuth signup page/)
    }
  }

  // A migration still works: the account proves control of the DID with a
  // service token, and the ordinary validation applies.
  const mover = fresh.status === 200 ? { did: fresh.json.did, accessJwt: fresh.json.accessJwt } : undefined
  s.note('legacy signup with hCaptcha configured', { fresh: fresh.status, withInvite: withInvite.status, claimingDid: claimingDid.status, moverAvailable: Boolean(mover) })
  const metadata = await s.http('OAuth metadata', { path: '/.well-known/oauth-authorization-server' })
  assert.equal(metadata.json.issuer, `https://${HOSTS.pds}`)
})
