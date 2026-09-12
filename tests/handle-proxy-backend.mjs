import { pds } from './patched-runtime.mjs'
import { createRequire } from 'node:module'
const require = createRequire(`${pds}/package.json`)
const express = require('express')
const { createRouter } = await import(`${pds}/dist/well-known.js`)
const app = express()
// Reserved hosts deliberately have a record to prove the helper still rejects
// them. Ordinary hosted records come from the account manager in production.
app.use(
  createRouter({
    cfg: { identity: { serviceHandleDomains: ['.linkjar.social'] } },
    accountManager: {
      getAccount: async (h) =>
        [
          'stribog.linkjar.social',
          'app.linkjar.social',
          'pds.linkjar.social',
        ].includes(h)
          ? { did: 'did:plc:abcdefghijklmnopqrstuvwx' }
          : undefined,
    },
  }),
)
app.get('/xrpc/com.atproto.sync.getBlob', (_req, res) =>
  res
    .set('Set-Cookie', 'unsafe=1; Domain=.linkjar.social')
    .type('html')
    .send('<script>user-controlled</script>'),
)
app.get('/account', (_req, res) =>
  res
    .set('Set-Cookie', 'pds-session=example; HttpOnly')
    .send('canonical PDS account UI'),
)
app.listen(3000, '0.0.0.0', () => console.log('fixture ready'))

// Separate fake mail listener makes accidental routing to metrics detectable.
const mail = express()
mail.use(express.raw({ type: '*/*', limit: '1mb' }))
mail.post('/webhooks/resend', (req, res) => res
  .set('Set-Cookie', 'mail-fixture=unsafe')
  .json({ body: req.body.toString(), cookie: req.headers.cookie ?? null,
    signature: req.headers['svix-signature'] ?? null }))
mail.get('/metrics', (_req, res) => res.send('private-mail-metrics'))
mail.get('/heartbeat', (_req, res) => res.send('private-mail-heartbeat'))
mail.listen(3001, '0.0.0.0')
