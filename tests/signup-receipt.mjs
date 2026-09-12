import assert from "node:assert/strict";
import {
  createHash,
  createSecretKey,
  randomBytes,
  randomUUID,
} from "node:crypto";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

const root = pathToFileURL(process.env.UPSTREAM_DIR || "/app").href;
const pds = `${root}/packages/pds`;
const provider = `${root}/packages/oauth/oauth-provider`;
const requirePds = createRequire(`${pds}/package.json`);
const jose = await import(requirePds.resolve("jose"));
const { JoseKey } = await import(
  `${root}/packages/oauth/jwk-jose/dist/index.js`
);
const { AccountManager } = await import(
  `${pds}/dist/account-manager/account-manager.js`
);
const { OAuthStore } = await import(
  `${pds}/dist/account-manager/oauth-store.js`
);
const { getDb, getMigrator } = await import(
  `${pds}/dist/account-manager/db/index.js`
);
const { AuthVerifier } = await import(`${pds}/dist/auth-verifier.js`);
const { OAuthVerifier } = await import(`${provider}/dist/oauth-verifier.js`);
const { AccountManager: OAuthAccountManager } = await import(
  `${provider}/dist/account/account-manager.js`
);
const { OAuthProvider } = await import(`${provider}/dist/oauth-provider.js`);
const { TokenManager, Signer } = await import(
  `${provider}/dist/token/token-manager.js`
);
const { RequestManager } = await import(
  `${provider}/dist/request/request-manager.js`
);
const { generateCode } = await import(`${provider}/dist/request/code.js`);
const { generateRequestId } = await import(
  `${provider}/dist/request/request-id.js`
);
const { Keyset } = await import(`${provider}/dist/signer/signer.js`);
const { default: getReceipt } = await import(
  `${pds}/dist/api/io/linkjar/account/getSignupReceipt.js`
);
const { default: acknowledgeReceipt } = await import(
  `${pds}/dist/api/io/linkjar/account/acknowledgeSignupReceipt.js`
);
const db = getDb(":memory:");
await getMigrator(db).migrateToLatestOrThrow();
const issuer = "https://pds.linkjar.test";
const audience = "did:web:pds.linkjar.test";
const manager = Object.assign(Object.create(AccountManager.prototype), {
  db,
  cfg: { service: { did: audience } },
});
const store = new OAuthStore(manager, { read: async () => null }, undefined, {
  add: () => {},
});
const keyset = new Keyset([await JoseKey.generate(["ES256"])]);
const signer = new Signer(issuer, keyset);
const lexicons = { buildTokenScope: async (scope) => scope };
const tokens = new TokenManager(store, lexicons, signer, {}, "stateless");
const requests = new RequestManager(store, lexicons, signer, {}, {});
const verifier = new OAuthVerifier({ issuer, keyset, dpopSecret: false });
const authVerifier = new AuthVerifier(manager, {}, verifier, {
  publicUrl: issuer,
  jwtKey: createSecretKey(randomBytes(32)),
  adminPass: "fixture-only",
  dids: { pds: audience },
});
const routes = new Map();
const server = { add: (schema, route) => routes.set(schema.$lxm, route) };
getReceipt(server, { accountManager: manager, authVerifier });
acknowledgeReceipt(server, { accountManager: manager, authVerifier });
const grantProvider = Object.assign(Object.create(OAuthProvider.prototype), {
  tokenManager: tokens,
  requestManager: requests,
  accountManager: new OAuthAccountManager(
    issuer,
    store,
    {},
    { inviteCodeRequired: false },
  ),
  replayManager: verifier.replayManager,
});
const deviceId = "dev-fixture";
const client = {
  id: "https://linkjar.test/client-metadata.json",
  metadata: {
    grant_types: ["authorization_code", "refresh_token"],
    dpop_bound_access_tokens: true,
    token_endpoint_auth_method: "none",
  },
  sessionLifetime: 60 * 60 * 1000,
};
const clientAuth = { method: "none" };
const clientMetadata = { ipAddress: "127.0.0.1", port: 443 };
const dpopKey = await jose.generateKeyPair("ES256");
const dpopPublic = await jose.exportJWK(dpopKey.publicKey);
const jkt = await jose.calculateJwkThumbprint(dpopPublic);
let accountSequence = 0;
function accountData(extra = {}) {
  const suffix = (++accountSequence).toString().padStart(24, "a");
  return {
    did: `did:plc:${suffix}`,
    handle: `person${accountSequence}.linkjar.social`,
    email: `person${accountSequence}@example.com`,
    password: "fixture password",
    repoCid: { toString: () => "bafyreieq" },
    repoRev: "rev",
    ...extra,
  };
}
async function request(overrides = {}) {
  const requestId = await generateRequestId();
  const verifier = randomBytes(32).toString("base64url");
  const parameters = {
    scope: "atproto",
    response_type: "code",
    redirect_uri: "https://linkjar.test/callback",
    dpop_jkt: jkt,
    code_challenge_method: "S256",
    code_challenge: createHash("sha256").update(verifier).digest("base64url"),
  };
  await store.createRequest(requestId, {
    clientId: client.id,
    clientAuth,
    parameters,
    expiresAt: new Date(Date.now() + 60_000),
    deviceId,
    did: null,
    code: null,
    ...overrides,
  });
  return { requestId, deviceId, clientId: client.id, verifier, parameters };
}
async function authorize(context, did, grantClient = client) {
  const code = await generateCode();
  await store.updateRequest(context.requestId, { did, code });
  return grantProvider.authorizationCodeGrant(
    grantClient,
    clientAuth,
    clientMetadata,
    {
      grant_type: "authorization_code",
      code,
      redirect_uri: context.parameters.redirect_uri,
      code_verifier: context.verifier,
    },
    { jkt },
  );
}
async function proof(token, method, url, key = dpopKey) {
  return new jose.SignJWT({
    htm: method,
    htu: url,
    ath: createHash("sha256").update(token).digest("base64url"),
  })
    .setProtectedHeader({
      alg: "ES256",
      typ: "dpop+jwt",
      jwk: await jose.exportJWK(key.publicKey),
    })
    .setJti(randomUUID())
    .setIssuedAt()
    .sign(key.privateKey);
}
async function call(token, method = "getSignupReceipt", options = {}) {
  const nsid = `io.linkjar.account.${method}`;
  const route = routes.get(nsid);
  assert.ok(route, "Generated lexicon registered");
  const httpMethod = method === "getSignupReceipt" ? "GET" : "POST";
  const url = `${issuer}/xrpc/${nsid}`;
  const headers = new Map();
  const res = {
    setHeader: (k, v) => headers.set(k, v),
    getHeader: (k) => headers.get(k),
    appendHeader: (k, v) => headers.set(k, v),
  };
  const req = {
    method: httpMethod,
    url: new URL(url).pathname,
    headers: {
      authorization: `${options.bearer ? "Bearer" : "DPoP"} ${token}`,
      ...(options.noProof
        ? {}
        : { dpop: await proof(token, httpMethod, url, options.key) }),
    },
  };
  const auth = await route.auth({ req, res, params: {} });
  const result = await route.handler({
    auth,
    req,
    res,
    params: {},
    input: { body: {} },
  });
  assert.equal(headers.get("Cache-Control"), "no-store");
  return result.body;
}
try {
  for (const fault of ["device", "client", "expired", "consumed", "deleted"]) {
    const context = await request(
      fault === "expired" ? { expiresAt: new Date(0) } : {},
    );
    if (fault === "consumed")
      await store.updateRequest(context.requestId, {
        code: await generateCode(),
        did: "did:plc:existing",
      });
    if (fault === "deleted") await store.deleteRequest(context.requestId);
    const authorization = {
      ...context,
      ...(fault === "device" ? { deviceId: "wrong" } : {}),
      ...(fault === "client" ? { clientId: "https://other.test" } : {}),
    };
    const account = accountData({
      signupAuthorization: authorization,
      password: undefined,
      linkjarAutoHandle: true,
    });
    account.externalIdentity = {
      provider: "google",
      subject: account.did,
      email: account.email,
      emailVerified: true,
    };
    await assert.rejects(
      () => manager.createAccount(account),
      /authorization is no longer available/,
    );
    for (const table of [
      "actor",
      "account",
      "external_identity",
      "linkjar_handle_policy",
    ]) {
      assert.equal(
        await db.db
          .selectFrom(table)
          .select("did")
          .where("did", "=", account.did)
          .executeTakeFirst(),
        undefined,
        `${fault} rolls back ${table}`,
      );
    }
  }
  console.log(
    "PASS invalid/expired/consumed/deleted signup bindings roll back account, identity, and handle policy",
  );

  const context = await request();
  const fresh = accountData({ signupAuthorization: context });
  await manager.createAccount(fresh);
  assert.equal(
    (await store.readRequest(context.requestId)).signupDid,
    fresh.did,
  );
  await assert.rejects(
    () => manager.createAccount(accountData({ signupAuthorization: context })),
    /authorization is no longer available/,
  );
  const session = await authorize(context, fresh.did);
  assert.deepEqual(await call(session.access_token), {
    created: true,
    did: fresh.did,
    handle: fresh.handle,
  });
  assert.equal(
    (await call(session.access_token)).created,
    true,
    "GET is non-consuming",
  );
  console.log(
    "PASS email account creation follows its consumed code into a signed DPoP session",
  );

  const existingContext = await request({ signupDid: fresh.did });
  assert.equal(
    (await store.readRequest(existingContext.requestId)).signupDid,
    null,
    "request creation cannot initialize a receipt",
  );
  // Use a fresh PKCE challenge even when the requested prompt is create.
  await store.updateRequest(existingContext.requestId, {
    parameters: { ...existingContext.parameters, prompt: "create" },
  });
  const returning = await authorize(existingContext, fresh.did);
  assert.equal((await call(returning.access_token)).created, false);
  assert.equal((await call(session.access_token)).created, true);
  const selectedContext = await request();
  const unselected = accountData({ signupAuthorization: selectedContext });
  await manager.createAccount(unselected);
  const selected = await authorize(selectedContext, fresh.did);
  assert.equal((await call(selected.access_token)).created, false);
  console.log(
    "PASS returning login, prompt=create, and a different selected account cannot acquire another receipt",
  );

  const wrongClientContext = await request();
  const wrongClientAccount = accountData({
    signupAuthorization: wrongClientContext,
  });
  await manager.createAccount(wrongClientAccount);
  await assert.rejects(
    () =>
      authorize(wrongClientContext, wrongClientAccount.did, {
        ...client,
        id: "https://different.test",
      }),
    /not issued to this client/,
  );
  assert.equal(await store.readRequest(wrongClientContext.requestId), null);
  await assert.rejects(() =>
    call(session.access_token, "getSignupReceipt", { noProof: true }),
  );
  await assert.rejects(async () =>
    call(session.access_token, "getSignupReceipt", {
      key: await jose.generateKeyPair("ES256"),
    }),
  );
  await assert.rejects(() =>
    call(session.access_token, "getSignupReceipt", { bearer: true }),
  );
  const signed = await signer.verifyAccessToken(session.access_token);
  const forgedClient = await signer.createAccessToken({
    ...signed.payload,
    client_id: "https://different.test",
    iss: undefined,
  });
  assert.equal((await call(forgedClient)).created, false);
  const tampered = session.access_token.slice(0, -5) + "xxxxx";
  await assert.rejects(() => call(tampered));
  console.log(
    "PASS token issuance client binding, verified signature, DPoP key binding, and persisted-session client isolation",
  );

  let current = await tokens.rotateToken(
    client,
    clientAuth,
    clientMetadata,
    await tokens.findByAccessToken(session.access_token),
  );
  assert.equal((await call(current.access_token)).created, true);
  assert.equal(
    (await call(session.access_token)).created,
    false,
    "rotated token id no longer finds receipt",
  );
  await call(current.access_token, "acknowledgeSignupReceipt");
  await call(current.access_token, "acknowledgeSignupReceipt");
  assert.equal((await call(current.access_token)).created, false);
  current = await tokens.rotateToken(
    client,
    clientAuth,
    clientMetadata,
    await tokens.findByAccessToken(current.access_token),
  );
  assert.equal((await call(current.access_token)).created, false);
  console.log(
    "PASS refresh preserves pending receipt, acknowledgment is idempotent, refresh cannot resurrect it",
  );

  const externalContext = await request();
  const external = accountData({
    signupAuthorization: externalContext,
    password: undefined,
    linkjarAutoHandle: true,
  });
  external.externalIdentity = {
    provider: "apple",
    subject: external.did,
    email: external.email,
    emailVerified: true,
  };
  await manager.createAccount(external);
  const externalSession = await authorize(externalContext, external.did);
  assert.equal((await call(externalSession.access_token)).created, true);
  const sameAccountOtherSession = await authorize(
    await request(),
    external.did,
  );
  await call(sameAccountOtherSession.access_token, "acknowledgeSignupReceipt");
  assert.equal((await call(externalSession.access_token)).created, true);
  await store.deleteToken(
    (await signer.verifyAccessToken(externalSession.access_token)).payload.jti,
  );
  assert.equal((await call(externalSession.access_token)).created, false);
  console.log(
    "PASS provider creation receipt, another-session acknowledgment isolation, and session revocation",
  );

  const raceContext = await request();
  const competitors = [
    accountData({ signupAuthorization: raceContext }),
    accountData({ signupAuthorization: raceContext }),
  ];
  const outcomes = await Promise.allSettled(
    competitors.map((account) => manager.createAccount(account)),
  );
  assert.equal(
    outcomes.filter((outcome) => outcome.status === "fulfilled").length,
    1,
  );
  const winner =
    competitors[
      outcomes.findIndex((outcome) => outcome.status === "fulfilled")
    ];
  const loser =
    competitors[outcomes.findIndex((outcome) => outcome.status === "rejected")];
  assert.equal(await manager.getAccount(loser.did), null);
  const raceSession = await authorize(raceContext, winner.did);
  const raceTokenInfo = await tokens.findByAccessToken(
    raceSession.access_token,
  );
  assert.equal((await call(raceSession.access_token)).created, true);
  await assert.rejects(() =>
    grantProvider.authorizationCodeGrant(
      client,
      clientAuth,
      clientMetadata,
      {
        grant_type: "authorization_code",
        code: raceTokenInfo.data.code,
        redirect_uri: raceContext.parameters.redirect_uri,
        code_verifier: raceContext.verifier,
      },
      { jkt },
    ),
  );
  assert.equal((await call(raceSession.access_token)).created, false);
  console.log(
    "PASS concurrent signups produce one account and receipt; authorization code replay revokes it",
  );
} finally {
  db.close();
}
