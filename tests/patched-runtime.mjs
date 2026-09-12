import { readFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
import { stripTypeScriptTypes } from 'node:module'
export const pds = '/app/packages/pds'
const changed = [
  'handle/index',
  'handle/linkjar',
  'handle/linkjar-reserved',
  'account-manager/account-manager',
  'account-manager/helpers/account',
  'account-manager/helpers/linkjar-handle-policy',
  'account-manager/db/migrations/index',
  'account-manager/db/migrations/007a-linkjar-093',
  'well-known',
]
for (const file of process.env.LINKJAR_USE_COMPILED ? [] : changed) {
  const target = `${pds}/dist/${file}.js`
  mkdirSync(dirname(target), { recursive: true })
  writeFileSync(
    target,
    stripTypeScriptTypes(
      readFileSync(`/patch-src/packages/pds/src/${file}.ts`, 'utf8'),
      { mode: 'transform' },
    ),
  )
}
