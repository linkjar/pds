// The `app.bsky` methods the Reference serves itself (SPEC 4.5, 6.5):
// preferences from the actor store, and reads that overlay records the
// AppView has not indexed yet.

import assert from 'node:assert/strict'
import { bearer, verifyServiceToken } from '../../src/oracles/service-auth.ts'
import { refused, scenario } from '../../src/scenario.ts'
import { HOSTS } from '../../src/stack/targets.ts'

const POST = 'app.bsky.feed.post'

scenario('14-appview-local', {}, async (s) => {
  const alice = await s.createAccount('alice')

  // Preferences live in the actor store.
  const empty = await s.query('getPreferences, none stored', 'app.bsky.actor.getPreferences', {}, { auth: alice })
  assert.deepEqual(empty.json, { preferences: [] })
  const preferences = [
    { $type: 'app.bsky.actor.defs#adultContentPref', enabled: false },
    { $type: 'app.bsky.actor.defs#savedFeedsPrefV2', items: [{ id: '3aaaaaaaaaaa2', type: 'timeline', value: 'following', pinned: true }] },
  ]
  const put = await s.procedure('putPreferences', 'app.bsky.actor.putPreferences', { preferences }, { auth: alice })
  assert.equal(put.status, 200)
  const stored = await s.query('getPreferences', 'app.bsky.actor.getPreferences', {}, { auth: alice })
  assert.deepEqual(stored.json.preferences, preferences)
  refused(await s.query('getPreferences without a credential', 'app.bsky.actor.getPreferences'), 'AuthMissing')
  refused(await s.procedure('putPreferences with a preference outside app.bsky', 'app.bsky.actor.putPreferences', { preferences: [{ $type: 'com.example.pref', on: true }] }, { auth: alice }))
  refused(await s.procedure('putPreferences without $type', 'app.bsky.actor.putPreferences', { preferences: [{ enabled: true }] }, { auth: alice }))

  // An app password without privilege does not see the personal-details preference (SPEC 6.5).
  await s.procedure('putPreferences with personal details', 'app.bsky.actor.putPreferences', { preferences: [...preferences, { $type: 'app.bsky.actor.defs#personalDetailsPref', birthDate: '1990-01-01T00:00:00.000Z' }] }, { auth: alice })
  const appPassword = await s.procedure('createAppPassword', 'com.atproto.server.createAppPassword', { name: 'prefs' }, { auth: alice, silent: true })
  const appSession = await s.procedure('createSession with the app password', 'com.atproto.server.createSession', { identifier: alice.handle, password: appPassword.json.password }, { silent: true })
  const limited = await s.query('getPreferences from an app-password session', 'app.bsky.actor.getPreferences', {}, { auth: appSession.json.accessJwt })
  const types = (result: { json: { preferences: { $type: string }[] } }) => result.json.preferences.map((p) => p.$type)
  const full = await s.query('getPreferences from the full session', 'app.bsky.actor.getPreferences', {}, { auth: alice })
  s.note('preference types the full session reads', types(full))
  s.note('preference types an app password reads', types(limited))
  assert.ok(types(full).includes('app.bsky.actor.defs#personalDetailsPref'))
  assert.ok(!types(limited).includes('app.bsky.actor.defs#personalDetailsPref'))
  // The Reference derives a declared-age preference from the birth date and shows it to both.
  assert.ok(types(full).includes('app.bsky.actor.defs#declaredAgePref'))

  // Read-after-write (SPEC 4.5). The AppView answers with the revision it has
  // indexed; the server overlays what the account wrote after it.
  // The overlay applies only when the AppView's revision is one this
  // repository has a record at or before, so the account needs an indexed post.
  const indexedPost = await s.procedure('write a post the AppView has indexed', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa7', record: { $type: POST, text: 'indexed', createdAt: '2026-01-01T00:00:00.000Z' } }, { auth: alice, silent: true })
  const before = { json: { rev: indexedPost.json.commit.rev } }
  const profile = await s.procedure('write a profile', 'com.atproto.repo.putRecord', { repo: alice.did, collection: 'app.bsky.actor.profile', rkey: 'self', record: { $type: 'app.bsky.actor.profile', displayName: 'Alice, just now' } }, { auth: alice, silent: true })
  const posted = await s.procedure('write a post', 'com.atproto.repo.createRecord', { repo: alice.did, collection: POST, rkey: '3aaaaaaaaaaa2', record: { $type: POST, text: 'not indexed yet', createdAt: '2026-01-01T00:00:00.000Z' } }, { auth: alice, silent: true })
  const stale = { 'atproto-repo-rev': before.json.rev }
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: 'app.bsky.actor.getProfile', headers: stale, body: { did: alice.did, handle: alice.handle, displayName: 'Alice, as indexed' } })
  // The feed overlay recognises the account's own feed by its first item.
  const indexedView = {
    uri: indexedPost.json.uri,
    cid: indexedPost.json.cid,
    author: { did: alice.did, handle: alice.handle },
    record: { $type: POST, text: 'indexed', createdAt: '2026-01-01T00:00:00.000Z' },
    indexedAt: '2026-01-01T00:00:00.000Z',
  }
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: 'app.bsky.feed.getAuthorFeed', headers: stale, body: { feed: [{ post: indexedView }] } })

  let mark = await s.fixtures.cursor()
  const ownProfile = await s.query('getProfile of the account itself', 'app.bsky.actor.getProfile', { actor: alice.did }, { auth: alice })
  assert.equal(ownProfile.status, 200)
  assert.equal(ownProfile.json.displayName, 'Alice, just now', 'the local record overlays the indexed one')
  // At the pin an overlaid response says how far behind the AppView was, in
  // milliseconds, and does not carry the AppView's `Atproto-Repo-Rev`.
  assert.match(ownProfile.headers.get('atproto-upstream-lag') ?? '', /^\d+$/)
  assert.equal(ownProfile.headers.get('atproto-repo-rev'), null)
  const [call] = await s.fixtures.calls(HOSTS.appview, mark)
  const token = await verifyServiceToken(s, bearer(call!.headers), alice.did)
  assert.equal(token.claims.lxm, 'app.bsky.actor.getProfile')

  const feed = await s.query('getAuthorFeed of the account itself', 'app.bsky.feed.getAuthorFeed', { actor: alice.did }, { auth: alice })
  assert.equal(feed.status, 200)
  assert.equal(feed.json.feed.length, 2, 'the post the AppView has not indexed is in the feed')
  assert.equal(feed.json.feed[0].post.record.text, 'not indexed yet', 'newest first')
  assert.equal(feed.json.feed[0].post.uri, posted.json.uri)
  assert.equal(feed.json.feed[0].post.author.displayName, 'Alice, just now', 'with the local profile')
  assert.match(feed.headers.get('atproto-upstream-lag') ?? '', /^\d+$/)

  // When the AppView is level with the repository, its answer passes through.
  const level = { 'atproto-repo-rev': posted.json.commit.rev }
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: 'app.bsky.actor.getProfile', headers: level, body: { did: alice.did, handle: alice.handle, displayName: 'Alice, as indexed' } })
  const indexed = await s.query('getProfile once the AppView has caught up', 'app.bsky.actor.getProfile', { actor: alice.did }, { auth: alice })
  assert.equal(indexed.json.displayName, 'Alice, as indexed')
  assert.equal(indexed.headers.get('atproto-repo-rev'), posted.json.commit.rev, "the AppView's revision header passes through")
  assert.equal(indexed.headers.get('atproto-upstream-lag'), null)
  // Another actor's profile is never overlaid.
  await s.fixtures.xrpc({ host: HOSTS.appview, nsid: 'app.bsky.actor.getProfile', headers: stale, body: { did: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa', handle: 'other.test', displayName: 'Someone else' } })
  const other = await s.query('getProfile of another actor', 'app.bsky.actor.getProfile', { actor: 'did:plc:aaaaaaaaaaaaaaaaaaaaaaaa' }, { auth: alice })
  assert.equal(other.json.displayName, 'Someone else')
  assert.ok(profile.json.commit.rev < posted.json.commit.rev)

  // Push registration goes to the notification service named in the request.
  mark = await s.fixtures.cursor()
  const push = await s.procedure('registerPush', 'app.bsky.notification.registerPush', { serviceDid: `did:web:${HOSTS.appview}`, token: 'device-token', platform: 'ios', appId: 'io.linkjar.app' }, { auth: alice })
  assert.equal(push.status, 200)
  const [pushCall] = await s.fixtures.calls(HOSTS.appview, mark)
  assert.equal(pushCall?.path, '/xrpc/app.bsky.notification.registerPush')
  const pushToken = await verifyServiceToken(s, bearer(pushCall!.headers), alice.did)
  assert.equal(pushToken.claims.aud, `did:web:${HOSTS.appview}`)
  s.note('registerPush forwarded body', JSON.parse(pushCall!.body))
})
