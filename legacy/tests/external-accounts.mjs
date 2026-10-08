import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
const pds = pathToFileURL(
  process.env.PDS_PACKAGE_PATH || "/app/packages/pds",
).href;
const { AccountManager } = await import(
  `${pds}/dist/account-manager/account-manager.js`
);
const { OAuthStore } = await import(
  `${pds}/dist/account-manager/oauth-store.js`
);
const { getDb, getMigrator } = await import(
  `${pds}/dist/account-manager/db/index.js`
);
const { verifyAccountPassword, updateUserPassword } = await import(
  `${pds}/dist/account-manager/helpers/password.js`
);
const { genSaltAndHash } = await import(
  `${pds}/dist/account-manager/helpers/scrypt.js`
);
const { deleteAccount } = await import(
  `${pds}/dist/account-manager/helpers/account.js`
);
const db = getDb(":memory:");
await getMigrator(db).migrateToLatestOrThrow();
const manager = Object.assign(Object.create(AccountManager.prototype), { db });
const store = new OAuthStore(manager);
const identity = {
  provider: "google",
  subject: "stable-123",
  email: "alice@example.com",
  emailVerified: true,
  displayName: "Alice",
};
const did = "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa";
async function create(accountDid, email, subject, extra = {}) {
  return manager.createAccount({
    did: accountDid,
    handle: `${accountDid.slice(-4)}.linkjar.social`,
    email,
    repoCid: { toString: () => "bafyreieq" },
    repoRev: "rev",
    externalIdentity: { ...identity, email, subject },
    linkjarAutoHandle: true,
    ...extra,
  });
}
try {
  await create(did, identity.email, identity.subject);
  assert.equal(
    await store.findExternalIdentity("google", identity.subject),
    did,
  );
  assert.equal(
    await store.findExternalIdentity("apple", identity.subject),
    null,
  );
  assert.equal(
    await store.findExternalIdentity("google", identity.email),
    null,
  );
  assert.equal(
    (await store.listExternalIdentities(did))[0].emailVerified,
    true,
  );
  assert.ok(
    (
      await db.db
        .selectFrom("account")
        .selectAll()
        .where("did", "=", did)
        .executeTakeFirst()
    ).emailConfirmedAt,
  );
  assert.equal(
    await verifyAccountPassword(db, did, "!external-identity"),
    false,
  );
  assert.equal(await verifyAccountPassword(db, did, "guess"), false);
  console.log(
    "PASS identity subject isolation, verified email, disabled password",
  );

  const duplicateDid = "did:plc:bbbbbbbbbbbbbbbbbbbbbbbb";
  await assert.rejects(() =>
    create(duplicateDid, "different@example.com", identity.subject),
  );
  assert.equal(
    await db.db
      .selectFrom("actor")
      .select("did")
      .where("did", "=", duplicateDid)
      .executeTakeFirst(),
    undefined,
  );
  assert.equal(
    await db.db
      .selectFrom("account")
      .select("did")
      .where("did", "=", duplicateDid)
      .executeTakeFirst(),
    undefined,
  );
  assert.equal(
    await db.db
      .selectFrom("linkjar_handle_policy")
      .select("did")
      .where("did", "=", duplicateDid)
      .executeTakeFirst(),
    undefined,
  );
  assert.equal(
    await store.findExternalIdentity("google", identity.subject),
    did,
  );
  console.log(
    "PASS duplicate provider identity rolls back actor, credentials, and handle policy",
  );

  await assert.rejects(() =>
    create(duplicateDid, identity.email, "different-subject"),
  );
  assert.equal(
    await store.findExternalIdentity("google", "different-subject"),
    null,
  );
  await assert.rejects(() =>
    create(duplicateDid, "bob@example.com", "bob", {
      externalIdentity: {
        ...identity,
        email: "bob@example.com",
        emailVerified: false,
      },
    }),
  );
  console.log(
    "PASS email collisions never auto-link and unverified signup is refused",
  );

  await updateUserPassword(db, {
    did,
    passwordScrypt: await genSaltAndHash("a new password"),
  });
  assert.equal(await verifyAccountPassword(db, did, "a new password"), true);
  assert.equal(await verifyAccountPassword(db, did, "guess"), false);
  console.log(
    "PASS existing password-setting primitive enables password login",
  );

  await store.linkExternalIdentity(did, {
    ...identity,
    provider: "apple",
    subject: "apple-123",
  });
  assert.equal((await store.listExternalIdentities(did)).length, 2);
  await store.unlinkExternalIdentity(did, "apple", "apple-123", ["apple", "google", "github"]);
  assert.equal(await store.findExternalIdentity("apple", "apple-123"), null);
  await deleteAccount(db, did);
  assert.equal(
    await store.findExternalIdentity("google", identity.subject),
    null,
  );
  console.log(
    "PASS link, unlink, and account deletion remove identity records",
  );
} finally {
  db.close();
}
