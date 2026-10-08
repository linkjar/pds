// Invite codes: SPEC 10.2, with PDS_INVITE_REQUIRED set.

import assert from 'node:assert/strict'
import { refused, scenario } from '../../src/scenario.ts'

scenario('01-invites', {}, async (s) => {
  const described = await s.query('describeServer', 'com.atproto.server.describeServer')
  assert.equal(described.json.inviteCodeRequired, true)

  const account = (name: string, inviteCode?: string) =>
    s.procedure(`createAccount ${name}`, 'com.atproto.server.createAccount', { handle: s.handle(name), email: s.email(name), password: `${name}-password-1`, inviteCode })
  refused(await account('nocode'), 'InvalidInviteCode')
  refused(await account('badcode', 'pds-linkjar-social-aaaaa-bbbbb'), 'InvalidInviteCode')

  // Only the admin creates codes (SPEC 5.3).
  const single = await s.procedure('createInviteCode, two uses', 'com.atproto.server.createInviteCode', { useCount: 2 }, { auth: 'admin' })
  assert.equal(single.status, 200)
  const first = await account('first', single.json.code)
  assert.equal(first.status, 200)
  const second = await account('second', single.json.code)
  assert.equal(second.status, 200)
  refused(await account('third', single.json.code), 'InvalidInviteCode')

  const listed = await s.query('getInviteCodes', 'com.atproto.admin.getInviteCodes', {}, { auth: 'admin' })
  const entry = listed.json.codes.find((c: { code: string }) => c.code === single.json.code)
  assert.equal(entry.available, 2)
  assert.equal(entry.uses.length, 2)
  assert.equal(entry.disabled, false)
  assert.equal(entry.forAccount, 'admin')
  assert.equal(entry.createdBy, 'admin')

  // Codes for an account, and what that account sees.
  const batch = await s.procedure('createInviteCodes for an account', 'com.atproto.server.createInviteCodes', { codeCount: 2, useCount: 1, forAccounts: [first.json.did] }, { auth: 'admin' })
  assert.equal(batch.status, 200)
  assert.equal(batch.json.codes[0].account, first.json.did)
  assert.equal(batch.json.codes[0].codes.length, 2)
  const own = await s.query('getAccountInviteCodes', 'com.atproto.server.getAccountInviteCodes', {}, { auth: first.json.accessJwt })
  assert.equal(own.status, 200)
  assert.deepEqual(own.json.codes.map((c: { code: string }) => c.code).sort(), [...batch.json.codes[0].codes].sort())
  const info = await s.query('getAccountInfo shows who invited the account', 'com.atproto.admin.getAccountInfo', { did: second.json.did }, { auth: 'admin' })
  assert.equal(info.json.invitedBy.code, single.json.code)
  assert.equal(info.json.invitesDisabled, false)

  // Disabling a code, and disabling an account's invites.
  const [kept, disabled] = batch.json.codes[0].codes as [string, string]
  const disable = await s.procedure('disableInviteCodes', 'com.atproto.admin.disableInviteCodes', { codes: [disabled] }, { auth: 'admin' })
  assert.equal(disable.status, 200)
  refused(await account('disabled', disabled), 'InvalidInviteCode')
  const off = await s.procedure('disableAccountInvites', 'com.atproto.admin.disableAccountInvites', { account: first.json.did }, { auth: 'admin' })
  assert.equal(off.status, 200)
  const whileOff = await account('whileoff', kept)
  s.note('a code of an account whose invites are disabled', { status: whileOff.status, error: whileOff.json?.error })
  const ownWhileOff = await s.query('getAccountInviteCodes while invites are disabled', 'com.atproto.server.getAccountInviteCodes', {}, { auth: first.json.accessJwt })
  s.note('getAccountInviteCodes while invites are disabled', { status: ownWhileOff.status, codes: ownWhileOff.json?.codes?.length ?? null })
  const on = await s.procedure('enableAccountInvites', 'com.atproto.admin.enableAccountInvites', { account: first.json.did }, { auth: 'admin' })
  assert.equal(on.status, 200)
  if (whileOff.status !== 200) assert.equal((await account('afteron', kept)).status, 200)

  // A taken-down inviter's codes stop working.
  const inviter = await s.procedure('createInviteCodes for the second account', 'com.atproto.server.createInviteCodes', { codeCount: 1, useCount: 1, forAccounts: [second.json.did] }, { auth: 'admin' })
  await s.procedure('take the second account down', 'com.atproto.admin.updateSubjectStatus', { subject: { $type: 'com.atproto.admin.defs#repoRef', did: second.json.did }, takedown: { applied: true } }, { auth: 'admin' })
  const fromTakendown = await account('frdown', inviter.json.codes[0].codes[0])
  s.note('a code of a taken-down account', { status: fromTakendown.status, error: fromTakendown.json?.error })
})
