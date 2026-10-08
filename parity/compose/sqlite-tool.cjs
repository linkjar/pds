// Runs one SQL statement against a database of the PDS data directory and
// prints the rows as JSON. The harness uses it for two things the Reference
// offers no interface for: reading a stored expiry, and ageing a row so that
// a scenario does not have to wait hours for a lifetime to pass.
//
// It runs in a one-shot container of the Reference image, which carries the
// SQLite driver, with the data volume of whichever target is under test.
// Schema version 1 is the Reference schema (SPEC 8.2), so the same statements
// work against a Candidate's data directory.
const { createRequire } = require('node:module')
const Database = createRequire('/app/packages/pds/package.json')('better-sqlite3')

let input = ''
process.stdin.on('data', (chunk) => (input += chunk))
process.stdin.on('end', () => {
  const { file, sql, params } = JSON.parse(input)
  const db = new Database(file, { timeout: 10000, fileMustExist: true })
  const statement = db.prepare(sql)
  const rows = statement.reader ? statement.all(...params) : (statement.run(...params), [])
  db.close()
  process.stdout.write(JSON.stringify(rows))
})
