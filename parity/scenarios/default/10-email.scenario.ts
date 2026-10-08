// Email-token flows: SPEC 10.1 (tokens) and 10.3 (mail). Confirm email,
// update email, reset password and delete account, each with the mail the
// server sends.

import assert from 'node:assert/strict'
import { forDids, isEvent, subscribe } from '../../src/lib/firehose.ts'
import { describeMail, mailToken, nextMail } from '../../src/lib/mail.ts'
import { summariseAll } from '../../src/oracles/firehose.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

scenario('10-email', {}, async (s) => {
  const alice = await s.createAccount('alice')
  let mails = 0

  // Confirm email.
  const requested = await s.procedure('requestEmailConfirmation', 'com.atproto.server.requestEmailConfirmation', undefined, { auth: alice })
  assert.equal(requested.status, 200)
  const confirmMail = await nextMail(s, alice.email, ++mails)
  s.note('confirm-email mail', describeMail(confirmMail))
  assert.equal(confirmMail.from, 'accounts@linkjar.io')
  refused(await s.procedure('confirmEmail with a wrong token', 'com.atproto.server.confirmEmail', { email: alice.email, token: 'AAAAA-BBBBB' }, { auth: alice }), 'InvalidToken')
  refused(await s.procedure('confirmEmail with another address', 'com.atproto.server.confirmEmail', { email: s.email('other'), token: mailToken(confirmMail) }, { auth: alice }), 'InvalidEmail')
  const confirmed = await s.procedure('confirmEmail', 'com.atproto.server.confirmEmail', { email: alice.email, token: mailToken(confirmMail) }, { auth: alice })
  assert.equal(confirmed.status, 200)
  const session = await s.query('getSession after confirmation', 'com.atproto.server.getSession', {}, { auth: alice })
  assert.equal(session.json.emailConfirmed, true)
  // Single use.
  refused(await s.procedure('confirmEmail with a used token', 'com.atproto.server.confirmEmail', { email: alice.email, token: mailToken(confirmMail) }, { auth: alice }), 'InvalidToken')

  // An expired token. The Reference keeps email tokens for fifteen minutes; the harness ages the row.
  await s.procedure('requestEmailConfirmation again', 'com.atproto.server.requestEmailConfirmation', undefined, { auth: alice })
  const secondMail = await nextMail(s, alice.email, ++mails)
  const tokens = await s.target.sqlite('account', "select purpose, (julianday('now') - julianday(requestedAt)) * 86400 as age from email_token where did = ?", [alice.did])
  s.note('email token rows', tokens.map((row) => row.purpose))
  await s.target.sqlite('account', "update email_token set requestedAt = '2000-01-01T00:00:00.000Z' where did = ? and purpose = 'confirm_email'", [alice.did])
  refused(await s.procedure('confirmEmail with an expired token', 'com.atproto.server.confirmEmail', { email: alice.email, token: mailToken(secondMail) }, { auth: alice }), 'ExpiredToken')

  // Update email. A confirmed address needs a token.
  const updateRequest = await s.procedure('requestEmailUpdate', 'com.atproto.server.requestEmailUpdate', undefined, { auth: alice })
  assert.equal(updateRequest.json.tokenRequired, true)
  const updateMail = await nextMail(s, alice.email, ++mails)
  s.note('update-email mail', describeMail(updateMail))
  refused(await s.procedure('updateEmail without the token', 'com.atproto.server.updateEmail', { email: s.email('alice2') }, { auth: alice }), 'TokenRequired')
  const updated = await s.procedure('updateEmail', 'com.atproto.server.updateEmail', { email: s.email('alice2'), token: mailToken(updateMail) }, { auth: alice })
  assert.equal(updated.status, 200)
  const afterUpdate = await s.query('getSession after the email update', 'com.atproto.server.getSession', {}, { auth: alice })
  assert.equal(afterUpdate.json.email, s.email('alice2'))
  assert.equal(afterUpdate.json.emailConfirmed, false, 'a new address starts unconfirmed')
  alice.email = s.email('alice2')

  // Reset password.
  const unknown = await s.procedure('requestPasswordReset for an address nobody has', 'com.atproto.server.requestPasswordReset', { email: s.email('nobody') })
  s.note('requestPasswordReset for an unknown address', unknown.status)
  const resetRequest = await s.procedure('requestPasswordReset', 'com.atproto.server.requestPasswordReset', { email: alice.email })
  assert.equal(resetRequest.status, 200)
  const resetMail = await nextMail(s, alice.email, 1)
  s.note('reset-password mail', describeMail(resetMail))
  refused(await s.procedure('resetPassword with a wrong token', 'com.atproto.server.resetPassword', { token: 'AAAAA-BBBBB', password: 'new-password-1' }), 'InvalidToken')
  const reset = await s.procedure('resetPassword', 'com.atproto.server.resetPassword', { token: mailToken(resetMail), password: 'new-password-1' })
  assert.equal(reset.status, 200)
  refused(await s.procedure('createSession with the old password', 'com.atproto.server.createSession', { identifier: alice.handle, password: alice.password }), 'AuthenticationRequired')
  const fresh = await s.procedure('createSession with the new password', 'com.atproto.server.createSession', { identifier: alice.handle, password: 'new-password-1' })
  assert.equal(fresh.status, 200)
  // A password reset ends the sessions that existed.
  const oldSession = await s.procedure('refreshSession of a session from before the reset', 'com.atproto.server.refreshSession', undefined, { auth: alice.refreshJwt })
  s.note('session from before the reset', { status: oldSession.status, error: oldSession.json?.error })
  alice.password = 'new-password-1'
  alice.accessJwt = fresh.json.accessJwt
  alice.refreshJwt = fresh.json.refreshJwt

  // Delete account (SPEC 10.1), with its firehose event (SPEC 10.4).
  const live = subscribe(s.target, HOSTS.pds)
  s.onCleanup(() => live.close())
  const deleteRequest = await s.procedure('requestAccountDelete', 'com.atproto.server.requestAccountDelete', undefined, { auth: alice })
  assert.equal(deleteRequest.status, 200)
  const deleteMail = await nextMail(s, alice.email, 2)
  s.note('delete-account mail', describeMail(deleteMail))
  refused(await s.procedure('deleteAccount with a wrong password', 'com.atproto.server.deleteAccount', { did: alice.did, password: 'wrong', token: mailToken(deleteMail) }), 'AuthenticationRequired')
  refused(await s.procedure('deleteAccount with a wrong token', 'com.atproto.server.deleteAccount', { did: alice.did, password: alice.password, token: 'AAAAA-BBBBB' }), 'InvalidToken')
  const deleted = await s.procedure('deleteAccount', 'com.atproto.server.deleteAccount', { did: alice.did, password: alice.password, token: mailToken(deleteMail) })
  assert.equal(deleted.status, 200)
  const events = await live.waitFor(forDids([alice.did]), 1)
  await live.settle()
  s.note('events of account deletion', await summariseAll(live.frames.filter(forDids([alice.did]))))
  assert.equal(events.filter(isEvent)[0]!.type, '#account')
  assert.equal(events.filter(isEvent)[0]!.body.status, 'deleted')

  // What is left of a deleted account.
  refused(await s.query('getSession of a deleted account', 'com.atproto.server.getSession', {}, { auth: alice }))
  refused(await s.procedure('createSession of a deleted account', 'com.atproto.server.createSession', { identifier: alice.handle, password: alice.password }), 'AuthenticationRequired')
  refused(await s.query('getRepo of a deleted account', 'com.atproto.sync.getRepo', { did: alice.did }))
  const status = await s.query('getRepoStatus of a deleted account', 'com.atproto.sync.getRepoStatus', { did: alice.did })
  s.note('getRepoStatus of a deleted account', { status: status.status, body: status.json })
  // The handle is free again.
  const reused = await s.procedure('createAccount reusing the handle', 'com.atproto.server.createAccount', { handle: alice.handle, email: s.email('alice3'), password: 'another-password-1' })
  assert.equal(reused.status, 200)
  assert.notEqual(reused.json.did, alice.did)
})
