import assert from 'node:assert/strict'
import { test } from 'node:test'
import { Normaliser } from './normalise.ts'

test('identifiers are aliased by first sight, per kind', () => {
  const a = new Normaliser()
  const b = new Normaliser()
  const left = a.value({
    did: 'did:plc:e6d3i5xexqzag6j6t2kupjm6',
    uri: 'at://did:plc:e6d3i5xexqzag6j6t2kupjm6/app.bsky.feed.post/3mxfhz6es7s2p',
    cid: 'bafyreiakjysges5gqcg5yc2btbqyglrekv4ikboapvljxzv5pvm77zrnxi',
    other: 'did:plc:n7txbmihbbs3tdnevrtznvmi',
  })
  const right = b.value({
    other: 'did:plc:hsc4szg72qo4ei33cqg33b6g',
    cid: 'bafyreigu72joyh4qk66f4a5m5fxhzgipzyr6vsosunwtkfunu7qla4x33e',
    uri: 'at://did:plc:aaaaaaaaaaaaaaaaaaaaaaaa/app.bsky.feed.post/3mxfi43azkc26',
    did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa',
  })
  // Keys are visited in sorted order, so first sight is the same on both sides.
  assert.deepEqual(left, right)
  assert.deepEqual(left, {
    cid: '<cid:1>',
    did: 'did:plc:<1>',
    other: 'did:plc:<2>',
    uri: 'at://did:plc:<1>/app.bsky.feed.post/<tid:1>',
  })
})

test('a repeated identifier keeps its alias and a different one gets the next', () => {
  const n = new Normaliser()
  assert.equal(n.text('3mxfhz6es7s2p then 3mxfi43azkc26 then 3mxfhz6es7s2p'), '<tid:1> then <tid:2> then <tid:1>')
})

test('times become a marker and ordinary words are left alone', () => {
  const n = new Normaliser()
  assert.equal(n.text('created 2026-10-08T22:10:05.242Z'), 'created <time>')
  assert.equal(n.text('InternalServerError: authentication required'), 'InternalServerError: authentication required')
  assert.equal(n.text('app.bsky.feed.post'), 'app.bsky.feed.post')
})

test('a token is reduced to the claims that must match, with its lifetime', () => {
  const part = (value: unknown) => Buffer.from(JSON.stringify(value)).toString('base64url')
  const token = [
    part({ alg: 'HS256', typ: 'at+jwt' }),
    part({ scope: 'com.atproto.access', sub: 'did:plc:e6d3i5xexqzag6j6t2kupjm6', aud: 'did:web:pds.linkjar.social', iat: 1000, exp: 8200 }),
    'c2lnbmF0dXJl',
  ].join('.')
  const n = new Normaliser()
  assert.equal(
    n.value({ accessJwt: token }) && n.text(token),
    '<jwt alg=HS256 typ=at+jwt scope=com.atproto.access sub=did:plc:<1> aud=did:web:pds.linkjar.social lifetime=7200>',
  )
})

test('bytes are reduced to their length and field order is ignored', () => {
  const n = new Normaliser()
  assert.deepEqual(n.value({ b: new Uint8Array(64), a: [1, 'x'] }), { a: [1, 'x'], b: '<bytes:64>' })
  assert.equal(JSON.stringify(n.value({ z: 1, a: 2 })), '{"a":2,"z":1}')
})
