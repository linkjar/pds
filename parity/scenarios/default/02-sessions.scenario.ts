// Legacy sessions and app passwords: SPEC 5.1 and 5.2, and VP-9 (token
// lifetimes and the refresh grace period).

import assert from 'node:assert/strict'
import { decodeJwt, reissue } from '../../src/lib/jwt.ts'
import { scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const HOUR = 60 * 60

scenario('02-sessions', {}, async (s) => {
  const alice = await s.createAccount('alice')

  // VP-9: an access token lives 120 minutes, a refresh token 90 days.
  const access = decodeJwt(alice.accessJwt)
  const refresh = decodeJwt(alice.refreshJwt)
  assert.deepEqual(access.header, { typ: 'at+jwt', alg: 'HS256' })
  assert.equal(access.claims.scope, 'com.atproto.access')
  assert.equal(access.claims.sub, alice.did)
  assert.equal(access.claims.aud, `did:web:${HOSTS.pds}`)
  assert.equal(access.claims.exp - access.claims.iat, 2 * HOUR, 'VP-9 access lifetime')
  assert.deepEqual(refresh.header, { typ: 'refresh+jwt', alg: 'HS256' })
  assert.equal(refresh.claims.scope, 'com.atproto.refresh')
  assert.equal(refresh.claims.exp - refresh.claims.iat, 90 * 24 * HOUR, 'VP-9 refresh lifetime')
  assert.ok(typeof refresh.claims.jti === 'string', 'a refresh token carries jti')

  // Sign-in by handle, by email and by DID.
  for (const [label, identifier] of [['handle', alice.handle], ['email', alice.email], ['DID', alice.did]] as const) {
    const session = await s.procedure(`createSession by ${label}`, 'com.atproto.server.createSession', { identifier, password: alice.password })
    assert.equal(session.status, 200, label)
    assert.equal(session.json.did, alice.did)
  }
  const wrong = await s.procedure('createSession with a wrong password', 'com.atproto.server.createSession', { identifier: alice.handle, password: 'not-the-password' })
  assert.equal(wrong.status, 401)
  assert.equal(wrong.json.error, 'AuthenticationRequired')
  const nobody = await s.procedure('createSession for an unknown account', 'com.atproto.server.createSession', { identifier: s.handle('nobody'), password: 'x' })
  assert.equal(nobody.status, 401)
  assert.equal(nobody.json.error, 'AuthenticationRequired')

  const session = await s.query('getSession', 'com.atproto.server.getSession', {}, { auth: alice })
  assert.equal(session.status, 200)
  assert.equal(session.json.handle, alice.handle)
  assert.equal(session.json.email, alice.email)
  assert.equal(session.json.emailConfirmed, false)
  assert.equal(session.json.active, true)

  // A token is checked by type: the refresh token is not an access token and the reverse.
  const refreshAsAccess = await s.query('getSession with the refresh token', 'com.atproto.server.getSession', {}, { auth: alice.refreshJwt })
  assert.equal(refreshAsAccess.status, 400)
  assert.equal(refreshAsAccess.json.error, 'InvalidToken')
  const accessAsRefresh = await s.procedure('refreshSession with the access token', 'com.atproto.server.refreshSession', undefined, { auth: alice.accessJwt })
  assert.equal(accessAsRefresh.status, 400)
  assert.equal(accessAsRefresh.json.error, 'InvalidToken')

  // SPEC 5.1: a token minted with the same secret is accepted, and one past its `exp` is `ExpiredToken`.
  const secret = s.target.state.secrets.jwtSecret
  const now = Math.floor(Date.now() / 1000)
  const sameSecret = await s.query('getSession with a token re-signed under the same secret', 'com.atproto.server.getSession', {}, { auth: reissue(secret, alice.accessJwt, { iat: now - 10, exp: now + 600 }) })
  assert.equal(sameSecret.status, 200)
  const expired = await s.query('getSession with an expired access token', 'com.atproto.server.getSession', {}, { auth: reissue(secret, alice.accessJwt, { iat: now - 3 * HOUR, exp: now - HOUR }) })
  assert.equal(expired.status, 400)
  assert.equal(expired.json.error, 'ExpiredToken')
  const otherSecret = await s.query('getSession with a token signed under another secret', 'com.atproto.server.getSession', {}, { auth: reissue('another-secret', alice.accessJwt, {}) })
  assert.equal(otherSecret.status, 400)
  assert.equal(otherSecret.json.error, 'InvalidToken')

  // Rotation. The first refresh mints a new pair.
  const rotated = await s.procedure('refreshSession', 'com.atproto.server.refreshSession', undefined, { auth: alice.refreshJwt })
  assert.equal(rotated.status, 200)
  assert.notEqual(rotated.json.refreshJwt, alice.refreshJwt)
  const next = decodeJwt(rotated.json.refreshJwt)
  assert.notEqual(next.claims.jti, refresh.claims.jti)

  // VP-9: the rotated token stays usable for a grace period of two hours and
  // leads to the same successor, so two racing clients end with one session.
  const again = await s.procedure('refreshSession with the rotated token, inside the grace period', 'com.atproto.server.refreshSession', undefined, { auth: alice.refreshJwt })
  assert.equal(again.status, 200)
  assert.equal(decodeJwt(again.json.refreshJwt).claims.jti, next.claims.jti, 'the same successor')
  const row = await s.target.sqlite(
    'account',
    "select (julianday(expiresAt) - julianday('now')) * 24 as hours from refresh_token where id = ?",
    [refresh.claims.jti],
  )
  assert.ok(row[0] && Math.abs(Number(row[0].hours) - 2) < 0.05, `VP-9 grace period is two hours, saw ${JSON.stringify(row)}`)
  s.note('VP-9 refresh grace period, hours', Math.round(Number(row[0]!.hours)))

  // After the grace period the rotated token is dead. The harness ages the row instead of waiting.
  await s.target.sqlite('account', "update refresh_token set expiresAt = '2000-01-01T00:00:00.000Z' where id = ?", [refresh.claims.jti])
  const stale = await s.procedure('refreshSession with the rotated token, after the grace period', 'com.atproto.server.refreshSession', undefined, { auth: alice.refreshJwt })
  assert.equal(stale.status, 400)
  assert.equal(stale.json.error, 'ExpiredToken')

  const out = await s.procedure('deleteSession', 'com.atproto.server.deleteSession', undefined, { auth: rotated.json.refreshJwt })
  assert.equal(out.status, 200)
  const afterDelete = await s.procedure('refreshSession after deleteSession', 'com.atproto.server.refreshSession', undefined, { auth: rotated.json.refreshJwt })
  assert.equal(afterDelete.status, 400)
  assert.equal(afterDelete.json.error, 'ExpiredToken')

  // App passwords (SPEC 5.2).
  const created = await s.procedure('createAppPassword', 'com.atproto.server.createAppPassword', { name: 'reader' }, { auth: alice })
  assert.equal(created.status, 200)
  assert.match(created.json.password, /^[a-z2-7]{4}-[a-z2-7]{4}-[a-z2-7]{4}-[a-z2-7]{4}$/)
  assert.equal(created.json.privileged, false)
  const privileged = await s.procedure('createAppPassword, privileged', 'com.atproto.server.createAppPassword', { name: 'chat', privileged: true }, { auth: alice })
  assert.equal(privileged.json.privileged, true)
  // Reference defect, recorded as observed: the unique constraint surfaces as
  // 500 InternalServerError. The Candidate answers 400 (SPEC 2.3, DD-3), so
  // the scenario only requires a refusal.
  const duplicate = await s.procedure('createAppPassword with a used name', 'com.atproto.server.createAppPassword', { name: 'reader' }, { auth: alice })
  assert.ok(duplicate.status >= 400)

  const appSession = await s.procedure('createSession with an app password', 'com.atproto.server.createSession', { identifier: alice.handle, password: created.json.password })
  assert.equal(appSession.status, 200)
  assert.equal(decodeJwt(appSession.json.accessJwt).claims.scope, 'com.atproto.appPass')
  const privilegedSession = await s.procedure('createSession with a privileged app password', 'com.atproto.server.createSession', { identifier: alice.handle, password: privileged.json.password })
  assert.equal(decodeJwt(privilegedSession.json.accessJwt).claims.scope, 'com.atproto.appPassPrivileged')

  // An app-password session cannot manage app passwords.
  const forbidden = await s.procedure('createAppPassword from an app-password session', 'com.atproto.server.createAppPassword', { name: 'nested' }, { auth: appSession.json.accessJwt })
  assert.equal(forbidden.status, 400)
  assert.equal(forbidden.json.error, 'InvalidToken')

  const listed = await s.query('listAppPasswords', 'com.atproto.server.listAppPasswords', {}, { auth: alice })
  assert.deepEqual(listed.json.passwords.map((p: { name: string }) => p.name).sort(), ['chat', 'reader'])
  const revoked = await s.procedure('revokeAppPassword', 'com.atproto.server.revokeAppPassword', { name: 'reader' }, { auth: alice })
  assert.equal(revoked.status, 200)
  const afterRevoke = await s.procedure('createSession with a revoked app password', 'com.atproto.server.createSession', { identifier: alice.handle, password: created.json.password })
  assert.equal(afterRevoke.status, 401)
  const revokedSession = await s.procedure('refreshSession of a revoked app password', 'com.atproto.server.refreshSession', undefined, { auth: appSession.json.refreshJwt })
  assert.equal(revokedSession.status, 400)
})
