// Targets and where their stacks live. A target is an image plus the shared
// configuration; nothing else may differ between two targets (SPEC 17.1).

import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const PARITY_DIR = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
export const REPO_DIR = resolve(PARITY_DIR, '..')
export const COMPOSE_FILE = join(PARITY_DIR, 'compose/compose.yml')

type PinnedImage = { image: string; tag: string; digest: string }
type Images = {
  reference: PinnedImage & { revision: string }
  relay: PinnedImage
  plc: { repository: string; commit: string }
  mail: PinnedImage
  edge: PinnedImage
  plcDatabase: PinnedImage
  s3: PinnedImage
}
type Upstream = { tag: string; commit: string; revision: number; officialImage: string }

export const images: Images = JSON.parse(readFileSync(join(PARITY_DIR, 'images.json'), 'utf8'))
export const upstream: Upstream = JSON.parse(readFileSync(join(REPO_DIR, 'upstream.json'), 'utf8'))

const byDigest = (pin: PinnedImage) => `${pin.image}@${pin.digest}`

// `repeat` is a second boot of the Reference image. Comparing it with
// `reference` must show no difference, which is the check on the normaliser.
export const TARGET_NAMES = ['reference', 'official', 'candidate', 'repeat'] as const
export type TargetName = (typeof TARGET_NAMES)[number]

export type TargetSpec = {
  name: TargetName
  /** What the stack runs as the PDS. */
  image: string
  /** Whether the LinkJar patch stack (or, later, the linkjar profile) is present. */
  linkjar: boolean
  /** Host port of the TLS edge. */
  edgePort: number
  /** Host port of the fixture server the edge forwards outside names to. */
  fixturePort: number
  runDir: string
}

/** The tag `pnpm parity images build-candidate` gives the Candidate image. */
export const CANDIDATE_TAG = 'linkjar-pds:candidate'

const PORT_BASE = Number(process.env.PARITY_PORT_BASE ?? 18400)

export function isTargetName(value: string): value is TargetName {
  return (TARGET_NAMES as readonly string[]).includes(value)
}

export function targetSpec(name: TargetName): TargetSpec {
  const index = TARGET_NAMES.indexOf(name)
  const image = {
    // The Reference is always the published image, by digest. An override
    // exists for one case only: checking an upstream bump before it is
    // published, where the next image is compared with the current one.
    reference: process.env.PARITY_REFERENCE_IMAGE ?? byDigest(images.reference),
    official: upstream.officialImage,
    // An empty variable counts as unset.
    candidate: process.env.PARITY_CANDIDATE_IMAGE || CANDIDATE_TAG,
    repeat: process.env.PARITY_REFERENCE_IMAGE ?? byDigest(images.reference),
  }[name]
  return {
    name,
    image,
    linkjar: name === 'reference' || name === 'candidate' || name === 'repeat',
    edgePort: PORT_BASE + index * 10,
    fixturePort: PORT_BASE + index * 10 + 1,
    runDir: join(PARITY_DIR, '.run', name),
  }
}

export const composeImages = () => ({
  // The SQL tool always runs in the Reference image, whatever the target.
  TOOL_IMAGE: byDigest(images.reference),
  EDGE_IMAGE: byDigest(images.edge),
  RELAY_IMAGE: byDigest(images.relay),
  MAIL_IMAGE: byDigest(images.mail),
  PLC_DATABASE_IMAGE: byDigest(images.plcDatabase),
  S3_IMAGE: byDigest(images.s3),
  PLC_REPOSITORY: images.plc.repository,
  PLC_COMMIT: images.plc.commit,
})

/** Names every stack answers to. The PDS runs under the production identity. */
export const HOSTS = {
  pds: 'pds.linkjar.social',
  /** The entryway of the topology with an entryway (src/stack/env.ts). */
  entryway: 'entryway.linkjar.social',
  handleDomain: '.linkjar.social',
  plc: 'plc.test',
  relay: 'relay.test',
  mail: 'mail.test',
  appview: 'appview.test',
  mod: 'mod.test',
  client: 'client.parity.linkjar.io',
  /** A custom-domain handle whose well-known file the fixture server answers. */
  customHandle: 'handle.test',
  app: 'app.linkjar.io',
  web: 'linkjar.io',
} as const

/**
 * Whether the scenarios run against the Candidate as a member host behind
 * an entryway (PARITY_CANDIDATE_ENTRYWAY=1, src/stack/env.ts).
 */
export function behindEntryway(): boolean {
  return process.env.PARITY_TARGET === 'candidate' && process.env.PARITY_CANDIDATE_ENTRYWAY === '1'
}

/**
 * The server that keeps the accounts: it signs the tokens and is the OAuth
 * issuer. The entryway behind one, the PDS otherwise.
 */
export function accountHost(): string {
  return behindEntryway() ? HOSTS.entryway : HOSTS.pds
}

/** Subject alternative names of the edge certificate. */
export const EDGE_NAMES = [
  HOSTS.pds,
  `*${HOSTS.handleDomain}`,
  // No `*.test`: TLS clients refuse a wildcard directly under a top-level domain.
  HOSTS.plc,
  HOSTS.relay,
  HOSTS.mail,
  HOSTS.appview,
  HOSTS.mod,
  HOSTS.client,
  HOSTS.customHandle,
  'edge.test',
  HOSTS.app,
  HOSTS.web,
  'accounts.google.com',
  'oauth2.googleapis.com',
  'www.googleapis.com',
  'github.com',
  'api.github.com',
  'appleid.apple.com',
  'api.hcaptcha.com',
]
