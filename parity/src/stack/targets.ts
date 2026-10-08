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
  stock: { tag: string; build: { context: string; args: Record<string, string> } }
  relay: PinnedImage
  plc: { repository: string; commit: string }
  mail: PinnedImage
  edge: PinnedImage
  plcDatabase: PinnedImage
}
type Upstream = { tag: string; commit: string; revision: number; officialImage: string }

export const images: Images = JSON.parse(readFileSync(join(PARITY_DIR, 'images.json'), 'utf8'))
export const upstream: Upstream = JSON.parse(readFileSync(join(REPO_DIR, 'upstream.json'), 'utf8'))

const byDigest = (pin: PinnedImage) => `${pin.image}@${pin.digest}`

export const TARGET_NAMES = ['reference', 'stock', 'official', 'candidate'] as const
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
    stock: process.env.PARITY_STOCK_IMAGE ?? images.stock.tag,
    official: upstream.officialImage,
    candidate: process.env.PARITY_CANDIDATE_IMAGE ?? '',
  }[name]
  if (!image) {
    throw new Error(`Target ${name} has no image. Set PARITY_CANDIDATE_IMAGE (the Candidate serves traffic from unit 3).`)
  }
  return {
    name,
    image,
    linkjar: name === 'reference' || name === 'candidate',
    edgePort: PORT_BASE + index * 10,
    fixturePort: PORT_BASE + index * 10 + 1,
    runDir: join(PARITY_DIR, '.run', name),
  }
}

export const composeImages = () => ({
  EDGE_IMAGE: byDigest(images.edge),
  RELAY_IMAGE: byDigest(images.relay),
  MAIL_IMAGE: byDigest(images.mail),
  PLC_DATABASE_IMAGE: byDigest(images.plcDatabase),
  PLC_REPOSITORY: images.plc.repository,
  PLC_COMMIT: images.plc.commit,
})

/** Names every stack answers to. The PDS runs under the production identity. */
export const HOSTS = {
  pds: 'pds.linkjar.social',
  handleDomain: '.linkjar.social',
  plc: 'plc.test',
  relay: 'relay.test',
  mail: 'mail.test',
  appview: 'appview.test',
  mod: 'mod.test',
  client: 'client.test',
  app: 'app.linkjar.io',
  web: 'linkjar.io',
} as const

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
