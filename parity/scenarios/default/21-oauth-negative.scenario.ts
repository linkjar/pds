// OAuth, what must be refused: oracle O4's negative cases and SPEC 11.3.
// Duplicate code redemption, refresh reuse, a wrong DPoP key, PKCE and
// redirect mismatches, and VP-8 (the lifetime of a pushed request).

import assert from 'node:assert/strict'
import { launchBrowser, newDevice } from '../../src/lib/browser.ts'
import type { BrowserContext } from '../../src/lib/browser.ts'
import { makeClient } from '../../src/lib/oauth.ts'
import { generateDpopKey, pkce, resource, tokenEndpoint } from '../../src/lib/oauth-raw.ts'
import type { RawClient } from '../../src/lib/oauth-raw.ts'
import { driveAuthorization } from '../../src/oracles/oauth.ts'
import { refused, scenario } from '../../src/scenario.ts'
import type { Account, Scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const ISSUER = `https://${HOSTS.pds}`
const SCOPE = 'atproto transition:generic'

type Grant = { code: string; verifier: string; requestUri: string }

async function push(s: Scenario, step: string, client: RawClient, account: Account, extra: Record<string, string> = {}) {
  const { verifier, challenge } = pkce()
  const result = await tokenEndpoint(s, step, client, '/oauth/par', {
    response_type: 'code',
    code_challenge: challenge,
    code_challenge_method: 'S256',
    state: 'raw-state',
    redirect_uri: client.redirectUri,
    scope: SCOPE,
    login_hint: account.handle,
    ...extra,
  })
  return { result, verifier }
}

async function authorize(s: Scenario, device: BrowserContext, client: RawClient, account: Account, step: string): Promise<Grant> {
  const { result, verifier } = await push(s, `${step}: PAR`, client, account)
  assert.equal(result.status, 201, result.text)
  const url = `${ISSUER}/oauth/authorize?client_id=${encodeURIComponent(client.clientId)}&request_uri=${encodeURIComponent(result.json.request_uri)}`
  const page = await device.newPage()
  const { redirect } = await driveAuthorization(page, url, { redirectUri: client.redirectUri, password: account.password, account: account.handle })
  await page.close()
  const code = redirect?.searchParams.get('code')
  assert.ok(code, `no code: ${redirect}`)
  return { code, verifier, requestUri: result.json.request_uri }
}

const exchange = (s: Scenario, step: string, client: RawClient, grant: Grant, change: Record<string, string> = {}, opts = {}) =>
  tokenEndpoint(s, step, client, '/oauth/token', { grant_type: 'authorization_code', code: grant.code, code_verifier: grant.verifier, redirect_uri: client.redirectUri, ...change }, opts)

const refresh = (s: Scenario, step: string, client: RawClient, token: string, opts = {}) =>
  tokenEndpoint(s, step, client, '/oauth/token', { grant_type: 'refresh_token', refresh_token: token }, opts)

scenario('21-oauth-negative', { timeoutMs: 300_000 }, async (s) => {
  const alice = await s.createAccount('alice')
  const browser = await launchBrowser(s.target)
  s.onCleanup(() => browser.close())
  const published = await makeClient(s, 'raw', 'public')
  const newClient = async (): Promise<RawClient> => ({ clientId: published.clientId, redirectUri: published.redirectUri, key: await generateDpopKey(), nonce: {} })
  const device = () => newDevice(browser, { ip: s.ip })

  // VP-8: a pushed request lives five minutes.
  const client = await newClient()
  const pushed = await push(s, 'PAR', client, alice)
  assert.equal(pushed.result.status, 201)
  assert.match(pushed.result.json.request_uri, /^urn:ietf:params:oauth:request_uri:req-[0-9a-f]+$/)
  assert.ok(pushed.result.json.expires_in > 295 && pushed.result.json.expires_in <= 300, `VP-8: expires_in is ${pushed.result.json.expires_in}`)
  s.note('VP-8 PAR lifetime, minutes', Math.round(pushed.result.json.expires_in / 60))

  // What PAR refuses (SPEC 11.3).
  refused(await push(s, 'PAR without a code challenge', client, alice, { code_challenge: '' }).then((r) => r.result))
  refused(await push(s, 'PAR with the plain challenge method', client, alice, { code_challenge_method: 'plain' }).then((r) => r.result))
  refused(await push(s, 'PAR with a redirect URI the client did not register', client, alice, { redirect_uri: `https://${HOSTS.client}/elsewhere` }).then((r) => r.result))
  refused(await push(s, 'PAR with a scope the client did not declare', client, alice, { scope: 'atproto transition:chat.bsky' }).then((r) => r.result))
  refused(await push(s, 'PAR without the atproto scope', client, alice, { scope: 'transition:generic' }).then((r) => r.result))
  refused(await push(s, 'PAR with response_type token', client, alice, { response_type: 'token' }).then((r) => r.result))
  const unknownClient = { ...client, clientId: `https://${HOSTS.client}/no-such-client/client-metadata.json` }
  refused(await push(s, 'PAR from a client whose metadata does not exist', unknownClient, alice).then((r) => r.result))
  // Hostnames a client may not use (SPEC 11.2): a local top-level domain, and the documentation domains.
  refused(await push(s, 'PAR from a client under a local top-level domain', { ...client, clientId: 'https://client.test/client-metadata.json' }, alice).then((r) => r.result), 'invalid_client_id')
  refused(await push(s, 'PAR from a client under a documentation domain', { ...client, clientId: 'https://client.example.com/client-metadata.json' }, alice).then((r) => r.result), 'invalid_client_metadata')

  // PAR is mandatory: the authorization endpoint does not take the parameters directly.
  const direct = await s.http('authorize without a pushed request', {
    path: '/oauth/authorize',
    // A fixed challenge: the request is recorded, and a random one would differ on every run.
    query: { client_id: client.clientId, redirect_uri: client.redirectUri, response_type: 'code', scope: SCOPE, state: 'x', code_challenge: 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM', code_challenge_method: 'S256' },
  })
  s.note('authorize without a pushed request', { status: direct.status, location: direct.headers.get('location')?.replace(/state=[^&]+/, 'state=…') ?? null })
  assert.notEqual(direct.status, 200)

  // VP-8, the other half: after five minutes the request is dead. The harness ages the row.
  const requestId = pushed.result.json.request_uri.split(':').at(-1)
  const rows = await s.target.sqlite('account', "select (julianday(expiresAt) - julianday('now')) * 1440 as minutes from authorization_request where id = ?", [requestId])
  assert.ok(rows[0] && Math.abs(Number(rows[0].minutes) - 5) < 0.2, `VP-8: stored expiry is ${JSON.stringify(rows)}`)
  await s.target.sqlite('account', "update authorization_request set expiresAt = '2000-01-01T00:00:00.000Z' where id = ?", [requestId])
  const stalePage = await (await device()).newPage()
  const stale = await driveAuthorization(stalePage, `${ISSUER}/oauth/authorize?client_id=${encodeURIComponent(client.clientId)}&request_uri=${encodeURIComponent(pushed.result.json.request_uri)}`, { redirectUri: client.redirectUri, password: alice.password, allowError: true, timeoutMs: 8_000 })
  await stalePage.close()
  s.note('VP-8 authorize with an expired request', stale.redirect ? Object.fromEntries([...stale.redirect.searchParams].filter(([k]) => k !== 'state')) : { page: stale.facts.error ?? 'no redirect' })

  // A good exchange, for reference.
  const grant = await authorize(s, await device(), client, alice, 'first flow')
  const tokens = await exchange(s, 'token exchange', client, grant)
  assert.equal(tokens.status, 200, tokens.text)
  assert.equal(tokens.json.token_type, 'DPoP')
  assert.equal(tokens.json.sub, alice.did)
  assert.equal(tokens.json.scope, SCOPE)
  assert.ok(tokens.json.refresh_token)
  const ok = await resource(s, 'resource call with the bound key', client, tokens.json.access_token, { path: '/xrpc/com.atproto.server.getSession' })
  assert.equal(ok.status, 200)

  // A wrong DPoP key (O4).
  const otherKey = await generateDpopKey()
  const wrongKey = await resource(s, 'resource call with a proof from another key', client, tokens.json.access_token, { path: '/xrpc/com.atproto.server.getSession' }, { key: otherKey })
  assert.equal(wrongKey.status, 401)
  const asBearer = await resource(s, 'resource call presenting the token as Bearer', client, tokens.json.access_token, { path: '/xrpc/com.atproto.server.getSession' }, { scheme: 'Bearer' })
  refused(asBearer)
  const noProof = await s.http('resource call without a DPoP proof', { path: '/xrpc/com.atproto.server.getSession', headers: { authorization: `DPoP ${tokens.json.access_token}` } })
  refused(noProof)
  const wrongKeyRefresh = await refresh(s, 'refresh with a proof from another key', { ...client, nonce: { ...client.nonce } }, tokens.json.refresh_token, { key: otherKey })
  refused(wrongKeyRefresh)

  // Duplicate code redemption (O4): refused, and every token issued from the code is revoked.
  const replay = await exchange(s, 'the same code a second time', client, grant)
  refused(replay, 'invalid_grant')
  const afterReplay = await resource(s, 'resource call after the code was replayed', client, tokens.json.access_token, { path: '/xrpc/com.atproto.server.getSession' })
  const refreshAfterReplay = await refresh(s, 'refresh after the code was replayed', client, tokens.json.refresh_token)
  s.note('after a code replay', { accessToken: afterReplay.status, refresh: refreshAfterReplay.json?.error ?? refreshAfterReplay.status })
  refused(refreshAfterReplay, 'invalid_grant')

  // PKCE and redirect checks at the token endpoint.
  const second = await authorize(s, await device(), await newClient().then((c) => Object.assign(client, { key: c.key, nonce: {} })), alice, 'second flow')
  refused(await exchange(s, 'token exchange with a wrong code verifier', client, second, { code_verifier: pkce().verifier }), 'invalid_grant')
  const third = await authorize(s, await device(), client, alice, 'third flow')
  refused(await exchange(s, 'token exchange with another redirect URI', client, third, { redirect_uri: `https://${HOSTS.client}/elsewhere` }), 'invalid_grant')
  const fourth = await authorize(s, await device(), client, alice, 'fourth flow')
  refused(await exchange(s, 'token exchange with a proof from another key than the pushed request', client, fourth, {}, { key: otherKey }))
  refused(await tokenEndpoint(s, 'token exchange with a code that was never issued', client, '/oauth/token', { grant_type: 'authorization_code', code: 'cod-' + '0'.repeat(64), code_verifier: pkce().verifier, redirect_uri: client.redirectUri }), 'invalid_grant')
  refused(await tokenEndpoint(s, 'token request with an unsupported grant type', client, '/oauth/token', { grant_type: 'password', username: alice.handle, password: alice.password }))

  // Refresh reuse (O4): the old refresh token is refused and the session is gone.
  const fifth = await authorize(s, await device(), client, alice, 'fifth flow')
  const session = await exchange(s, 'token exchange for the refresh case', client, fifth)
  assert.equal(session.status, 200, session.text)
  const rotated = await refresh(s, 'refresh', client, session.json.refresh_token)
  assert.equal(rotated.status, 200, rotated.text)
  assert.notEqual(rotated.json.refresh_token, session.json.refresh_token)
  const reused = await refresh(s, 'the rotated refresh token again', client, session.json.refresh_token)
  refused(reused, 'invalid_grant')
  const successor = await refresh(s, 'the successor after the reuse', client, rotated.json.refresh_token)
  refused(successor, 'invalid_grant')
  const accessAfterReuse = await resource(s, 'resource call after refresh reuse', client, rotated.json.access_token, { path: '/xrpc/com.atproto.server.getSession' })
  s.note('access token after refresh reuse', accessAfterReuse.status)

  // Revocation (RFC 7009).
  const sixth = await authorize(s, await device(), client, alice, 'sixth flow')
  const toRevoke = await exchange(s, 'token exchange for the revocation case', client, sixth)
  const revoked = await tokenEndpoint(s, 'revoke the refresh token', client, '/oauth/revoke', { token: toRevoke.json.refresh_token })
  assert.equal(revoked.status, 200)
  refused(await refresh(s, 'refresh after revocation', client, toRevoke.json.refresh_token), 'invalid_grant')
  const accessAfterRevoke = await resource(s, 'resource call after revocation', client, toRevoke.json.access_token, { path: '/xrpc/com.atproto.server.getSession' })
  s.note('access token after revocation', accessAfterRevoke.status)
  const revokeUnknown = await tokenEndpoint(s, 'revoke a token nobody holds', client, '/oauth/revoke', { token: 'ref-' + '0'.repeat(64) })
  assert.equal(revokeUnknown.status, 200, 'revocation of an unknown token succeeds (RFC 7009)')
})
