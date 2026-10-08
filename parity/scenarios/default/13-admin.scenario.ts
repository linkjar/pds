// Admin and moderation: SPEC 5.3 (the admin credential), 10.2 (admin
// methods, takedown with blob quarantine), 15.4 (what a taken-down account
// serves), and the #account events of 10.4.

import assert from 'node:assert/strict'
import { forDids, isEvent, subscribe } from '../../src/lib/firehose.ts'
import { describeMail, nextMail } from '../../src/lib/mail.ts'
import { NOTE, surfaces } from '../../src/oracles/account-surfaces.ts'
import { summariseAll } from '../../src/oracles/firehose.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

scenario('13-admin', {}, async (s) => {
  const alice = await s.createAccount('alice')
  const bob = await s.createAccount('bob')
  const blob = (await s.http('uploadBlob', { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body: 'admin blob', contentType: 'text/plain', auth: alice, silent: true })).json.blob
  const written = await s.procedure('record with a blob', 'com.atproto.repo.createRecord', { repo: alice.did, collection: NOTE, rkey: 'one', record: { $type: NOTE, file: blob } }, { auth: alice, silent: true })

  // SPEC 5.3: HTTP Basic as `admin`.
  const info = await s.query('getAccountInfo', 'com.atproto.admin.getAccountInfo', { did: alice.did }, { auth: 'admin' })
  assert.equal(info.status, 200)
  assert.equal(info.json.handle, alice.handle)
  assert.equal(info.json.email, alice.email)
  refused(await s.query('getAccountInfo without a credential', 'com.atproto.admin.getAccountInfo', { did: alice.did }), 'AuthenticationRequired')
  refused(await s.query('getAccountInfo with a user session', 'com.atproto.admin.getAccountInfo', { did: alice.did }, { auth: alice }))
  refused(await s.query('getAccountInfo with a wrong admin password', 'com.atproto.admin.getAccountInfo', { did: alice.did }, { headers: { authorization: 'Basic ' + Buffer.from('admin:wrong').toString('base64') } }))
  refused(await s.query('getAccountInfo with another user name', 'com.atproto.admin.getAccountInfo', { did: alice.did }, { headers: { authorization: 'Basic ' + Buffer.from(`root:${s.target.state.secrets.adminPassword}`).toString('base64') } }))
  refused(await s.query('getAccountInfo for an unknown DID', 'com.atproto.admin.getAccountInfo', { did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' }, { auth: 'admin' }))
  // The order of the answer follows the DIDs, which differ on every target.
  const infos = await s.query('getAccountInfos', 'com.atproto.admin.getAccountInfos', { dids: [alice.did, bob.did] }, { auth: 'admin', silent: true })
  assert.deepEqual(infos.json.infos.map((i: { did: string }) => i.did).sort(), [alice.did, bob.did].sort())
  s.note('getAccountInfos', [...infos.json.infos].sort((a: { handle: string }, b: { handle: string }) => a.handle.localeCompare(b.handle)))

  // Takedown of an account (SPEC 10.2, 15.4).
  const live = subscribe(s.target, HOSTS.pds)
  s.onCleanup(() => live.close())
  const mine = forDids([alice.did])
  const before = await surfaces(s, 'active', alice, blob.ref.$link)
  let mark = await live.quiet()
  const repoRef = { $type: 'com.atproto.admin.defs#repoRef', did: alice.did }
  const unset = await s.query('getSubjectStatus before any action', 'com.atproto.admin.getSubjectStatus', { did: alice.did }, { auth: 'admin' })
  assert.equal(unset.json.takedown.applied, false)
  const takedown = await s.procedure('updateSubjectStatus: take the account down', 'com.atproto.admin.updateSubjectStatus', { subject: repoRef, takedown: { applied: true, ref: 'parity-1' } }, { auth: 'admin' })
  assert.equal(takedown.status, 200)
  const set = await s.query('getSubjectStatus after the takedown', 'com.atproto.admin.getSubjectStatus', { did: alice.did }, { auth: 'admin' })
  assert.deepEqual(set.json.takedown, { applied: true, ref: 'parity-1' })
  const takedownEvents = await live.after(mark, mine, 1)
  s.note('events of an account takedown', await summariseAll(takedownEvents))
  assert.equal(takedownEvents.filter(isEvent)[0]!.body.status, 'takendown')

  const down = await surfaces(s, 'taken down', alice, blob.ref.$link)
  assert.notEqual(down.getRepo, 200)
  assert.notEqual(down.getBlob, 200)
  assert.notEqual(down.createRecord, 200)
  assert.deepEqual(down.getRepoStatus, { active: false, status: 'takendown' })
  refused(await s.procedure('createSession of a taken-down account', 'com.atproto.server.createSession', { identifier: alice.handle, password: alice.password }), 'AccountTakedown')
  refused(await s.procedure('refreshSession of a taken-down account', 'com.atproto.server.refreshSession', undefined, { auth: alice.refreshJwt }))
  // The admin still reads the repository.
  const adminExport = await s.query('getRepo of a taken-down account, as admin', 'com.atproto.sync.getRepo', { did: alice.did }, { auth: 'admin' })
  assert.equal(adminExport.status, 200)

  const reversed = await s.procedure('updateSubjectStatus: reverse the takedown', 'com.atproto.admin.updateSubjectStatus', { subject: repoRef, takedown: { applied: false } }, { auth: 'admin' })
  assert.equal(reversed.status, 200)
  mark = await live.quiet()
  const reversedEvents = live.frames.filter(mine).slice(-1)
  s.note('events of a reversed takedown', await summariseAll(reversedEvents))
  const signIn = await s.procedure('createSession after the takedown is reversed', 'com.atproto.server.createSession', { identifier: alice.handle, password: alice.password })
  assert.equal(signIn.status, 200)
  alice.accessJwt = signIn.json.accessJwt
  alice.refreshJwt = signIn.json.refreshJwt
  const after = await surfaces(s, 'restored', alice, blob.ref.$link)
  assert.deepEqual(after, before)

  // Deactivation by the admin.
  const adminDeactivate = await s.procedure('updateSubjectStatus: deactivate', 'com.atproto.admin.updateSubjectStatus', { subject: repoRef, deactivated: { applied: true } }, { auth: 'admin' })
  s.note('admin deactivation', { status: adminDeactivate.status, subject: (await s.query('getSubjectStatus', 'com.atproto.admin.getSubjectStatus', { did: alice.did }, { auth: 'admin', silent: true })).json })
  await s.procedure('updateSubjectStatus: reactivate', 'com.atproto.admin.updateSubjectStatus', { subject: repoRef, deactivated: { applied: false } }, { auth: 'admin' })

  // Takedown of one blob: quarantined, and back.
  const blobRef = { $type: 'com.atproto.admin.defs#repoBlobRef', did: alice.did, cid: blob.ref.$link }
  const blobDown = await s.procedure('updateSubjectStatus: take a blob down', 'com.atproto.admin.updateSubjectStatus', { subject: blobRef, takedown: { applied: true, ref: 'parity-blob' } }, { auth: 'admin' })
  assert.equal(blobDown.status, 200)
  refused(await s.query('getBlob of a taken-down blob', 'com.atproto.sync.getBlob', { did: alice.did, cid: blob.ref.$link }))
  const blobStatus = await s.query('getSubjectStatus of the blob', 'com.atproto.admin.getSubjectStatus', { did: alice.did, blob: blob.ref.$link }, { auth: 'admin' })
  assert.equal(blobStatus.json.takedown.applied, true)
  await s.procedure('updateSubjectStatus: restore the blob', 'com.atproto.admin.updateSubjectStatus', { subject: blobRef, takedown: { applied: false } }, { auth: 'admin' })
  const blobBack = await s.query('getBlob of a restored blob', 'com.atproto.sync.getBlob', { did: alice.did, cid: blob.ref.$link })
  assert.equal(blobBack.status, 200)
  assert.equal(Buffer.from(blobBack.bytes).toString(), 'admin blob')

  // Takedown of one record.
  const recordRef = { $type: 'com.atproto.repo.strongRef', uri: written.json.uri, cid: written.json.cid }
  const recordDown = await s.procedure('updateSubjectStatus: take a record down', 'com.atproto.admin.updateSubjectStatus', { subject: recordRef, takedown: { applied: true, ref: 'parity-record' } }, { auth: 'admin' })
  assert.equal(recordDown.status, 200)
  const hidden = await s.query('getRecord of a taken-down record', 'com.atproto.repo.getRecord', { repo: alice.did, collection: NOTE, rkey: 'one' })
  s.note('getRecord of a taken-down record', { status: hidden.status, error: hidden.json?.error })
  const hiddenList = await s.query('listRecords with a taken-down record', 'com.atproto.repo.listRecords', { repo: alice.did, collection: NOTE })
  s.note('listRecords with a taken-down record', hiddenList.json.records.map((r: { uri: string }) => r.uri.split('/').at(-1)))
  await s.procedure('updateSubjectStatus: restore the record', 'com.atproto.admin.updateSubjectStatus', { subject: recordRef, takedown: { applied: false } }, { auth: 'admin' })

  // Account administration.
  const emailChange = await s.procedure('admin updateAccountEmail', 'com.atproto.admin.updateAccountEmail', { account: bob.did, email: s.email('bob2') }, { auth: 'admin' })
  assert.equal(emailChange.status, 200)
  const handleChange = await s.procedure('admin updateAccountHandle', 'com.atproto.admin.updateAccountHandle', { did: bob.did, handle: s.handle('robert') }, { auth: 'admin' })
  assert.equal(handleChange.status, 200)
  const passwordChange = await s.procedure('admin updateAccountPassword', 'com.atproto.admin.updateAccountPassword', { did: bob.did, password: 'set-by-admin-1' }, { auth: 'admin' })
  assert.equal(passwordChange.status, 200)
  const bobSignIn = await s.procedure('createSession with the handle and password the admin set', 'com.atproto.server.createSession', { identifier: s.handle('robert'), password: 'set-by-admin-1' })
  assert.equal(bobSignIn.status, 200)
  assert.equal(bobSignIn.json.email, s.email('bob2'))

  const sent = await s.procedure('admin sendEmail', 'com.atproto.admin.sendEmail', { recipientDid: bob.did, senderDid: bob.did, subject: 'A notice from the operator', content: 'Parity harness notice.' }, { auth: 'admin' })
  assert.equal(sent.status, 200)
  assert.equal(sent.json.sent, true)
  s.note('admin mail', describeMail(await nextMail(s, s.email('bob2'))))

  // Invite administration works with invites not required.
  const code = await s.procedure('createInviteCode', 'com.atproto.server.createInviteCode', { useCount: 2 }, { auth: 'admin' })
  assert.equal(code.status, 200)
  assert.match(code.json.code, /^pds-linkjar-social-[a-z2-7]{5}-[a-z2-7]{5}$/)
  refused(await s.procedure('createInviteCode without the admin credential', 'com.atproto.server.createInviteCode', { useCount: 1 }, { auth: alice }))
  const codes = await s.query('getInviteCodes', 'com.atproto.admin.getInviteCodes', {}, { auth: 'admin' })
  assert.ok(codes.json.codes.some((c: { code: string }) => c.code === code.json.code))

  // Admin deletion, with its event.
  const deleted = await s.procedure('admin deleteAccount', 'com.atproto.admin.deleteAccount', { did: bob.did }, { auth: 'admin' })
  assert.equal(deleted.status, 200)
  mark = await live.quiet()
  const deleted2 = live.frames.filter(forDids([bob.did])).slice(-1)
  s.note('events of an admin deletion', await summariseAll(deleted2))
  refused(await s.procedure('createSession of an account the admin deleted', 'com.atproto.server.createSession', { identifier: s.handle('robert'), password: 'set-by-admin-1' }))
})
