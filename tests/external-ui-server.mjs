import assert from "node:assert/strict";
import { createServer } from "node:http";
import { once } from "node:events";
import { pathToFileURL } from "node:url";
const upstream = pathToFileURL(
  process.env.UPSTREAM_DIR || `${process.cwd()}/.upstream`,
).href;
const { sendAuthorizePageFactory } = await import(
  `${upstream}/packages/oauth/oauth-provider/dist/router/assets/send-authorization-page.js`
);
const { sendAccountPageFactory } = await import(
  `${upstream}/packages/oauth/oauth-provider/dist/router/assets/send-account-page.js`
);
const { createApiMiddleware } = await import(
  `${upstream}/packages/oauth/oauth-provider/dist/router/create-api-middleware.js`
);
const { assetsMiddleware } = await import(
  `${upstream}/packages/oauth/oauth-provider/dist/router/assets/assets.js`
);
const customization = {
  availableUserDomains: [".linkjar.social"],
  inviteCodeRequired: false,
  externalProviders: [
    { id: "apple", label: "Apple" },
    { id: "google", label: "Google" },
    { id: "github", label: "GitHub" },
  ],
};
if (process.env.LINKJAR_BRANDING_TEST) {
  const { readEnv } = await import(`${upstream}/packages/pds/dist/config/env.js`);
  const { envToCfg } = await import(`${upstream}/packages/pds/dist/config/config.js`);
  const { brandingSchema } = await import(`${upstream}/packages/oauth/oauth-provider/dist/customization/branding.js`);
  const config = envToCfg({ ...readEnv(), blobstoreDiskLocation: "/tmp/unused" });
  customization.branding = brandingSchema.parse(config.oauth.provider.branding);
  assert.deepEqual(config.oauth.provider.trustedClients, [
    "https://app.linkjar.io/client-metadata.json",
    "https://linkjar.io/ios-client-metadata.json",
    "https://linkjar.io/ext-client-metadata.json",
  ]);
}
const render = sendAuthorizePageFactory(customization, { hsts: false });
const renderAccount = sendAccountPageFactory(customization, { hsts: false });
const requestUri =
  "urn:ietf:params:oauth:request_uri:req-01234567890123456789012345678901";
let origin;
let deactivated = false;
const account = {
  did: "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa",
  pds: "did:web:pds.test",
  handle: "alice.linkjar.social",
  get deactivated() {
    return deactivated;
  },
};
const ephemeralToken =
  "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJkaWQ6cGxjOmFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYSJ9.c2lnbmF0dXJl";
const fake = {
  get issuer() {
    return origin;
  },
  deviceManager: {
    load: async () => ({
      deviceId: "dev-a",
      deviceMetadata: { ipAddress: "127.0.0.1" },
    }),
  },
  requestManager: {
    get: async (uri) => {
      assert.equal(uri, requestUri);
      return {
        clientId: "https://client.test/metadata.json",
        parameters: {
          scope: "atproto",
          response_type: "code",
          redirect_uri: "https://client.test/callback",
        },
      };
    },
    setAuthorized: async () => "bound-code",
  },
  signer: {
    verifyEphemeralToken: async (token) => {
      assert.equal(token, ephemeralToken);
      return { payload: { sub: account.did, deviceId: "dev-a", requestUri } };
    },
  },
  accountManager: {
    getAccount: async () => ({ account, authorizedClients: new Map() }),
    getDeviceAccount: async () => {
      throw new Error("No remembered account");
    },
    setAuthorizedClient: async () => {},
    reactivateAccount: async () => {
      deactivated = false;
      return account;
    },
  },
  clientManager: { getClient: async (id) => ({ id }) },
  checkConsentRequired: () => true,
};
let api;
const server = createServer((req, res) => {
  origin = `http://${req.headers.host}`;
  api ??= createApiMiddleware(fake, {
    onError: (_req, _res, err) => console.error(err),
  });
  return assetsMiddleware(req, res, () => {
    if (req.url.startsWith("/@atproto/oauth-provider/~api")) {
      return api(req, res, () => res.writeHead(404).end());
    }
    if (req.url.startsWith("/oauth/external/google/complete")) {
      deactivated = req.url.includes("deactivated=1");
      return render(req, res, {
        issuer: origin,
        requestUri,
        parameters: { scope: "atproto" },
        client: {
          id: "https://client.test/metadata.json",
          metadata: {
            client_name: "Other app",
            client_uri: "https://client.test",
          },
          isTrusted: false,
          isFirstParty: false,
        },
        sessions: [{ account, loginRequired: false, ephemeralToken }],
        selectedDid: account.did,
        permissionSets: new Map(),
      });
    }
    if (req.url.startsWith("/account"))
      return renderAccount(req, res, { deviceSessions: [] });
    if (!req.url.startsWith("/oauth/authorize")) {
      res.writeHead(404).end();
      return;
    }
    render(req, res, {
      issuer: origin,
      requestUri,
      parameters: {
        scope: "atproto",
        prompt:
          new URL(req.url, origin).searchParams.get("prompt") || undefined,
      },
      client: {
        id: "https://client.test/metadata.json",
        metadata: { client_name: "LinkJar", client_uri: "https://linkjar.io" },
        isTrusted: true,
        isFirstParty: true,
      },
      sessions: [],
      permissionSets: new Map(),
    }).catch((error) => {
      console.error(error);
      res.writeHead(500).end();
    });
  });
});
server.listen(
  Number(process.env.UI_PORT || 0),
  process.env.UI_HOST || "127.0.0.1",
);
await once(server, "listening");
origin = `http://127.0.0.1:${server.address().port}`;
export { origin };
export const close = () => new Promise((resolve) => server.close(resolve));
