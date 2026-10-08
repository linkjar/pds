// Identity: SPEC 9.1 (PLC operations), 9.2 (handles), 9.4 (keys) and the
// handle route of 4.1.

import assert from 'node:assert/strict'
import { Secp256k1Keypair } from '@atproto/crypto'
import { forDids, isEvent, subscribe } from '../../src/lib/firehose.ts'
import { describeMail, mailToken, nextMail } from '../../src/lib/mail.ts'
import { summariseAll } from '../../src/oracles/firehose.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

scenario('11-identity', {}, async (s) => {
  const alice = await s.createAccount('alice')
  const bob = await s.createAccount('bob')

  // SPEC 4.1: the Host header names a hosted actor. The edge sends handle hosts to the same server.
  const wellKnown = await s.http('atproto-did on a handle host', { host: alice.handle, path: '/.well-known/atproto-did' })
  assert.equal(wellKnown.status, 200)
  assert.equal(wellKnown.text, alice.did)
  assert.match(wellKnown.headers.get('content-type') ?? '', /^text\/plain/)
  refused(await s.http('atproto-did on a handle nobody has', { host: s.handle('nobody'), path: '/.well-known/atproto-did' }))

  // resolveHandle.
  const resolved = await s.query('resolveHandle', 'com.atproto.identity.resolveHandle', { handle: alice.handle })
  assert.deepEqual(resolved.json, { did: alice.did })
  refused(await s.query('resolveHandle for a hosted handle nobody has', 'com.atproto.identity.resolveHandle', { handle: s.handle('nobody') }))
  refused(await s.query('resolveHandle with invalid syntax', 'com.atproto.identity.resolveHandle', { handle: 'not a handle' }), 'InvalidRequest')

  // The PLC document the server wrote at creation.
  const document = await s.http('PLC document', { host: HOSTS.plc, path: `/${alice.did}` })
  assert.deepEqual(document.json.alsoKnownAs, [`at://${alice.handle}`])
  assert.deepEqual(document.json.service, [{ id: '#atproto_pds', type: 'AtprotoPersonalDataServer', serviceEndpoint: `https://${HOSTS.pds}` }])
  const data = await s.http('PLC data', { host: HOSTS.plc, path: `/${alice.did}/data` })
  assert.equal(data.json.rotationKeys.length, 1, 'no recovery key is configured, so the server key is the only rotation key')

  // Handle syntax and policy on creation (SPEC 9.2).
  for (const [label, handle] of [
    ['two characters', `ab${HOSTS.handleDomain}`],
    ['nineteen characters', `${'a'.repeat(19)}${HOSTS.handleDomain}`],
    ['a dot inside the hosted label', `a.b-${s.tag}${HOSTS.handleDomain}`],
    ['a reserved name', `admin${HOSTS.handleDomain}`],
    ['an unsupported domain', `charlie-${s.tag}.example.com`],
    ['a leading hyphen', `-charlie${HOSTS.handleDomain}`],
    ['a top-level domain that is refused', `charlie-${s.tag}.invalid`],
  ] as const) {
    const result = await s.procedure(`createAccount with ${label}`, 'com.atproto.server.createAccount', { handle, email: s.email(`x${label.length}`), password: 'handle-policy-1' })
    refused(result)
  }
  refused(await s.procedure('createAccount with a handle that is taken', 'com.atproto.server.createAccount', { handle: alice.handle, email: s.email('dup'), password: 'handle-policy-1' }))
  refused(await s.procedure('createAccount with an email that is taken', 'com.atproto.server.createAccount', { handle: s.handle('dup'), email: alice.email, password: 'handle-policy-1' }))
  // Handles are case-insensitive and stored in lower case.
  const mixed = await s.procedure('createAccount with capitals in the handle', 'com.atproto.server.createAccount', { handle: `Carol-${s.tag}${HOSTS.handleDomain}`, email: s.email('carol'), password: 'handle-policy-1' })
  assert.equal(mixed.status, 200)
  assert.equal(mixed.json.handle, s.handle('carol'))

  // updateHandle: PLC first, then local state, then #identity (SPEC 9.2).
  const live = subscribe(s.target, HOSTS.pds)
  s.onCleanup(() => live.close())
  await new Promise((resolve) => setTimeout(resolve, 300))
  refused(await s.procedure('updateHandle to a handle that is taken', 'com.atproto.identity.updateHandle', { handle: alice.handle }, { auth: bob }))
  refused(await s.procedure('updateHandle to a reserved name', 'com.atproto.identity.updateHandle', { handle: `admin${HOSTS.handleDomain}` }, { auth: bob }))

  // A custom domain, verified through HTTPS /.well-known/atproto-did.
  refused(await s.procedure('updateHandle to a domain that does not verify', 'com.atproto.identity.updateHandle', { handle: HOSTS.customHandle }, { auth: bob }))
  await s.fixtures.serve({ host: HOSTS.customHandle, path: '/.well-known/atproto-did', contentType: 'text/plain', body: `${alice.did}\n` })
  refused(await s.procedure('updateHandle to a domain that names another DID', 'com.atproto.identity.updateHandle', { handle: HOSTS.customHandle }, { auth: bob }))
  await s.fixtures.serve({ host: HOSTS.customHandle, path: '/.well-known/atproto-did', contentType: 'text/plain', body: `  ${bob.did}\n` })
  const custom = await s.procedure('updateHandle to a custom domain', 'com.atproto.identity.updateHandle', { handle: HOSTS.customHandle }, { auth: bob })
  assert.equal(custom.status, 200)
  const events = await live.waitFor(forDids([bob.did]), 1)
  s.note('events of a handle change to a custom domain', await summariseAll(events))
  assert.equal(events.filter(isEvent)[0]!.body.handle, HOSTS.customHandle)
  const bobDocument = await s.http('PLC document after the handle change', { host: HOSTS.plc, path: `/${bob.did}` })
  assert.deepEqual(bobDocument.json.alsoKnownAs, [`at://${HOSTS.customHandle}`])
  const described = await s.query('describeRepo after the handle change', 'com.atproto.repo.describeRepo', { repo: bob.did })
  assert.equal(described.json.handle, HOSTS.customHandle)
  // The Reference answers from its DID cache, which still holds the document from before the change.
  s.note('handleIsCorrect right after a handle change', described.json.handleIsCorrect)
  assert.deepEqual((await s.query('resolveHandle for the custom domain', 'com.atproto.identity.resolveHandle', { handle: HOSTS.customHandle })).json, { did: bob.did })
  const signIn = await s.procedure('createSession by the custom handle', 'com.atproto.server.createSession', { identifier: HOSTS.customHandle, password: bob.password })
  assert.equal(signIn.status, 200)

  // PLC operations (SPEC 9.1).
  const recommended = await s.query('getRecommendedDidCredentials', 'com.atproto.identity.getRecommendedDidCredentials', {}, { auth: alice })
  assert.equal(recommended.status, 200)
  assert.deepEqual(recommended.json.alsoKnownAs, [`at://${alice.handle}`])
  assert.deepEqual(recommended.json.services, { atproto_pds: { type: 'AtprotoPersonalDataServer', endpoint: `https://${HOSTS.pds}` } })
  assert.deepEqual(recommended.json.rotationKeys, data.json.rotationKeys)

  const requested = await s.procedure('requestPlcOperationSignature', 'com.atproto.identity.requestPlcOperationSignature', undefined, { auth: alice })
  assert.equal(requested.status, 200)
  const plcMail = await nextMail(s, alice.email)
  s.note('PLC-operation mail', describeMail(plcMail))
  const extraKey = (await Secp256k1Keypair.create()).did()
  refused(await s.procedure('signPlcOperation without a token', 'com.atproto.identity.signPlcOperation', { rotationKeys: [extraKey, ...recommended.json.rotationKeys] }, { auth: alice }))
  refused(await s.procedure('signPlcOperation with a wrong token', 'com.atproto.identity.signPlcOperation', { token: 'AAAAA-BBBBB', rotationKeys: [extraKey, ...recommended.json.rotationKeys] }, { auth: alice }), 'InvalidToken')
  const signed = await s.procedure('signPlcOperation adding a rotation key', 'com.atproto.identity.signPlcOperation', { token: mailToken(plcMail), rotationKeys: [extraKey, ...recommended.json.rotationKeys] }, { auth: alice })
  assert.equal(signed.status, 200)
  assert.equal(signed.json.operation.type, 'plc_operation')
  assert.deepEqual(signed.json.operation.rotationKeys, [extraKey, ...recommended.json.rotationKeys])
  assert.ok(signed.json.operation.sig && signed.json.operation.prev)

  // submitPlcOperation runs safety checks before it forwards (SPEC 9.1).
  const unsafe = (change: Record<string, unknown>) => ({ ...signed.json.operation, ...change })
  refused(await s.procedure('submitPlcOperation that drops the server rotation key', 'com.atproto.identity.submitPlcOperation', { operation: unsafe({ rotationKeys: [extraKey] }) }, { auth: alice }))
  refused(await s.procedure('submitPlcOperation that points at another PDS', 'com.atproto.identity.submitPlcOperation', { operation: unsafe({ services: { atproto_pds: { type: 'AtprotoPersonalDataServer', endpoint: 'https://elsewhere.example' } } }) }, { auth: alice }))
  refused(await s.procedure('submitPlcOperation that changes the handle', 'com.atproto.identity.submitPlcOperation', { operation: unsafe({ alsoKnownAs: ['at://someone-else.test'] }) }, { auth: alice }))
  const submitted = await s.procedure('submitPlcOperation', 'com.atproto.identity.submitPlcOperation', { operation: signed.json.operation }, { auth: alice })
  assert.equal(submitted.status, 200)
  const afterSubmit = await s.http('PLC data after the operation', { host: HOSTS.plc, path: `/${alice.did}/data` })
  assert.deepEqual(afterSubmit.json.rotationKeys, [extraKey, ...recommended.json.rotationKeys])
  const log = await s.http('PLC operation log', { host: HOSTS.plc, path: `/${alice.did}/log` })
  assert.equal(log.json.length, 2)
  const identityEvents = await live.waitFor(forDids([alice.did]), 1)
  s.note('events of a submitted PLC operation', await summariseAll(identityEvents))

  // Keys and status.
  const reserved = await s.procedure('reserveSigningKey', 'com.atproto.server.reserveSigningKey', { did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' })
  assert.equal(reserved.status, 200)
  assert.match(reserved.json.signingKey, /^did:key:zQ3s/)
  const reservedAgain = await s.procedure('reserveSigningKey for the same DID', 'com.atproto.server.reserveSigningKey', { did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' })
  assert.equal(reservedAgain.json.signingKey, reserved.json.signingKey, 'the reservation is stable for a DID')
  const status = await s.query('checkAccountStatus', 'com.atproto.server.checkAccountStatus', {}, { auth: alice })
  assert.equal(status.status, 200)
  assert.equal(status.json.activated, true)
  assert.equal(status.json.validDid, true)
  assert.equal(status.json.repoBlocks, 2)
  assert.equal(status.json.indexedRecords, 0)
})
