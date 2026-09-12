import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { once } from 'node:events'
import { request } from 'node:http'

const fetch = (url, options = {}) =>
  new Promise((resolve, reject) => {
    const req = request(
      url,
      { method: options.method ?? 'GET', headers: options.headers },
      (res) => {
        let body = ''
        res.setEncoding('utf8')
        res.on('data', (chunk) => {
          body += chunk
        })
        res.on('end', () =>
          resolve({
            status: res.statusCode,
            headers: { get: (name) => res.headers[name] ?? null },
            text: async () => body,
          }),
        )
      },
    )
    req.on('error', reject)
    req.end()
  })

import { pds } from './patched-runtime.mjs'
const { deriveLinkjarHandle, isLinkjarHandle } = await import(
  `${pds}/dist/handle/linkjar.js`
)
const { isLinkjarReservedLabel, linkjarReservedLabels } = await import(
  `${pds}/dist/handle/linkjar-reserved.js`
)
const { ensureHandleServiceConstraints } = await import(
  `${pds}/dist/handle/index.js`
)
const { reservedSubdomains } = await import(`${pds}/dist/handle/reserved.js`)
const { hasExplicitSlur } = await import(`${pds}/dist/handle/explicit-slurs.js`)
const { getDb, getMigrator } = await import(
  `${pds}/dist/account-manager/db/index.js`
)
const { registerInitialHandlePolicy, claimInitialHandleRename } = await import(
  `${pds}/dist/account-manager/helpers/linkjar-handle-policy.js`
)
const { AccountManager } = await import(
  `${pds}/dist/account-manager/account-manager.js`
)
const { deleteAccount } = await import(
  `${pds}/dist/account-manager/helpers/account.js`
)
const { createRouter } = await import(`${pds}/dist/well-known.js`)
const require = createRequire(`${pds}/package.json`)
const express = require('express')
let passed = 0
async function test(name, run) {
  await run()
  passed++
  console.log(`PASS ${name}`)
}

