// Account creation on the server under test (SPEC 10.1, 10.4). The other
// scenarios of a writes run take their accounts from the pool; this one
// asks the server itself, so in the Candidate's pass the Candidate signs
// the PLC operation, and the stack's PLC directory checks it.

import assert from 'node:assert/strict'
import { checkRepo } from '../../src/oracles/repo.ts'
import { refused, scenario } from '../../src/scenario.ts'

const NOTE = 'com.example.parity.note'
const CREATE = 'com.atproto.server.createAccount'

scenario('24-create-account', {}, async (s) => {
  const handle = s.handle('fresh')
  const email = s.email('fresh')
  const password = `fresh-password-${s.tag}`

  // Refusals first: nothing is created.
  const body = (change: Record<string, unknown>) => ({ handle, email, password, ...change })
  refused(await s.procedure('createAccount without an email', CREATE, { handle, password }))
  refused(await s.procedure('createAccount with an invalid email', CREATE, body({ email: 'not an address' })))
  refused(await s.procedure('createAccount with a long password', CREATE, body({ password: 'x'.repeat(257) })))
  refused(await s.procedure('createAccount under another domain', CREATE, body({ handle: 'fresh.example.org' })), 'UnsupportedDomain')
  refused(await s.procedure('createAccount with a dot in the name', CREATE, body({ handle: `a.${handle}` })), 'InvalidHandle')
  refused(await s.procedure('createAccount with a short name', CREATE, body({ handle: handle.replace(/^[^.]+/, 'ab') })), 'InvalidHandle')
  refused(await s.procedure('createAccount with a reserved name', CREATE, body({ handle: handle.replace(/^[^.]+/, 'admin') })), 'HandleNotAvailable')
  refused(await s.procedure('createAccount with a plcOp', CREATE, body({ plcOp: {} })))
  refused(await s.procedure('createAccount with a DID and no service auth', CREATE, body({ did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' })))

  const created = await s.procedure('createAccount', CREATE, body({}))
  assert.equal(created.status, 200, created.text)
  assert.equal(created.json.handle, handle)
  assert.match(created.json.did, /^did:plc:[a-z2-7]{24}$/)
  const account = { name: 'fresh', handle, email, password, did: created.json.did, accessJwt: created.json.accessJwt, refreshJwt: created.json.refreshJwt }

  refused(await s.procedure('createAccount with the handle again', CREATE, body({ email: s.email('other') })))
  refused(await s.procedure('createAccount with the email again', CREATE, body({ handle: s.handle('other') })))

  // The identity is in the directory, with this server as its PDS.
  const doc = await s.http('the DID document in the PLC directory', { host: 'plc.test', path: `/${account.did}` })
  assert.equal(doc.status, 200, doc.text)
  assert.deepEqual(doc.json.alsoKnownAs, [`at://${handle}`])
  assert.equal(doc.json.service[0].type, 'AtprotoPersonalDataServer')

  const session = await s.query('getSession', 'com.atproto.server.getSession', {}, { auth: account })
  assert.equal(session.status, 200)
  assert.equal(session.json.emailConfirmed, false)
  const login = await s.procedure('createSession', 'com.atproto.server.createSession', { identifier: handle, password })
  assert.equal(login.status, 200)
  const resolved = await s.query('resolveHandle', 'com.atproto.identity.resolveHandle', { handle })
  assert.equal(resolved.json.did, account.did)
  const described = await s.query('describeRepo', 'com.atproto.repo.describeRepo', { repo: account.did })
  assert.equal(described.status, 200)
  assert.equal(described.json.handleIsCorrect, true)

  const written = await s.procedure('createRecord', 'com.atproto.repo.createRecord', { repo: account.did, collection: NOTE, rkey: 'first', record: { $type: NOTE, text: 'hello' } }, { auth: account })
  assert.equal(written.status, 200, written.text)
  await checkRepo(s, 'for the new account', account)
})
