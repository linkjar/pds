// Bounds on the waits the harness makes on a scenario's behalf. A scenario
// that wedges must fail the run, not hang it: on 2026-10-15 one sat for an
// hour on a browser that never answered, and the run with it.

export class TimeoutError extends Error {
  constructor(what: string, ms: number) {
    super(`${what} did not finish within ${ms} ms`)
    this.name = 'TimeoutError'
  }
}

/**
 * The value of `promise`, or a `TimeoutError` after `ms`. The promise is
 * then abandoned: what it settles to later is dropped, and a late rejection
 * is not an unhandled one.
 */
export function bound<T>(promise: Promise<T>, ms: number, what: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new TimeoutError(what, ms)), ms)
    promise.then(
      (value) => {
        clearTimeout(timer)
        resolve(value)
      },
      (error: unknown) => {
        clearTimeout(timer)
        reject(error)
      },
    )
  })
}

export type Cleanup = { what: string; run: () => void | Promise<void> }

/**
 * Runs cleanups, the last registered first. One that throws, or that does
 * not finish within `ms`, is reported through `warn` and the next one still
 * runs: a browser that does not close must not keep a scenario's sockets open.
 */
export async function runCleanups(cleanups: readonly Cleanup[], ms: number, warn: (line: string) => void): Promise<void> {
  for (const { what, run } of [...cleanups].reverse()) {
    try {
      await bound(Promise.resolve().then(run), ms, what)
    } catch (error) {
      warn(error instanceof TimeoutError ? `${error.message}; moving on` : `${what} failed: ${describe(error)}`)
    }
  }
}

/**
 * Waits until `probe` returns a value, polling quietly. The deadline holds
 * when a probe does not return: that probe is abandoned, and the wait fails.
 */
export async function eventually<T>(what: string, probe: () => Promise<T | undefined | false>, timeoutMs = 15_000): Promise<T> {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    let value: T | undefined | false
    try {
      value = await bound(probe(), Math.max(1, deadline - Date.now()), `the probe for ${what}`)
    } catch (error) {
      if (!(error instanceof TimeoutError)) throw error
      value = false
    }
    if (value) return value
    if (Date.now() > deadline) throw new Error(`Timed out waiting for ${what}`)
    await new Promise((resolve) => setTimeout(resolve, 150))
  }
}

export function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