await test('all stock reserved names and local service labels remain unavailable', () => {
  for (const label of [
    ...Object.keys(reservedSubdomains),
    ...linkjarReservedLabels,
  ]) {
    assert.throws(() =>
      ensureHandleServiceConstraints(`${label}.linkjar.social`, [
        '.linkjar.social',
      ]),
    )
  }
  for (const label of [
    'api-prod',
    'pds2',
    'link-jar',
    'l1nkjar',
    'b1uesky',
    'xn--abcd',
  ]) {
    assert.equal(isLinkjarReservedLabel(label), true, label)
  }
  for (const label of ['olivia', 'jared', 'stribog', 'jonas-doe']) {
    assert.equal(isLinkjarReservedLabel(label), false, label)
  }
  assert.throws(() =>
    ensureHandleServiceConstraints(
      'app.linkjar.social',
      ['.linkjar.social'],
      true,
    ),
  )
  ensureHandleServiceConstraints('app.other.test', ['.other.test'], true)
})
await test('Unicode names, unsafe labels and short names yield valid ASCII handles', async () => {
  for (const displayName of [
    'Zoë Ångström',
    '東京',
    '💚',
    'a',
    '---',
    'app',
    'a'.repeat(600),
    'xn--spoof',
  ]) {
    const handle = await deriveLinkjarHandle({
      displayName,
      email: 'person@example.test',
      isTaken: async () => false,
    })
    assert.equal(isLinkjarHandle(handle), true, handle)
    assert.match(handle, /^[a-z0-9-]+\.linkjar\.social$/)
    ensureHandleServiceConstraints(handle, ['.linkjar.social'])
    assert.equal(hasExplicitSlur(handle), false)
  }
  assert.equal(
    await deriveLinkjarHandle({
      displayName: 'Zoë Ångström',
      isTaken: async () => false,
    }),
    'zoe-angstrom.linkjar.social',
  )
})
await test('upstream explicit-slur filter applies before allocating derived handles', async () => {
  const disallowed = String.fromCodePoint(110, 105, 103, 103, 101, 114)
  assert.equal(hasExplicitSlur(disallowed), true)
  const handle = await deriveLinkjarHandle({
    displayName: disallowed,
    email: 'app@example.test',
    isTaken: async () => false,
  })
  assert.equal(hasExplicitSlur(handle), false)
  assert.match(handle, /^reader(?:-[a-f0-9]{6})?\.linkjar\.social$/)
})
await test('collision uses short cryptographic suffix and exhaustion is bounded', async () => {
  let checks = 0
  const handle = await deriveLinkjarHandle({
    displayName: 'stribog',
    isTaken: async () => ++checks === 1,
  })
  assert.match(handle, /^stribog-[a-f0-9]{6}\.linkjar\.social$/)
  checks = 0
  await assert.rejects(
    deriveLinkjarHandle({
      displayName: 'stribog',
      isTaken: async () => {
        checks++
        return true
      },
    }),
  )
  assert.equal(checks, 32)
  await assert.rejects(
    deriveLinkjarHandle({
      isTaken: async () => {
        throw new Error('database down')
      },
    }),
    /database down/,
  )
})
const db = getDb(':memory:')
await getMigrator(db).migrateToLatestOrThrow()
const day = 24 * 60 * 60 * 1000
const now = Date.parse('2026-09-12T12:00:00Z')
const add = async (id, age = 0, marked = true) => {
  const did = `did:plc:${id}`
  await db.transaction(async (tx) => {
    await tx.db
      .insertInto('actor')
      .values({
        did,
        handle: `${id}.linkjar.social`,
        createdAt: new Date(now - age).toISOString(),
        takedownRef: null,
        deactivatedAt: null,
        deleteAfter: null,
      })
      .execute()
    if (marked)
      await registerInitialHandlePolicy(
        tx,
        did,
        new Date(now - age).toISOString(),
      )
  })
  return did
}
try {
  await test('actual account creation records policy atomically only with internal signup flag', async () => {
    const args = {
      did: 'did:plc:newprovider',
      handle: 'newprovider.linkjar.social',
      repoCid: { toString: () => 'fixture-cid' },
      repoRev: 'fixture-rev',
    }
    await AccountManager.prototype.createAccount.call(
      { db },
      { ...args, linkjarAutoHandle: true },
    )
    assert.ok(
      await db.db
        .selectFrom('linkjar_handle_policy')
        .selectAll()
        .where('did', '=', args.did)
        .executeTakeFirst(),
    )
    await AccountManager.prototype.createAccount.call(
      { db },
      { ...args, did: 'did:plc:stock', handle: 'stock.linkjar.social' },
    )
    assert.equal(
      await db.db
        .selectFrom('linkjar_handle_policy')
        .selectAll()
        .where('did', '=', 'did:plc:stock')
        .executeTakeFirst(),
      undefined,
    )
    await assert.rejects(
      AccountManager.prototype.createAccount.call(
        { db },
        {
          ...args,
          did: 'did:plc:rollback',
          handle: 'rollback.linkjar.social',
          linkjarAutoHandle: true,
          repoCid: {
            toString: () => {
              throw new Error('repo root failure')
            },
          },
        },
      ),
      /repo root failure/,
    )
    assert.equal(
      await db.db
        .selectFrom('actor')
        .selectAll()
        .where('did', '=', 'did:plc:rollback')
        .executeTakeFirst(),
      undefined,
    )
    assert.equal(
      await db.db
        .selectFrom('linkjar_handle_policy')
        .selectAll()
        .where('did', '=', 'did:plc:rollback')
        .executeTakeFirst(),
      undefined,
    )
  })
  await test('fresh provider accounts get one hosted rename; stock accounts and custom domains retain upstream behavior', async () => {
    const did = await add('fresh')
    await claimInitialHandleRename(
      db,
      did,
      'fresh.linkjar.social',
      'fresh.linkjar.social',
      now,
    )
    await claimInitialHandleRename(
      db,
      did,
      'fresh.linkjar.social',
      'first.linkjar.social',
      now,
    )
    await assert.rejects(
      claimInitialHandleRename(
        db,
        did,
        'first.linkjar.social',
        'second.linkjar.social',
        now,
      ),
    )
    await claimInitialHandleRename(
      db,
      did,
      'first.linkjar.social',
      'my-domain.example',
      now + 100 * day,
    )
    const ordinary = await add('ordinary', 100 * day, false)
    await claimInitialHandleRename(
      db,
      ordinary,
      'ordinary.linkjar.social',
      'anything.linkjar.social',
      now,
    )
  })
  await test('window boundary and same-target recovery after ambiguous PLC errors', async () => {
    const did = await add('boundary', 30 * day)
    await assert.rejects(
      claimInitialHandleRename(
        db,
        did,
        'boundary.linkjar.social',
        'too-late.linkjar.social',
        now,
      ),
    )
    const fresh = await add('lastsecond', 30 * day - 1)
    await claimInitialHandleRename(
      db,
      fresh,
      'lastsecond.linkjar.social',
      'chosen.linkjar.social',
      now,
    )
    await claimInitialHandleRename(
      db,
      fresh,
      'lastsecond.linkjar.social',
      'chosen.linkjar.social',
      now + day,
    )
    await assert.rejects(
      claimInitialHandleRename(
        db,
        fresh,
        'lastsecond.linkjar.social',
        'different.linkjar.social',
        now + day,
      ),
    )
  })
  await test('competing rename requests cannot claim two targets', async () => {
    const did = await add('concurrent')
    const result = await Promise.allSettled(
      ['target-one', 'target-two'].map((h) =>
        claimInitialHandleRename(
          db,
          did,
          'concurrent.linkjar.social',
          `${h}.linkjar.social`,
          now,
        ),
      ),
    )
    assert.equal(result.filter((r) => r.status === 'fulfilled').length, 1)
    assert.equal(result.filter((r) => r.status === 'rejected').length, 1)
  })
  await test('actual AccountManager updateHandle gates PLC and keeps retry after failure', async () => {
    const did = await add('manager')
    // AccountManager uses real time; seed current timestamp for this test.
    await db.db
      .updateTable('linkjar_handle_policy')
      .set({ createdAt: new Date().toISOString() })
      .where('did', '=', did)
      .execute()
    let plcWrites = 0
    let fail = true
    const manager = {
      db,
      plcRotationKey: {},
      validateHandleUpdate: async (_did, handle) => ({
        account: { did, handle: 'manager.linkjar.social' },
        handle,
      }),
      plcClient: {
        updateHandle: async () => {
          plcWrites++
          if (fail) throw new Error('uncertain PLC response')
        },
      },
      updateAccountHandle: async () => {},
    }
    await assert.rejects(
      AccountManager.prototype.updateHandle.call(
        manager,
        did,
        'selected.linkjar.social',
      ),
      /uncertain/,
    )
    await assert.rejects(
      AccountManager.prototype.updateHandle.call(
        manager,
        did,
        'another.linkjar.social',
      ),
      /One hosted handle change/,
    )
    assert.equal(plcWrites, 1)
    fail = false
    await AccountManager.prototype.updateHandle.call(
      manager,
      did,
      'selected.linkjar.social',
    )
    assert.equal(plcWrites, 2)
  })
  await test('account deletion removes handle policy even without SQLite foreign key pragma', async () => {
    const did = await add('removed')
    await deleteAccount(db, did)
    assert.equal(
      await db.db
        .selectFrom('linkjar_handle_policy')
        .selectAll()
        .where('did', '=', did)
        .executeTakeFirst(),
      undefined,
    )
  })
  await test('real HTTP well-known and profile helper use hosted actor DID only', async () => {
    const app = express()
    const did = 'did:plc:abcdefghijklmnopqrstuvwx'
    const ctx = {
      cfg: { identity: { serviceHandleDomains: ['.linkjar.social'] } },
      accountManager: {
        getAccount: async (handle) =>
          handle === 'stribog.linkjar.social' ? { did } : undefined,
      },
    }
    app.use(createRouter(ctx))
    const server = app.listen(0, '0.0.0.0')
    await once(server, 'listening')
    const origin = `http://127.0.0.1:${server.address().port}`
    try {
      for (const method of ['GET', 'HEAD']) {
        const result = await fetch(
          `${origin}/.well-known/linkjar-profile?next=https://evil.test`,
          {
            method,
            headers: { Host: 'stribog.linkjar.social' },
            redirect: 'manual',
          },
        )
        assert.equal(result.status, 302)
        assert.equal(
          result.headers.get('location'),
          `https://linkjar.io/jar/${encodeURIComponent(did)}`,
        )
        assert.equal(result.headers.get('cache-control'), 'no-store')
        assert.equal(result.headers.get('set-cookie'), null)
      }
      const wellKnown = await fetch(`${origin}/.well-known/atproto-did`, {
        headers: { Host: 'stribog.linkjar.social' },
      })
      assert.equal(await wellKnown.text(), did)
      for (const host of [
        'unknown.linkjar.social',
        'pds.linkjar.social',
        'stribog.linkjar.social.evil.test',
        'stribog.linkjar.io',
        'foo.stribog.linkjar.social',
      ]) {
        assert.equal(
          (
            await fetch(`${origin}/.well-known/linkjar-profile`, {
              headers: { Host: host },
              redirect: 'manual',
            })
          ).status,
          404,
          host,
        )
      }
    } finally {
      server.close()
      await once(server, 'close')
    }
  })
} finally {
  await db.db.destroy()
}
console.log(`${passed} focused handle tests passed`)
