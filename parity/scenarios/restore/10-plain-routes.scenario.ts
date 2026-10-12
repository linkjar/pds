// SPEC 4.1 and 4.2 on a restored data directory: the routes outside
// /xrpc/<nsid>, the shape of an /xrpc/ path, methods, CORS, and what a
// method without a handler answers.

import assert from 'node:assert/strict'
import { loadSeed } from '../../src/lib/seed.ts'
import { get, raw, read } from '../../src/lib/reads.ts'
import { scenario } from '../../src/scenario.ts'
import { HOSTS, upstream } from '../../src/stack/targets.ts'

scenario('10-plain-routes', {}, async (s) => {
  const seed = loadSeed(s.target.name)
  const { alice, deactivated, takendown } = seed.accounts

  assert.equal((await raw(s, 'banner', '/')).status, 200)
  assert.equal((await raw(s, 'robots.txt', '/robots.txt')).status, 200)
  assert.equal((await raw(s, 'health', '/xrpc/_health')).json.version, upstream.tag.split('@').at(-1))
  await raw(s, 'a path that is not served', '/nope')
  await raw(s, 'a path with markup', '/<b>&"x')
  await raw(s, '/xrpc without a slash', '/xrpc')
  await read(s, 'POST to the banner', { method: 'POST', path: '/', body: '' })
  await read(s, 'POST to the health route', { method: 'POST', path: '/xrpc/_health', body: '' })

  // /.well-known/atproto-did answers for the host it was asked on.
  assert.equal((await read(s, 'atproto-did of a hosted handle', { host: alice.handle, path: '/.well-known/atproto-did' })).text, alice.did)
  await read(s, 'atproto-did of a deactivated account', { host: deactivated.handle, path: '/.well-known/atproto-did' })
  await read(s, 'atproto-did of an account that is taken down', { host: takendown.handle, path: '/.well-known/atproto-did' })
  await read(s, 'atproto-did of a handle nobody has', { host: `nobody${HOSTS.handleDomain}`, path: '/.well-known/atproto-did' })
  await read(s, 'atproto-did on the service host', { path: '/.well-known/atproto-did' })

  // The shape of an /xrpc/ path.
  for (const path of ['/xrpc/', '/xrpc/a', '/xrpc/a/b', '/xrpc/a..b', '/xrpc/.a', '/xrpc/a.', '/xrpc/_nope']) await raw(s, `xrpc path ${path}`, path)
  await raw(s, 'a trailing slash after the method', `/xrpc/com.atproto.sync.getLatestCommit/?did=${alice.did}`)

  // Methods.
  await read(s, 'POST to a query', { method: 'POST', path: '/xrpc/com.atproto.sync.getLatestCommit', query: { did: alice.did }, body: '' })
  await read(s, 'PUT to a query', { method: 'PUT', path: '/xrpc/com.atproto.sync.getLatestCommit', query: { did: alice.did }, body: '' })
  await read(s, 'DELETE to a method without a handler', { method: 'DELETE', path: '/xrpc/com.atproto.nope.nope' })
  await read(s, 'PUT to a path that is not a method', { method: 'PUT', path: '/xrpc/a/b', body: '' })
  const head = await s.http('HEAD of a query', { method: 'HEAD', path: '/xrpc/com.atproto.sync.getLatestCommit', query: { did: alice.did } })
  assert.equal(head.status, 200)

  // CORS (VP-11).
  const preflight = await s.http('preflight', { method: 'OPTIONS', path: '/xrpc/com.atproto.sync.getLatestCommit', headers: { origin: 'https://app.example', 'access-control-request-method': 'GET', 'access-control-request-headers': 'authorization, x-test' } })
  assert.equal(preflight.status, 204)
  await s.http('preflight without requested headers', { method: 'OPTIONS', path: '/xrpc/com.atproto.sync.getLatestCommit', headers: { origin: 'https://app.example' } })
  assert.equal((await get(s, 'a query with an Origin', 'com.atproto.sync.getLatestCommit', { did: alice.did }, { headers: { origin: 'https://app.example' } })).headers.get('access-control-allow-origin'), '*')

  // A method without a handler wants a session before anything else (VP-6).
  for (const nsid of ['com.atproto.nope.nope', 'nope', 'app.bsky.graph.getFollows', 'com.atproto.sync.listReposByCollection']) await get(s, `no handler, no credential: ${nsid}`, nsid)
  await read(s, 'no handler, POST, no credential', { method: 'POST', path: '/xrpc/com.atproto.nope.nope', json: {} })
  await get(s, 'no handler, the admin password', 'com.atproto.nope.nope', {}, { auth: 'admin' })
  await get(s, 'no handler, a token that does not verify', 'com.atproto.nope.nope', {}, { auth: 'abc' })
  await get(s, 'no handler, an authorization type nobody knows', 'com.atproto.nope.nope', {}, { headers: { authorization: 'Weird abc' } })
})
