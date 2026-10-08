// The harness CA and the edge certificate of one stack. The CA is trusted
// only by that stack's containers and by the harness's own HTTP client. It is
// never installed into a system or browser trust store, and its key stays in
// the gitignored run directory.

import { webcrypto } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
// @peculiar/x509 resolves its dependencies through tsyringe, which needs this first.
import 'reflect-metadata'
import * as x509 from '@peculiar/x509'

const ALG = { name: 'ECDSA', namedCurve: 'P-256', hash: 'SHA-256' } as const
const DAY = 24 * 60 * 60 * 1000

async function pkcs8Pem(key: webcrypto.CryptoKey): Promise<string> {
  const der = Buffer.from(await webcrypto.subtle.exportKey('pkcs8', key))
  const body = der.toString('base64').replace(/(.{64})/g, '$1\n').trimEnd()
  return `-----BEGIN PRIVATE KEY-----\n${body}\n-----END PRIVATE KEY-----\n`
}

/**
 * Writes `ca/ca.crt`, `tls/edge.crt` and `tls/edge.key` under `runDir`. Returns
 * true when it wrote new material, which the containers must then reload.
 */
export async function ensureTls(runDir: string, names: string[]): Promise<boolean> {
  const caFile = join(runDir, 'ca/ca.crt')
  const namesFile = join(runDir, 'tls/names.json')
  const wanted = JSON.stringify(names)
  if (existsSync(caFile) && existsSync(namesFile) && readFileSync(namesFile, 'utf8') === wanted) return false
  x509.cryptoProvider.set(webcrypto as never)
  mkdirSync(join(runDir, 'ca'), { recursive: true })
  mkdirSync(join(runDir, 'tls'), { recursive: true })

  const notBefore = new Date(Date.now() - DAY)
  const notAfter = new Date(Date.now() + 30 * DAY)
  const caKeys = await webcrypto.subtle.generateKey(ALG, true, ['sign', 'verify'])
  const ca = await x509.X509CertificateGenerator.createSelfSigned({
    serialNumber: '01',
    name: 'CN=LinkJar PDS parity harness (test only)',
    notBefore,
    notAfter,
    signingAlgorithm: ALG,
    keys: caKeys,
    extensions: [
      new x509.BasicConstraintsExtension(true, 0, true),
      new x509.KeyUsagesExtension(x509.KeyUsageFlags.keyCertSign | x509.KeyUsageFlags.cRLSign, true),
    ],
  })

  const leafKeys = await webcrypto.subtle.generateKey(ALG, true, ['sign', 'verify'])
  const leaf = await x509.X509CertificateGenerator.create({
    serialNumber: '02',
    subject: 'CN=parity edge',
    issuer: ca.subject,
    notBefore,
    notAfter,
    signingAlgorithm: ALG,
    publicKey: leafKeys.publicKey,
    signingKey: caKeys.privateKey,
    extensions: [
      new x509.BasicConstraintsExtension(false, undefined, true),
      new x509.KeyUsagesExtension(x509.KeyUsageFlags.digitalSignature, true),
      new x509.ExtendedKeyUsageExtension(['1.3.6.1.5.5.7.3.1']),
      new x509.SubjectAlternativeNameExtension(names.map((value) => ({ type: 'dns', value }))),
    ],
  })

  writeFileSync(caFile, ca.toString('pem') + '\n')
  writeFileSync(join(runDir, 'tls/edge.crt'), leaf.toString('pem') + '\n')
  writeFileSync(join(runDir, 'tls/edge.key'), await pkcs8Pem(leafKeys.privateKey), { mode: 0o644 })
  writeFileSync(namesFile, wanted)
  return true
}
