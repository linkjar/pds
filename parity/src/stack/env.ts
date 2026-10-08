// The PDS environment of a stack. The base is the production configuration
// (legacy/staging/provider.env.example and handles.env.example) with the
// outside services replaced by the stack's own. A profile changes a few
// variables for the scenarios that need them.

import { randomBytes } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { HOSTS, upstream } from './targets.ts'
import type { TargetSpec } from './targets.ts'

export const PROFILES = ['default', 'invites', 'firehose', 'captcha', 'perf'] as const
export type Profile = (typeof PROFILES)[number]

export function isProfile(value: string): value is Profile {
  return (PROFILES as readonly string[]).includes(value)
}

/** Generated once per run directory. None of these values is used anywhere else. */
export type Secrets = {
  jwtSecret: string
  dpopSecret: string
  adminPassword: string
  plcRotationKeyHex: string
  rateLimitBypassKey: string
  relayAdminPassword: string
  externalClientSecret: string
  hcaptchaSecretKey: string
  hcaptchaTokenSalt: string
}

export function ensureSecrets(runDir: string): Secrets {
  const file = join(runDir, 'secrets.json')
  if (existsSync(file)) return JSON.parse(readFileSync(file, 'utf8'))
  const hex = (bytes: number) => randomBytes(bytes).toString('hex')
  const secrets: Secrets = {
    jwtSecret: hex(32),
    dpopSecret: hex(32),
    adminPassword: hex(24),
    plcRotationKeyHex: hex(32),
    rateLimitBypassKey: hex(16),
    relayAdminPassword: hex(16),
    externalClientSecret: hex(16),
    hcaptchaSecretKey: hex(16),
    hcaptchaTokenSalt: hex(16),
  }
  mkdirSync(runDir, { recursive: true })
  writeFileSync(file, JSON.stringify(secrets, null, 2) + '\n', { mode: 0o600 })
  return secrets
}

/** Client ids the fixture identity providers accept. */
export const EXTERNAL_CLIENT_IDS = {
  apple: 'io.linkjar.parity.apple',
  google: 'parity.apps.googleusercontent.com',
  github: 'parity-github-client',
} as const

export const HCAPTCHA_SITE_KEY = '10000000-ffff-ffff-ffff-000000000001'

