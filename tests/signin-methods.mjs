import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { test } from 'node:test'

const pds = '/app/packages/pds'
const { ServerMailer } = await import(`${pds}/dist/mailer/index.js`)
const { AccountManager } = await import(`${pds}/dist/account-manager/account-manager.js`)
const { OAuthStore } = await import(`${pds}/dist/account-manager/oauth-store.js`)
const { getDb, getMigrator } = await import(`${pds}/dist/account-manager/db/index.js`)
const { verifyAccountPassword } = await import(`${pds}/dist/account-manager/helpers/password.js`)
const { deliverSignInNotices } = await import(`${pds}/dist/account-manager/helpers/sign-in-notices.js`)
const did = 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa'
const enabled = ['apple', 'google', 'github']
async function fixture(run, password, { location = ':memory:', provider = 'google', email = 'a@example.com' } = {}) {
  const db = getDb(location)
  await getMigrator(db).migrateToLatestOrThrow()
  const manager = Object.assign(Object.create(AccountManager.prototype), { db })
  const store = new OAuthStore(manager)
  const identity = { provider, subject: `${provider}-a`, email, emailVerified: true }
  await manager.createAccount({ did, handle: 'alice.linkjar.social', email: identity.email,
    repoCid: { toString: () => 'fixture' }, repoRev: 'fixture', externalIdentity: identity })
  if (password) {
    await manager.updateAccountPassword({ did, password })
    assert.equal(await verifyAccountPassword(db, did, password), true)
  }
  try { await run(store, db) } finally { await db.db.destroy() }
}
test('a verified contact email is not a second sign-in method', async () => fixture(async (store, db) => {
  await assert.rejects(store.unlinkExternalIdentity(did, 'google', 'google-a', enabled), /another sign-in method/)
  assert.equal((await store.listExternalIdentities(did)).length, 1)
  assert.equal((await db.db.selectFrom('sign_in_notice').selectAll().execute()).length, 0)
}))
test('a usable password permits unlink and retry is idempotent', async () => fixture(async (store, db) => {
  await store.unlinkExternalIdentity(did, 'google', 'google-a', enabled)
  await store.unlinkExternalIdentity(did, 'google', 'google-a', enabled)
  assert.equal((await store.listExternalIdentities(did)).length, 0)
  const notices = await db.db.selectFrom('sign_in_notice').selectAll().orderBy('createdAt').execute()
  assert.deepEqual(notices.map(row => row.action), ['password-set', 'unlinked'])
}, 'fixture-password'))
test('simultaneous unlink requests cannot remove every remaining provider', async () => fixture(async store => {
  await store.linkExternalIdentity(did, { provider: 'apple', subject: 'apple-a', emailVerified: true })
  const results = await Promise.allSettled([
    store.unlinkExternalIdentity(did, 'google', 'google-a', enabled),
    store.unlinkExternalIdentity(did, 'apple', 'apple-a', enabled),
  ])
  assert.equal(results.filter(result => result.status === 'fulfilled').length, 1)
  assert.equal((await store.listExternalIdentities(did)).length, 1)
}))
test('a disabled provider does not count as a usable remaining method', async () => fixture(async store => {
  await store.linkExternalIdentity(did, { provider: 'apple', subject: 'apple-a', emailVerified: true })
  await assert.rejects(store.unlinkExternalIdentity(did, 'google', 'google-a', ['google']), /another sign-in method/)
  assert.equal((await store.listExternalIdentities(did)).length, 2)
}))
test('unlink remains scoped to the authenticated account', async () => fixture(async store => {
  await store.unlinkExternalIdentity(did, 'google', 'someone-else', enabled)
  assert.equal((await store.listExternalIdentities(did)).length, 1)
}))

