// The hosted handle policy (patch 093, SPEC 12.4 and 13): the extra
// reservations, the single hosted rename of a freshly signed-up account, and
// what a handle hostname serves. Port of legacy/tests/handles.mjs and of the
// Caddy integration in handle-proxy-probes.mjs: the edge of the stack runs
// the production snippet, legacy/staging/Caddyfile.handles.

import assert from 'node:assert/strict'
import { launchBrowser, newDevice } from '../../src/lib/browser.ts'
import { makeExtensionClient } from '../../src/lib/oauth.ts'
import { externalSignIn } from '../../src/oracles/linkjar.ts'
import { viaSession } from '../../src/oracles/oauth.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

scenario('31-linkjar-handles', { linkjar: true, timeoutMs: 300_000 }, async (s) => {
  // Reservations on top of the upstream list: infrastructure and brand names,
  // numbered and hyphenated variants, and digit lookalikes.
  const attempt = (label: string) =>
    s.procedure(`createAccount as ${label}`, 'com.atproto.server.createAccount', { handle: `${label}${HOSTS.handleDomain}`, email: s.email(`r${label.replace(/[^a-z0-9]/g, '')}`), password: 'reserved-check-1' })
  for (const label of ['linkjar', 'pds', 'api', 'support', 'status', 'pds2', 'api-prod', 'link-jar', '1inkjar', 'l1nkj4r', 'supp0rt']) {
    refused(await attempt(label), 'HandleNotAvailable')
  }
  // A name that merely contains a service word is free.
  for (const label of [`jared-${s.tag}`, `olivia-${s.tag}`]) assert.equal((await attempt(label)).status, 200, label)

  // A password account keeps the upstream behaviour: it may rename as often as upstream allows.
  const local = await s.createAccount('local')
  for (const name of ['local2', 'local3']) {
    const renamed = await s.procedure(`password account renames to ${name}`, 'com.atproto.identity.updateHandle', { handle: s.handle(name) }, { auth: local })
    assert.equal(renamed.status, 200)
  }

  // A freshly signed-up provider account may choose one new hosted handle within 30 days.
  const browser = await launchBrowser(s.target)
  s.onCleanup(() => browser.close())
  const app = await makeExtensionClient(s)
  const fresh = await externalSignIn(s, await newDevice(browser, { ip: s.ip }), app, { provider: 'google', subject: `google-${s.tag}-h1`, email: s.email('fresh'), emailVerified: true, name: `Fresh ${s.tag}` })
  assert.ok(fresh.session)
  const rename = (step: string, handle: string) => viaSession(s, step, fresh.session!, '/xrpc/com.atproto.identity.updateHandle', { json: { handle } })
  const same = await rename('rename to the handle it already has', `fresh-${s.tag}${HOSTS.handleDomain}`)
  assert.equal(same.status, 200, 'an unchanged handle is idempotent and spends nothing')
  const first = await rename('the one hosted rename', s.handle('chosen'))
  assert.equal(first.status, 200)
  assert.equal((await viaSession(s, 'getSession after the rename', fresh.session, '/xrpc/com.atproto.server.getSession')).json.handle, s.handle('chosen'))
  const retry = await rename('the same target again', s.handle('chosen'))
  assert.equal(retry.status, 200, 'the chosen target stays available')
  const second = await rename('a second hosted rename', s.handle('another'))
  refused(second)
  s.note('second hosted rename', { status: second.status, error: second.json?.error, message: second.json?.message })
  // A custom domain does not consume the allowance and keeps upstream verification.
  await s.fixtures.serve({ host: HOSTS.customHandle, path: '/.well-known/atproto-did', contentType: 'text/plain', body: fresh.did! })
  const custom = await rename('rename to a custom domain', HOSTS.customHandle)
  assert.equal(custom.status, 200)
  const back = await rename('back to a hosted handle after the custom domain', s.handle('chosen'))
  s.note('hosted handle after a custom domain', { status: back.status, error: back.json?.error })

  // What a handle hostname serves, through the production Caddy snippet.
  const host = s.handle('local3')
  const did = await s.http('handle host: atproto-did', { host, path: '/.well-known/atproto-did', headers: { cookie: 'session=should-not-reach-the-server' } })
  assert.equal(did.status, 200)
  assert.equal(did.text, local.did)
  assert.equal(did.headers.get('content-security-policy'), "default-src 'none'; sandbox")
  assert.equal(did.headers.get('x-content-type-options'), 'nosniff')
  assert.equal(did.headers.get('referrer-policy'), 'no-referrer')
  assert.equal(did.headers.get('set-cookie'), null)
  for (const path of ['/.well-known/oauth-authorization-server', '/.well-known/oauth-protected-resource', '/xrpc/com.atproto.server.describeServer', '/xrpc/_health', `/xrpc/com.atproto.sync.getRepo?did=${local.did}`]) {
    const blocked = await s.http(`handle host: ${path.split('?')[0]}`, { host, path })
    assert.equal(blocked.status, 404, path)
    assert.match(blocked.text, /Use the PDS service endpoint/)
  }
  for (const [method, path] of [['GET', '/'], ['GET', '/anything/else?x=1'], ['HEAD', '/'], ['GET', '/oauth/authorize'], ['GET', '/account']] as const) {
    const redirected = await s.http(`handle host: ${method} ${path.split('?')[0]}`, { host, path, method })
    assert.equal(redirected.status, 302, path)
    assert.equal(redirected.headers.get('location'), `https://${HOSTS.web}/jar/${encodeURIComponent(local.did)}`)
    assert.equal(redirected.headers.get('set-cookie'), null)
  }
  const post = await s.http('handle host: POST', { host, path: '/', method: 'POST', body: 'x', contentType: 'text/plain' })
  s.note('POST to a handle host', post.status)
  // A hostname nobody holds, and a reserved one.
  refused(await s.http('unclaimed handle host: atproto-did', { host: s.handle('nobody'), path: '/.well-known/atproto-did' }))
  const unclaimed = await s.http('unclaimed handle host: front page', { host: s.handle('nobody'), path: '/' })
  s.note('front page of an unclaimed handle host', { status: unclaimed.status, location: unclaimed.headers.get('location') })
  const reserved = await s.http('reserved handle host: front page', { host: `support${HOSTS.handleDomain}`, path: '/' })
  s.note('front page of a reserved handle host', { status: reserved.status, location: reserved.headers.get('location') })
  // The service host itself is not a handle host.
  const service = await s.http('service host: front page', { path: '/' })
  assert.equal(service.status, 200)
  const profile = await s.http('service host: linkjar-profile', { path: '/.well-known/linkjar-profile' })
  s.note('linkjar-profile on the service host', profile.status)
})
