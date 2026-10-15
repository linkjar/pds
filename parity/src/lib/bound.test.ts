import assert from 'node:assert/strict'
import { test } from 'node:test'
import { bound, eventually, runCleanups, TimeoutError } from './bound.ts'

const never = () => new Promise<never>(() => {})

test('bound passes a value through and gives up at its limit', async () => {
  assert.equal(await bound(Promise.resolve(7), 50, 'seven'), 7)
  await assert.rejects(
    bound(never(), 20, 'a wait that never ends'),
    (error: unknown) => error instanceof TimeoutError && error.message === 'a wait that never ends did not finish within 20 ms',
  )
  await assert.rejects(bound(Promise.reject(new Error('its own failure')), 50, 'a failing wait'), { message: 'its own failure' })
})

test('a cleanup that never finishes is left behind at the bound, and the others still run', async () => {
  const ran: string[] = []
  const warnings: string[] = []
  const started = Date.now()
  await runCleanups(
    [
      { what: 'the first registered', run: () => void ran.push('first') },
      { what: 'a failing one', run: () => assert.fail('boom') },
      { what: 'a wedged one', run: () => never() },
      { what: 'the last registered', run: async () => void ran.push('last') },
    ],
    50,
    (line) => warnings.push(line),
  )
  assert.deepEqual(ran, ['last', 'first'])
  assert.deepEqual(warnings, ['a wedged one did not finish within 50 ms; moving on', 'a failing one failed: boom'])
  assert.ok(Date.now() - started < 1_000, 'the wedged cleanup held the others up for its bound only')
})

test('eventually gives up at its deadline when the probe never returns', async () => {
  await assert.rejects(eventually('a reply that never comes', () => never(), 60), { message: 'Timed out waiting for a reply that never comes' })
})

test('eventually polls a probe that answers until it has a value', async () => {
  let calls = 0
  assert.equal(await eventually('the third call', async () => (++calls >= 3 ? calls : undefined), 2_000), 3)
})