test('notification failure retains the event and retries the same message after backoff', async () => fixture(async (store, db) => {
  await store.linkExternalIdentity(did, { provider: 'apple', subject: 'apple-a', emailVerified: true })
  const [original] = await db.db.selectFrom('sign_in_notice').selectAll().execute()
  assert.equal(original.action, 'linked')
  assert.equal(original.recipient, 'a@example.com')
  let attempts = 0
  const mailer = { async sendSignInMethodChange(row) {
    attempts++
    assert.equal(row.id, original.id)
    if (attempts === 1) throw new Error('fixture mail outage')
  } }
  await deliverSignInNotices(db, mailer, original.createdAt)
  const [failed] = await db.db.selectFrom('sign_in_notice').selectAll().execute()
  assert.equal(failed.attempts, 1)
  await deliverSignInNotices(db, mailer, failed.nextAttemptAt - 1)
  assert.equal(attempts, 1)
  await deliverSignInNotices(db, mailer, failed.nextAttemptAt)
  assert.equal(attempts, 2)
  assert.equal((await db.db.selectFrom('sign_in_notice').selectAll().execute()).length, 0)
}))

test('identity and its notice both roll back if the notice cannot be recorded', async () => fixture(async (store, db) => {
  await db.db.schema.dropTable('sign_in_notice').execute()
  await assert.rejects(store.linkExternalIdentity(did, { provider: 'apple', subject: 'apple-a', emailVerified: true }))
  assert.equal((await store.listExternalIdentities(did)).length, 1)
}))

test('pending security mail survives closing and reopening the account database', async () => {
  const directory = mkdtempSync(`${tmpdir()}/linkjar-notices-`)
  const location = `${directory}/account.sqlite`
  try {
    await fixture(async store => {
      await store.linkExternalIdentity(did, { provider: 'apple', subject: 'apple-a', emailVerified: true })
    }, undefined, { location })
    const reopened = getDb(location)
    try {
      const rows = await reopened.db.selectFrom('sign_in_notice').selectAll().execute()
      assert.equal(rows.length, 1)
      let delivered = 0
      await deliverSignInNotices(reopened, { async sendSignInMethodChange(row) {
        assert.equal(row.id, rows[0].id)
        delivered++
      } })
      assert.equal(delivered, 1)
      assert.equal((await reopened.db.selectFrom('sign_in_notice').selectAll().execute()).length, 0)
    } finally { await reopened.db.destroy() }
  } finally { rmSync(directory, { recursive: true, force: true }) }
})

test('an explicit Google link preserves the original Apple relay contact email', async () => fixture(async (store, db) => {
  await store.linkExternalIdentity(did, { provider: 'google', subject: 'google-a', email: 'different@gmail.com', emailVerified: true })
  const account = await db.db.selectFrom('account').select('email').where('did', '=', did).executeTakeFirstOrThrow()
  assert.equal(account.email, 'relay@privaterelay.appleid.com')
  const [notice] = await db.db.selectFrom('sign_in_notice').selectAll().execute()
  assert.equal(notice.recipient, account.email)
  const methods = await store.getSignInMethods(did)
  assert.equal(methods.hasPassword, false)
  assert.equal(methods.identities.length, 2)
  assert.equal('passwordScrypt' in methods, false)
}, undefined, { provider: 'apple', email: 'relay@privaterelay.appleid.com' }))


test('security mail uses the stored recipient and a stable message ID, and missing SMTP fails closed', async () => {
  const sent = []
  const transporter = { use() {}, async sendMail(message) { sent.push(message) } }
  const branding = { name: 'LinkJar', links: [{ rel: 'canonical', href: 'https://linkjar.io' }] }
  const notice = { id: 'fixture-notice', did, recipient: 'relay@privaterelay.appleid.com',
    action: 'linked', provider: 'google', createdAt: 0, nextAttemptAt: 0, attempts: 0 }
  const mailer = new ServerMailer(transporter, { smtpUrl: 'smtp://fixture.invalid', fromAddress: 'support@fixture.invalid' }, branding)
  await mailer.sendSignInMethodChange(notice)
  await mailer.sendSignInMethodChange(notice)
  assert.equal(sent[0].to, notice.recipient)
  assert.equal(sent[0].messageId, '<fixture-notice@linkjar.io>')
  assert.equal(sent[1].messageId, sent[0].messageId)
  assert.match(sent[0].subject, /^LinkJar:/)
  assert.match(sent[0].text, /Google was linked to your account/)
  assert.equal('html' in sent[0], false)
  await assert.rejects(new ServerMailer(transporter, null, branding).sendSignInMethodChange(notice), /requires SMTP/)
  assert.equal(sent.length, 2)
})
