// Parameter validation of the read methods (SPEC 4.2): the Candidate must
// answer an invalid request with the Reference's message. The cases follow
// the two phases of the Reference's validator: conversion in the order the
// query was written, then the checks in the order of the lexicon.

import { loadSeed } from '../../src/lib/seed.ts'
import { raw } from '../../src/lib/reads.ts'
import { scenario } from '../../src/scenario.ts'

scenario('13-parameters', { timeoutMs: 300_000 }, async (s) => {
  const seed = loadSeed(s.target.name)
  const a = seed.accounts.alice.did
  const post = 'app.bsky.feed.post'
  const x = '/xrpc/com.atproto'
  const cid = 'bafyreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku'
  const cases = [
    // Required and empty.
    `${x}.sync.getLatestCommit`,
    `${x}.sync.getLatestCommit?did=`,
    `${x}.sync.getLatestCommit?did=${a}&did=`,
    `${x}.sync.getLatestCommit?did=${a}&did=${a}`,
    `${x}.sync.getLatestCommit?did=${a}&did=${a}&did=${a}`,
    `${x}.sync.getLatestCommit?did=${a}&unknown=1&unknown=2`,
    `${x}.sync.getRepo`,
    `${x}.sync.getRecord?did=${a}`,
    `${x}.sync.getRecord?did=${a}&collection=${post}`,
    `${x}.sync.getBlocks?did=${a}`,
    `${x}.sync.getBlocks?did=${a}&cids=`,
    `${x}.sync.getBlob?did=${a}`,
    `${x}.repo.getRecord?repo=${a}`,
    `${x}.repo.listRecords?repo=${a}`,
    `${x}.repo.describeRepo`,
    `${x}.identity.resolveHandle`,
    // Formats.
    `${x}.sync.getLatestCommit?did=nope`,
    `${x}.sync.getLatestCommit?did=did:`,
    `${x}.sync.getLatestCommit?did=did:web:example.com`,
    `${x}.sync.getLatestCommit?did=${seed.accounts.alice.handle}`,
    `${x}.sync.getLatestCommit?did=${'x'.repeat(300)}`,
    `${x}.sync.getLatestCommit?did=did%3Aplc%3A${a.slice(8)}`,
    `${x}.sync.getRepo?did=${a}&since=nottid`,
    `${x}.sync.getRepo?did=${a}&since=`,
    `${x}.sync.getRepo?did=${a}&since=3aaaaaaaaaaa2&since=3aaaaaaaaaaa3`,
    `${x}.sync.getRecord?did=${a}&collection=bad&rkey=x`,
    `${x}.sync.getRecord?did=${a}&collection=${post}&rkey=a/b`,
    `${x}.sync.getRecord?did=${a}&collection=${post}&rkey=.`,
    `${x}.sync.getRecord?did=${a}&collection=${post}&rkey=a%20b`,
    `${x}.sync.getBlocks?did=${a}&cids=zzz`,
    `${x}.sync.getBlocks?did=${a}&cids=${cid}&cids=zzz`,
    `${x}.sync.getBlocks?did=${a}&cids=${cid.toUpperCase()}`,
    `${x}.sync.getBlob?did=${a}&cid=nope`,
    `${x}.sync.getBlob?did=${a}&cid=${cid.slice(0, -1)}`,
    `${x}.sync.getBlob?did=${a}&cid=QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG`,
    `${x}.repo.getRecord?repo=bad_id&collection=${post}&rkey=x`,
    `${x}.repo.getRecord?repo=${a}&collection=${post}&rkey=x&cid=zzz`,
    `${x}.repo.getRecord?repo=${a}&collection=${post}&rkey=x&cid=`,
    `${x}.repo.describeRepo?repo=bad_id`,
    `${x}.repo.describeRepo?repo=did:plc:short`,
    `${x}.identity.resolveHandle?handle=bad_handle`,
    `${x}.identity.resolveHandle?handle=nodots`,
    `${x}.identity.resolveHandle?handle=${a}`,
    // Integers and booleans.
    `${x}.sync.listRepos?limit=0`,
    `${x}.sync.listRepos?limit=1`,
    `${x}.sync.listRepos?limit=1000`,
    `${x}.sync.listRepos?limit=1001`,
    `${x}.sync.listRepos?limit=-1`,
    `${x}.sync.listRepos?limit=1.5`,
    `${x}.sync.listRepos?limit=1e1`,
    `${x}.sync.listRepos?limit=%2B5`,
    `${x}.sync.listRepos?limit=07`,
    `${x}.sync.listRepos?limit=abc`,
    `${x}.sync.listRepos?limit=`,
    `${x}.sync.listRepos?limit=2&limit=3`,
    `${x}.sync.listRepos?limit=99999999999999999999`,
    `${x}.sync.listRepos?limit=9007199254740993`,
    `${x}.sync.listRepos?limit=1000000000000000000000`,
    `${x}.sync.listRepos?limit=-99999999999999999999`,
    `${x}.sync.listRepos?limit=${'9'.repeat(400)}`,
    `${x}.sync.listRepos?cursor=`,
    `${x}.sync.listBlobs?did=${a}&limit=0`,
    `${x}.sync.listBlobs?did=${a}&limit=1001`,
    `${x}.repo.listRecords?repo=${a}&collection=${post}&limit=0`,
    `${x}.repo.listRecords?repo=${a}&collection=${post}&limit=100`,
    `${x}.repo.listRecords?repo=${a}&collection=${post}&limit=101`,
    `${x}.repo.listRecords?repo=${a}&collection=${post}&reverse=`,
    `${x}.repo.listRecords?repo=${a}&collection=${post}&reverse=false`,
    `${x}.repo.listRecords?repo=${a}&collection=${post}&reverse=1`,
    `${x}.repo.listRecords?repo=${a}&collection=${post}&reverse=TRUE`,
    `${x}.repo.listRecords?repo=${a}&collection=${post}&reverse=maybe`,
    `${x}.repo.listRecords?repo=${a}&collection=${post}&reverse=true&reverse=false`,
    `${x}.repo.listRecords?repo=${a}&collection=${post}&reverse=true&reverse=false&reverse=true`,
    `${x}.repo.listRecords?repo=${a}&collection=${post}&cursor=`,
    // Order: conversion first, then the lexicon's order.
    `${x}.repo.listRecords?limit=abc&collection=bad&repo=bad_id`,
    `${x}.repo.listRecords?reverse=x&limit=abc`,
    `${x}.repo.listRecords?repo=${a}&collection=&limit=abc`,
    `${x}.repo.listRecords?collection=bad&repo=bad_id`,
    `${x}.repo.listRecords?collection=bad&repo=${a}&limit=0`,
    `${x}.repo.getRecord?rkey=a/b&collection=bad&repo=bad_id`,
    `${x}.repo.getRecord?rkey=a/b&collection=bad&repo=${a}`,
    `${x}.repo.getRecord?rkey=a/b&collection=${post}&repo=${a}`,
  ]
  for (const path of cases) await raw(s, path.replace(`${x}.`, '').replaceAll(a, '<alice>'), path)
})