export function pdsEnv(spec: TargetSpec, profile: Profile, secrets: Secrets): Record<string, string> {
  const env: Record<string, string> = {
    // Identity: the production hostnames. Patch 093 keys on `.linkjar.social`.
    PDS_HOSTNAME: HOSTS.pds,
    PDS_PORT: '3000',
    PDS_SERVICE_HANDLE_DOMAINS: HOSTS.handleDomain,
    PDS_DEV_MODE: 'false',
    // The official distribution bakes its own release label into this
    // variable; every target states the package version instead.
    PDS_VERSION: upstream.tag.split('@').at(-1) ?? '',

    // Branding and policy, as on the production host.
    PDS_SERVICE_NAME: 'LinkJar',
    PDS_LOGO_URL: `https://${HOSTS.web}/logo/linkjar-128.png`,
    PDS_HOME_URL: `https://${HOSTS.web}`,
    PDS_PRIMARY_COLOR: '#1d5fec',
    PDS_ERROR_COLOR: '#ff383c',
    PDS_WARNING_COLOR: '#f5a524',
    PDS_INFO_COLOR: '#1d5fec',
    PDS_SUCCESS_COLOR: '#059669',
    PDS_TERMS_OF_SERVICE_URL: `https://${HOSTS.web}/terms`,
    PDS_PRIVACY_POLICY_URL: `https://${HOSTS.web}/privacy`,
    PDS_SUPPORT_URL: `https://${HOSTS.web}/support`,
    PDS_CONTACT_EMAIL_ADDRESS: 'support@linkjar.io',
    PDS_OAUTH_TRUSTED_CLIENTS: [
      `https://${HOSTS.app}/client-metadata.json`,
      `https://${HOSTS.web}/ios-client-metadata.json`,
      `https://${HOSTS.web}/ext-client-metadata.json`,
    ].join(','),

    // Storage.
    PDS_DATA_DIRECTORY: '/app/data',
    PDS_BLOBSTORE_DISK_LOCATION: '/app/data/blocks',

    // Secrets.
    PDS_JWT_SECRET: secrets.jwtSecret,
    PDS_DPOP_SECRET: secrets.dpopSecret,
    PDS_ADMIN_PASSWORD: secrets.adminPassword,
    PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX: secrets.plcRotationKeyHex,

    // The stack's own outside world.
    PDS_DID_PLC_URL: 'http://plc:2582',
    PDS_CRAWLERS: 'http://relay:2470',
    PDS_BSKY_APP_VIEW_URL: `https://${HOSTS.appview}`,
    PDS_BSKY_APP_VIEW_DID: `did:web:${HOSTS.appview}`,
    PDS_REPORT_SERVICE_URL: `https://${HOSTS.mod}`,
    PDS_REPORT_SERVICE_DID: `did:web:${HOSTS.mod}`,
    PDS_EMAIL_SMTP_URL: 'smtp://mail:1025',
    PDS_EMAIL_FROM_ADDRESS: 'accounts@linkjar.io',
    // Without these `com.atproto.admin.sendEmail` answers `sent: true` and sends nothing.
    PDS_MODERATION_EMAIL_SMTP_URL: 'smtp://mail:1025',
    PDS_MODERATION_EMAIL_ADDRESS: 'moderation@linkjar.io',
    // Every outside name resolves to a private address inside the stack.
    PDS_DISABLE_SSRF_PROTECTION: 'true',

    // Accounts and limits.
    PDS_INVITE_REQUIRED: 'false',
    PDS_RATE_LIMITS_ENABLED: 'true',
    PDS_RATE_LIMIT_BYPASS_KEY: secrets.rateLimitBypassKey,

    // External sign-in (patch 099). The stock build ignores these.
    PDS_EXTERNAL_APPLE_CLIENT_ID: EXTERNAL_CLIENT_IDS.apple,
    PDS_EXTERNAL_APPLE_CLIENT_SECRET: secrets.externalClientSecret,
    PDS_EXTERNAL_GOOGLE_CLIENT_ID: EXTERNAL_CLIENT_IDS.google,
    PDS_EXTERNAL_GOOGLE_CLIENT_SECRET: secrets.externalClientSecret,
    PDS_EXTERNAL_GITHUB_CLIENT_ID: EXTERNAL_CLIENT_IDS.github,
    PDS_EXTERNAL_GITHUB_CLIENT_SECRET: secrets.externalClientSecret,

    LOG_ENABLED: 'true',
    LOG_LEVEL: 'info',
  }

  if (profile === 'invites') {
    env.PDS_INVITE_REQUIRED = 'true'
  }
  if (profile === 'firehose') {
    // Small limits so that the two cursor rows of SPEC 7.3 that depend on
    // them can be reached in seconds: a five-second backfill window and a
    // subscriber buffer of five events.
    env.PDS_REPO_BACKFILL_LIMIT_MS = '5000'
    env.PDS_MAX_SUBSCRIPTION_BUFFER = '5'
  }
  if (profile === 'captcha') {
    env.PDS_HCAPTCHA_SITE_KEY = HCAPTCHA_SITE_KEY
    env.PDS_HCAPTCHA_SECRET_KEY = secrets.hcaptchaSecretKey
    env.PDS_HCAPTCHA_TOKEN_SALT = secrets.hcaptchaTokenSalt
  }
  if (profile === 'perf') {
    // Seeding 100,000 records would spend the write limiters many times over.
    env.PDS_RATE_LIMITS_ENABLED = 'false'
    delete env.PDS_RATE_LIMIT_BYPASS_KEY
    env.LOG_LEVEL = 'warn'
  }
  return env
}

export function writePdsEnv(spec: TargetSpec, env: Record<string, string>): void {
  const lines = Object.entries(env).map(([key, value]) => `${key}=${value}`)
  writeFileSync(join(spec.runDir, 'pds.env'), lines.join('\n') + '\n', { mode: 0o600 })
}
