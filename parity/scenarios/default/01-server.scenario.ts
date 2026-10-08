// Server surface that needs no account: SPEC 4.1 routes, 4.2 conventions,
// and the Appendix A classifications that verify-at-pin settled (VP-5 to
// VP-7, VP-11).

import assert from 'node:assert/strict'
import { scenario } from '../../src/scenario.ts'
import { HOSTS, upstream } from '../../src/stack/targets.ts'

scenario('01-server', {}, async (s) => {
  const health = await s.http('health', { path: '/xrpc/_health' })
  assert.equal(health.status, 200)
  assert.deepEqual(health.json, { version: upstream.tag.split('@').at(-1) })

  const banner = await s.http('banner', { path: '/' })
  assert.equal(banner.status, 200)
  assert.match(banner.headers.get('content-type') ?? '', /^text\/plain/)
  assert.match(banner.text, /This is an AT Protocol Personal Data Server/)

  const robots = await s.http('robots.txt', { path: '/robots.txt' })
  assert.equal(robots.status, 200)

  const describe = await s.query('describeServer', 'com.atproto.server.describeServer')
  assert.equal(describe.status, 200)
  assert.equal(describe.json.did, `did:web:${HOSTS.pds}`)
  assert.deepEqual(describe.json.availableUserDomains, [HOSTS.handleDomain])
  assert.equal(describe.json.inviteCodeRequired, false)

  // SPEC 4.1: the Host header names a hosted actor, or the answer is 404.
  const noActor = await s.http('atproto-did on the service host', { path: '/.well-known/atproto-did' })
  assert.equal(noActor.status, 404)

  // A method with no local handler falls to the catch-all proxy (SPEC 4.4),
  // which wants a session before it looks at the target. Without one the
  // answer is 401, never 501. VP-6 and VP-7 are two such methods; the
  // forwarded half is in 08-proxy.
  for (const [label, nsid, params] of [
    ['unknown method', 'com.example.doesNotExist', {}],
    ['VP-6 resolveLexicon', 'com.atproto.lexicon.resolveLexicon', { nsid: 'app.bsky.feed.post' }],
    ['VP-7 listReposByCollection', 'com.atproto.sync.listReposByCollection', { collection: 'app.bsky.feed.post' }],
    ['relay-only getHostStatus', 'com.atproto.sync.getHostStatus', { hostname: HOSTS.pds }],
    ['relay-only listHosts', 'com.atproto.sync.listHosts', {}],
  ] as const) {
    const result = await s.query(`${label} without a credential`, nsid, params)
    assert.equal(result.status, 401, label)
    assert.equal(result.json.error, 'AuthMissing', label)
  }
  for (const nsid of ['com.atproto.sync.requestCrawl', 'com.atproto.sync.notifyOfUpdate']) {
    const result = await s.procedure(`relay-only ${nsid} without a credential`, nsid, { hostname: 'pds.example.com' })
    assert.equal(result.status, 401, nsid)
    assert.equal(result.json.error, 'AuthMissing', nsid)
  }

  // SPEC 4.2: the wrong HTTP method is invalid input, not 405.
  const wrongMethod = await s.http('POST to a query', { method: 'POST', path: '/xrpc/com.atproto.server.describeServer' })
  assert.equal(wrongMethod.status, 400)
  assert.equal(wrongMethod.json.error, 'InvalidRequest')
  const wrongMethod2 = await s.http('GET to a procedure', { path: '/xrpc/com.atproto.server.createSession' })
  assert.equal(wrongMethod2.status, 400)
  assert.equal(wrongMethod2.json.error, 'InvalidRequest')

  // SPEC 4.2: invalid input.
  const invalid = await s.query('invalid parameter', 'com.atproto.repo.getRecord', { repo: 'invalid' })
  assert.equal(invalid.status, 400)
  assert.equal(invalid.json.error, 'InvalidRequest')

  // SPEC 4.2: a missing credential is 401; an unreadable one is 400 InvalidToken.
  const anonymous = await s.query('no credential', 'com.atproto.server.getSession')
  assert.equal(anonymous.status, 401)
  assert.equal(anonymous.json.error, 'AuthMissing')
  const badToken = await s.query('garbage bearer token', 'com.atproto.server.getSession', {}, { auth: 'not-a-token' })
  assert.equal(badToken.status, 400)
  assert.equal(badToken.json.error, 'InvalidToken')

  // VP-11: CORS. The XRPC surface answers any origin and caches the
  // preflight for one day; the OAuth endpoints carry their own headers.
  const origin = 'https://app.example'
  const simple = await s.query('VP-11 GET with Origin', 'com.atproto.server.describeServer', {}, { headers: { origin }, bypass: false })
  assert.equal(simple.headers.get('access-control-allow-origin'), '*')
  const preflight = await s.http('VP-11 preflight', {
    method: 'OPTIONS',
    path: '/xrpc/com.atproto.repo.createRecord',
    headers: { origin, 'access-control-request-method': 'POST', 'access-control-request-headers': 'authorization,content-type,atproto-proxy' },
  })
  assert.equal(preflight.status, 204)
  assert.equal(preflight.headers.get('access-control-allow-origin'), '*')
  assert.equal(preflight.headers.get('access-control-max-age'), '86400')
  assert.equal(preflight.headers.get('access-control-allow-headers'), 'authorization,content-type,atproto-proxy')
  assert.equal(preflight.headers.get('access-control-allow-methods'), 'GET,HEAD,PUT,PATCH,POST,DELETE')
  const metadata = await s.http('VP-11 OAuth metadata with Origin', { path: '/.well-known/oauth-authorization-server', headers: { origin } })
  assert.equal(metadata.status, 200)
  assert.equal(metadata.headers.get('access-control-allow-origin'), '*')
  const tokenPreflight = await s.http('VP-11 token endpoint preflight', {
    method: 'OPTIONS',
    path: '/oauth/token',
    headers: { origin, 'access-control-request-method': 'POST', 'access-control-request-headers': 'content-type,dpop' },
  })
  assert.equal(tokenPreflight.headers.get('access-control-allow-origin'), '*')
  assert.equal(tokenPreflight.headers.get('access-control-allow-methods'), '*')
  assert.equal(tokenPreflight.headers.get('access-control-allow-headers'), 'Content-Type,DPoP')
  assert.equal(tokenPreflight.headers.get('access-control-max-age'), '86400')
  const resource = await s.http('VP-11 protected resource metadata with Origin', { path: '/.well-known/oauth-protected-resource', headers: { origin } })
  assert.equal(resource.headers.get('access-control-allow-origin'), '*')
  assert.deepEqual(resource.json.authorization_servers, [`https://${HOSTS.pds}`])
})
