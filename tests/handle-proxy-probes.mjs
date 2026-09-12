import assert from 'node:assert/strict'
import { request } from 'node:http'
const origin = process.argv[2]
const probe = (host, path, method = 'GET') =>
  new Promise((resolve, reject) => {
    const req = request(
      new URL(path, origin),
      { method, headers: { Host: host, Cookie: 'attempt=1' } },
      (res) => {
        let body = ''
        res.setEncoding('utf8')
        res.on('data', (c) => (body += c))
        res.on('end', () =>
          resolve({ status: res.statusCode, headers: res.headers, body }),
        )
      },
    )
    req.on('error', reject)
    req.end()
  })
for (let i = 0; i < 50; i++) {
  try {
    const ready = await probe('pds.linkjar.social', '/account')
    if (ready.status !== 200) throw new Error('PDS not ready')
    break
  } catch (e) {
    if (i === 49) throw e
    await new Promise((r) => setTimeout(r, 100))
  }
}
for (const method of ['GET', 'HEAD'])
  for (const path of [
    '/',
    '/account',
    '/anything?next=https://evil.test',
    '/xrpc-looking',
  ]) {
    const r = await probe('stribog.linkjar.social', path, method)
    assert.equal(r.status, 302, `${method} ${path}`)
    assert.equal(
      r.headers.location,
      'https://linkjar.io/jar/did%3Aplc%3Aabcdefghijklmnopqrstuvwx',
    )
    assert.equal(r.headers['set-cookie'], undefined)
    assert.equal(r.headers['x-content-type-options'], 'nosniff')
  }
const did = await probe('stribog.linkjar.social', '/.well-known/atproto-did')
assert.equal(did.status, 200)
assert.equal(did.body, 'did:plc:abcdefghijklmnopqrstuvwx')
for (const path of [
  '/xrpc/com.atproto.sync.getBlob?did=anything&cid=anything',
  '/.well-known/oauth-authorization-server',
]) {
  const r = await probe('stribog.linkjar.social', path)
  assert.equal(r.status, 404)
  assert.equal(r.headers.location, undefined)
  assert.equal(r.headers['set-cookie'], undefined)
  assert.doesNotMatch(r.body, /user-controlled/)
}
for (const host of [
  'unknown.linkjar.social',
  'app.linkjar.social',
  'nested.stribog.linkjar.social',
  'stribog.linkjar.io',
]) {
  const r = await probe(host, '/')
  assert.equal(r.status, 404, host)
  assert.equal(r.headers.location, undefined)
}
const account = await probe('pds.linkjar.social', '/account')
assert.equal(account.status, 200)
assert.match(account.headers['set-cookie'][0], /pds-session=/)
const blob = await probe('pds.linkjar.social', '/xrpc/com.atproto.sync.getBlob')
assert.equal(blob.status, 200)
assert.match(blob.body, /user-controlled/)
console.log(
  'PASS Caddy: DID resolution, DID-based302 (GET/HEAD/arbitrarypaths), reserved/unknownhost404, no handle-host content/cookies, unchanged canonical PDS routes',
)
