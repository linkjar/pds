// Proxying and service auth: SPEC 4.4 and 5.4, with VP-2 (the `aud` of the
// forwarded token), VP-3 (`lxm` on inbound service auth), VP-4 (token
// lifetime), and the forwarded half of VP-6 and VP-7.

import assert from 'node:assert/strict'
import { bearer, verifyServiceToken } from '../../src/oracles/service-auth.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const APPVIEW_DID = `did:web:${HOSTS.appview}`
const MOD_DID = `did:web:${HOSTS.mod}`
const PDS_DID = `did:web:${HOSTS.pds}`

scenario('08-proxy', {}, async (s) => {
  const alice = await s.createAccount('alice')
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: 'app.bsky.graph.getFollows', body: { subject: { did: alice.did, handle: alice.handle }, follows: [] }, headers: { 'atproto-content-labelers': `${MOD_DID};redact` } })

  // A method the PDS does not serve goes to the configured AppView.
  let mark = await s.fixtures.cursor()
  const follows = await s.query('proxied query', 'app.bsky.graph.getFollows', { actor: alice.did }, { auth: alice, headers: { 'atproto-accept-labelers': `${MOD_DID};redact` } })
  assert.equal(follows.status, 200)
  assert.deepEqual(follows.json.follows, [])
  assert.equal(follows.headers.get('atproto-content-labelers'), `${MOD_DID};redact`, 'the labeler header comes back (SPEC 4.4)')
  let [forwarded] = await s.fixtures.calls(HOSTS.appview, mark)
  assert.ok(forwarded)
  assert.equal(forwarded.path, '/xrpc/app.bsky.graph.getFollows')
  assert.equal(forwarded.query.actor, alice.did)
  assert.equal(forwarded.headers['atproto-accept-labelers'], `${MOD_DID};redact`, 'the labeler header goes out (SPEC 4.4)')
  let token = await verifyServiceToken(s, bearer(forwarded.headers), alice.did)
  // VP-2: at the pin the outbound `aud` is the bare DID, without the service fragment.
  assert.equal(token.claims.aud, APPVIEW_DID, 'VP-2')
  assert.equal(token.claims.lxm, 'app.bsky.graph.getFollows')
  // VP-4: sixty seconds.
  assert.equal(token.lifetime, 60, 'VP-4')
  assert.ok(typeof token.claims.jti === 'string')
  s.note('forwarded token', { header: token.header, aud: token.claims.aud, lxm: token.claims.lxm, lifetime: token.lifetime, claims: Object.keys(token.claims).sort() })
  s.note('forwarded request headers', Object.keys(forwarded.headers).filter((h) => !['host', 'authorization', 'x-forwarded-for', 'x-forwarded-host', 'x-forwarded-proto', 'via', 'user-agent', 'accept-encoding', 'connection', 'content-length'].includes(h)).sort())

  // The proxy needs a session (SPEC 4.4).
  refused(await s.query('proxied query without a credential', 'app.bsky.graph.getFollows', { actor: alice.did }), 'AuthMissing')

  // VP-6 and VP-7, forwarded half: no local handler, so with a session both
  // go to the AppView like any unknown method.
  for (const [label, nsid, params] of [
    ['VP-6 resolveLexicon', 'com.atproto.lexicon.resolveLexicon', { nsid: 'app.bsky.feed.post' }],
    ['VP-7 listReposByCollection', 'com.atproto.sync.listReposByCollection', { collection: 'app.bsky.feed.post' }],
    ['unknown method', 'com.example.doesNotExist', {}],
  ] as const) {
    mark = await s.fixtures.cursor()
    const result = await s.query(`${label} with a session`, nsid, params, { auth: alice })
    assert.equal(result.status, 200, label)
    ;[forwarded] = await s.fixtures.calls(HOSTS.appview, mark)
    assert.equal(forwarded?.path, `/xrpc/${nsid}`, `${label} is forwarded to the AppView`)
    token = await verifyServiceToken(s, bearer(forwarded!.headers), alice.did)
    assert.equal(token.claims.lxm, nsid)
  }

  // An explicit target (SPEC 4.4): `<did>#<service id>` from the target's DID document.
  mark = await s.fixtures.cursor()
  const explicit = await s.query('explicit proxy target', 'com.example.parity.ping', {}, { auth: alice, headers: { 'atproto-proxy': `${MOD_DID}#atproto_labeler` } })
  assert.equal(explicit.status, 200)
  const [toMod] = await s.fixtures.calls(HOSTS.mod, mark)
  assert.equal(toMod?.path, '/xrpc/com.example.parity.ping')
  token = await verifyServiceToken(s, bearer(toMod!.headers), alice.did)
  assert.equal(token.claims.aud, MOD_DID, 'VP-2 holds for an explicit target')
  refused(await s.query('proxy target with a service id the document lacks', 'com.example.parity.ping', {}, { auth: alice, headers: { 'atproto-proxy': `${MOD_DID}#no_such_service` } }))
  refused(await s.query('proxy target without a service id', 'com.example.parity.ping', {}, { auth: alice, headers: { 'atproto-proxy': MOD_DID } }))
  refused(await s.query('proxy target that is not a DID', 'com.example.parity.ping', {}, { auth: alice, headers: { 'atproto-proxy': 'not-a-did#service' } }))
  refused(await s.query('proxy target whose DID does not resolve', 'com.example.parity.ping', {}, { auth: alice, headers: { 'atproto-proxy': 'did:web:client.parity.linkjar.io#service' } }))
  // The Reference does not parse the labeler header: a malformed value goes
  // to the upstream as it is, and the upstream decides.
  mark = await s.fixtures.cursor()
  const malformed = await s.query('malformed labeler header', 'app.bsky.graph.getFollows', { actor: alice.did }, { auth: alice, headers: { 'atproto-accept-labelers': 'not a did;;' } })
  assert.equal(malformed.status, 200)
  ;[forwarded] = await s.fixtures.calls(HOSTS.appview, mark)
  assert.equal(forwarded?.headers['atproto-accept-labelers'], 'not a did;;')

  // Procedures are forwarded with their body.
  mark = await s.fixtures.cursor()
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: 'app.bsky.graph.muteActor', body: {} })
  const procedure = await s.procedure('proxied procedure', 'app.bsky.graph.muteActor', { actor: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' }, { auth: alice })
  assert.equal(procedure.status, 200)
  ;[forwarded] = await s.fixtures.calls(HOSTS.appview, mark)
  assert.equal(forwarded?.method, 'POST')
  assert.deepEqual(JSON.parse(forwarded!.body), { actor: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' })

  // What the upstream answers comes back: an XRPC error as it is, a failure as UpstreamFailure.
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: 'app.bsky.graph.getBlocks', status: 400, body: { error: 'InvalidRequest', message: 'upstream said no' } })
  const upstreamError = await s.query('upstream answers an XRPC error', 'app.bsky.graph.getBlocks', {}, { auth: alice })
  assert.equal(upstreamError.status, 400)
  assert.deepEqual(upstreamError.json, { error: 'InvalidRequest', message: 'upstream said no' })
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: 'app.bsky.graph.getMutes', status: 500, body: 'boom' })
  const upstreamFailure = await s.query('upstream fails', 'app.bsky.graph.getMutes', {}, { auth: alice })
  s.note('upstream 500', { status: upstreamFailure.status, error: upstreamFailure.json?.error })

  // Account management cannot be reached through a service token or the proxy.
  refused(await s.query('getServiceAuth for a protected method', 'com.atproto.server.getServiceAuth', { aud: APPVIEW_DID, lxm: 'com.atproto.server.createAppPassword' }, { auth: alice }))

  // getServiceAuth (SPEC 5.4). VP-4: the default lifetime is sixty seconds.
  const issued = await s.query('getServiceAuth', 'com.atproto.server.getServiceAuth', { aud: APPVIEW_DID, lxm: 'app.bsky.feed.getTimeline' }, { auth: alice })
  assert.equal(issued.status, 200)
  token = await verifyServiceToken(s, issued.json.token, alice.did)
  assert.equal(token.lifetime, 60, 'VP-4')
  assert.equal(token.claims.aud, APPVIEW_DID)
  assert.equal(token.claims.lxm, 'app.bsky.feed.getTimeline')
  // A requested expiry is an absolute time, so these exchanges are kept out
  // of the transcript and their outcomes recorded instead.
  const now = Math.floor(Date.now() / 1000)
  const withExp = async (step: string, exp: number, lxm?: string) => {
    const result = await s.query(step, 'com.atproto.server.getServiceAuth', { aud: APPVIEW_DID, lxm, exp }, { auth: alice, silent: true })
    s.note(step, result.status === 200 ? { status: 200 } : { status: result.status, error: result.json?.error, message: result.json?.message })
    return result
  }
  const longer = await withExp('getServiceAuth with exp in 30 minutes', now + 1800, 'app.bsky.feed.getTimeline')
  assert.equal(longer.status, 200)
  assert.equal((await verifyServiceToken(s, longer.json.token, alice.did)).claims.exp, now + 1800, 'the requested expiry is used as given')
  refused(await withExp('getServiceAuth with exp over an hour away', now + 3700, 'app.bsky.feed.getTimeline'), 'BadExpiration')
  refused(await withExp('getServiceAuth with exp in the past', now - 10, 'app.bsky.feed.getTimeline'), 'BadExpiration')
  refused(await withExp('getServiceAuth without lxm and exp over a minute away', now + 120), 'BadExpiration')
  const methodless = await s.query('getServiceAuth without lxm', 'com.atproto.server.getServiceAuth', { aud: PDS_DID }, { auth: alice })
  assert.equal(methodless.status, 200)
  assert.equal((await verifyServiceToken(s, methodless.json.token, alice.did)).claims.lxm, undefined)

  // Inbound service auth (SPEC 5.4). uploadBlob accepts a token the account
  // minted for this server and this method.
  const upload = (step: string, bearerToken: string) =>
    s.http(step, { method: 'POST', path: '/xrpc/com.atproto.repo.uploadBlob', body: 'service auth upload', contentType: 'text/plain', auth: bearerToken })
  const forUpload = await s.query('getServiceAuth for uploadBlob on this server', 'com.atproto.server.getServiceAuth', { aud: PDS_DID, lxm: 'com.atproto.repo.uploadBlob' }, { auth: alice })
  const accepted = await upload('uploadBlob with a service token', forUpload.json.token)
  assert.equal(accepted.status, 200)
  // VP-3: a token without `lxm` is refused where a method is expected. On
  // uploadBlob the Reference tells a service token from a session token by
  // the presence of `lxm`, so a method-less token is read as a session token
  // and fails as one. On createAccount it reaches the service-auth verifier,
  // which names the missing claim.
  refused(await upload('VP-3 uploadBlob with a service token that has no lxm', methodless.json.token), 'InvalidToken')
  const fresh = await s.query('getServiceAuth without lxm, again', 'com.atproto.server.getServiceAuth', { aud: PDS_DID }, { auth: alice })
  const noLxm = await s.procedure('VP-3 createAccount with a service token that has no lxm', 'com.atproto.server.createAccount', { handle: s.handle('moved'), did: alice.did }, { auth: fresh.json.token })
  refused(noLxm)
  s.note('VP-3 createAccount without lxm', { status: noLxm.status, error: noLxm.json?.error, message: noLxm.json?.message })
  const otherMethod = await s.query('getServiceAuth for another method', 'com.atproto.server.getServiceAuth', { aud: PDS_DID, lxm: 'com.atproto.repo.createRecord' }, { auth: alice })
  refused(await upload('uploadBlob with a service token for another method', otherMethod.json.token))
  const otherAudience = await s.query('getServiceAuth for another audience', 'com.atproto.server.getServiceAuth', { aud: APPVIEW_DID, lxm: 'com.atproto.repo.uploadBlob' }, { auth: alice })
  refused(await upload('uploadBlob with a service token for another audience', otherAudience.json.token))
  // A replayed token: the same jti twice.
  const replay = await upload('uploadBlob with the same service token again', forUpload.json.token)
  s.note('replayed service token', { status: replay.status, error: replay.json?.error })

  // createReport is forwarded to the report service (SPEC 10.2).
  mark = await s.fixtures.cursor()
  const report = { id: 7, reasonType: 'com.atproto.moderation.defs#reasonSpam', subject: { $type: 'com.atproto.admin.defs#repoRef', did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' }, reportedBy: alice.did, createdAt: '2026-01-01T00:00:00.000Z' }
  await s.fixtures.xrpc({ host: HOSTS.mod, nsid: 'com.atproto.moderation.createReport', body: report })
  const reported = await s.procedure('createReport', 'com.atproto.moderation.createReport', { reasonType: report.reasonType, subject: report.subject, reason: 'parity' }, { auth: alice })
  assert.equal(reported.status, 200)
  assert.equal(reported.json.id, 7)
  const [toReports] = await s.fixtures.calls(HOSTS.mod, mark)
  assert.equal(toReports?.path, '/xrpc/com.atproto.moderation.createReport')
  token = await verifyServiceToken(s, bearer(toReports!.headers), alice.did)
  assert.equal(token.claims.aud, MOD_DID)
  assert.equal(token.claims.lxm, 'com.atproto.moderation.createReport')
  assert.equal(JSON.parse(toReports!.body).reason, 'parity')
})
