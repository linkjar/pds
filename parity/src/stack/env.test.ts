import assert from 'node:assert/strict'
import { after, before, test } from 'node:test'
import { entrywayEnvs, pdsEnv, PROFILES } from './env.ts'
import type { Secrets } from './env.ts'
import { CANDIDATE_TAG, targetSpec } from './targets.ts'

const previousImages = {
  PARITY_CANDIDATE_IMAGE: process.env.PARITY_CANDIDATE_IMAGE,
  PARITY_REFERENCE_IMAGE: process.env.PARITY_REFERENCE_IMAGE,
}

before(() => {
  delete process.env.PARITY_CANDIDATE_IMAGE
  delete process.env.PARITY_REFERENCE_IMAGE
})

after(() => {
  for (const [name, value] of Object.entries(previousImages)) {
    if (value === undefined) delete process.env[name]
    else process.env[name] = value
  }
})

// Fixed fixture values only; no generated run secrets or production credentials.
const secrets: Secrets = {
  jwtSecret: '11'.repeat(32),
  dpopSecret: '22'.repeat(32),
  adminPassword: 'dummy-parity-admin-password',
  plcRotationKeyHex: '33'.repeat(32),
  rateLimitBypassKey: '44'.repeat(16),
  relayAdminPassword: 'dummy-relay-admin-password',
  externalClientSecret: '55'.repeat(16),
  hcaptchaSecretKey: '66'.repeat(16),
  hcaptchaTokenSalt: '77'.repeat(16),
}

function assertFixtureMode(env: Record<string, string>, devMode: string): void {
  assert.deepEqual({
    devMode: env.PDS_DEV_MODE,
    disableSsrfProtection: env.PDS_DISABLE_SSRF_PROTECTION,
  }, {
    devMode,
    disableSsrfProtection: 'true',
  })
}

for (const profile of PROFILES) {
  test(`candidate ${profile} explicitly permits its private fixture services`, () => {
    assertFixtureMode(pdsEnv(targetSpec('candidate'), profile, secrets), 'true')
  })
}

for (const target of ['reference', 'official', 'repeat'] as const) {
  for (const profile of PROFILES) {
    test(`${target} ${profile} retains its existing fixture mode`, () => {
      const legacyDevMode = profile === 'writes-crash' ? 'true' : 'false'
      assertFixtureMode(pdsEnv(targetSpec(target), profile, secrets), legacyDevMode)
    })
  }
}

for (const profile of PROFILES) {
  test(`Reference image in the candidate ${profile} stack retains its existing fixture mode`, () => {
    const legacyDevMode = profile === 'writes-crash' ? 'true' : 'false'
    const env = pdsEnv(targetSpec('candidate'), profile, secrets, targetSpec('reference').image)
    assertFixtureMode(env, legacyDevMode)
  })
}

test('an explicit Candidate image swap enables fixture mode in a reference stack', () => {
  const env = pdsEnv(targetSpec('reference'), 'restore', secrets, targetSpec('candidate').image)
  assertFixtureMode(env, 'true')
})

test('the target image is used when no explicit image override is supplied', () => {
  const candidate = targetSpec('candidate')
  const reference = targetSpec('reference')
  assertFixtureMode(pdsEnv({ ...candidate, image: reference.image }, 'restore', secrets), 'false')
  assertFixtureMode(pdsEnv({ ...reference, image: candidate.image }, 'restore', secrets), 'true')
})

test('a configured Candidate image override determines which image needs fixture mode', () => {
  process.env.PARITY_CANDIDATE_IMAGE = 'example.invalid/linkjar-pds:fixture-candidate'
  try {
    const candidate = targetSpec('candidate')
    assertFixtureMode(pdsEnv(candidate, 'default', secrets), 'true')
    assertFixtureMode(pdsEnv(targetSpec('reference'), 'restore', secrets, candidate.image), 'true')
    assertFixtureMode(pdsEnv(candidate, 'restore', secrets, CANDIDATE_TAG), 'false')
  } finally {
    delete process.env.PARITY_CANDIDATE_IMAGE
  }
})

for (const profile of PROFILES) {
  test(`candidate ${profile} member and entryway inherit the allowed fixture mode`, () => {
    const { member, entryway } = entrywayEnvs(pdsEnv(targetSpec('candidate'), profile, secrets))
    assertFixtureMode(member, 'true')
    assertFixtureMode(entryway, 'true')
  })
}
