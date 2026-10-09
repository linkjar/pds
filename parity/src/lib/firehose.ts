// Firehose reading for oracle O3 (SPEC 17.3). Frames are decoded with the
// same libraries the SDK uses: two DAG-CBOR objects per message, and the CAR
// slice of a commit through `@atproto/repo`.

import { performance } from 'node:perf_hooks'
import * as dagCbor from '@ipld/dag-cbor'
import { readCarWithRoot } from '@atproto/repo'
import { decodeFirst } from 'cborg'
import type WebSocket from 'ws'
import type { Target } from './target.ts'

/** `at` is when the frame arrived, on the monotonic clock, for the delivery-lag measurements. */
export type Frame =
  | { kind: 'event'; type: string; body: Record<string, unknown>; bytes: number; at: number }
  | { kind: 'error'; error: string; message?: string; bytes: number; at: number }

export function decodeFrame(data: Uint8Array): Frame {
  const [header, rest] = decodeFirst(data, dagCbor.decodeOptions) as [{ op: number; t?: string }, Uint8Array]
  const body = dagCbor.decode(rest) as Record<string, unknown>
  const at = performance.now()
  if (header.op === -1) {
    return { kind: 'error', error: String(body.error), message: body.message as string | undefined, bytes: data.byteLength, at }
  }
  return { kind: 'event', type: String(header.t), body, bytes: data.byteLength, at }
}

export type Subscription = {
  frames: Frame[]
  /** Set when the server closed the socket or the upgrade failed. */
  closed: Promise<{ code: number; httpStatus?: number; httpBody?: string }>
  /** Resolves when `count` frames matching `match` have arrived, or rejects after `timeoutMs`. */
  waitFor(match: (frame: Frame) => boolean, count?: number, timeoutMs?: number): Promise<Frame[]>
  /** Resolves when no frame has arrived for `quietMs`. */
  settle(quietMs?: number): Promise<Frame[]>
  /**
   * Waits until the stream is quiet and returns a position in it. The
   * Reference hands events to live subscribers from a database poll that
   * backs off to one second, so a subscription opened right after a write
   * still receives that write. A scenario that subscribes in mid-flight calls
   * this first and then reads with `after`.
   */
  quiet(): Promise<number>
  /** Like `waitFor`, counting only frames at or after `mark`. */
  after(mark: number, match: (frame: Frame) => boolean, count?: number, timeoutMs?: number): Promise<Frame[]>
  /**
   * The frames from `mark` on that `match` accepts, once one of them satisfies
   * `until` and the stream is quiet again. Naming the event that ends the
   * sequence keeps the result the same on a fast and on a slow host.
   */
  collect(mark: number, match: (frame: Frame) => boolean, until: (frame: Frame) => boolean): Promise<Frame[]>
  /** Stops reading from the socket, so that the server sees a consumer that does not keep up. */
  pause(): void
  resume(): void
  close(): void
}

/** Subscribes to `subscribeRepos` on the PDS or the relay of a stack. */
export function subscribe(target: Target, host: string, cursor?: number | string): Subscription {
  const query = cursor === undefined ? '' : `?cursor=${cursor}`
  const socket: WebSocket = target.socket(host, `/xrpc/com.atproto.sync.subscribeRepos${query}`)
  const frames: Frame[] = []
  let lastFrameAt = Date.now()
  const listeners = new Set<() => void>()
  socket.on('message', (data: Buffer) => {
    frames.push(decodeFrame(new Uint8Array(data)))
    lastFrameAt = Date.now()
    for (const listener of listeners) listener()
  })
  const closed = new Promise<{ code: number; httpStatus?: number; httpBody?: string }>((resolve) => {
    socket.on('close', (code) => resolve({ code }))
    socket.on('unexpected-response', (_request, response) => {
      let body = ''
      response.on('data', (chunk: Buffer) => (body += chunk.toString('utf8')))
      response.on('end', () => resolve({ code: 0, httpStatus: response.statusCode, httpBody: body }))
    })
    socket.on('error', () => undefined)
  })
  return {
    frames,
    closed,
    waitFor(match, count = 1, timeoutMs = 15_000) {
      return new Promise((resolve, reject) => {
        const check = () => {
          const found = frames.filter(match)
          if (found.length >= count) {
            listeners.delete(check)
            clearTimeout(timer)
            resolve(found)
          }
        }
        const timer = setTimeout(() => {
          listeners.delete(check)
          reject(new Error(`Timed out waiting for ${count} matching frame(s); saw ${frames.filter(match).length} of ${frames.length}`))
        }, timeoutMs)
        listeners.add(check)
        check()
      })
    },
    async settle(quietMs = 700) {
      while (Date.now() - lastFrameAt < quietMs) {
        await new Promise((resolve) => setTimeout(resolve, 50))
      }
      return frames
    },
    async quiet() {
      await this.settle(1_500)
      return frames.length
    },
    after(mark, match, count = 1, timeoutMs = 15_000) {
      const position = new Map<Frame, number>()
      return this.waitFor((frame) => {
        let index = position.get(frame)
        if (index === undefined) position.set(frame, (index = frames.indexOf(frame)))
        return index >= mark && match(frame)
      }, count, timeoutMs)
    },
    async collect(mark, match, until) {
      await this.after(mark, (frame) => match(frame) && until(frame), 1)
      const end = await this.quiet()
      return frames.slice(mark, end).filter(match)
    },
    pause() {
      socket.pause()
    },
    resume() {
      socket.resume()
    },
    close() {
      socket.close()
    },
  }
}

export const isEvent = (frame: Frame): frame is Extract<Frame, { kind: 'event' }> => frame.kind === 'event'

/** The DID an event is about. `#info` has none. */
export function eventDid(frame: Frame): string | undefined {
  if (frame.kind !== 'event') return undefined
  return (frame.body.repo ?? frame.body.did) as string | undefined
}

export const forDids = (dids: string[]) => (frame: Frame) => {
  const did = eventDid(frame)
  return did !== undefined && dids.includes(did)
}

/** A CID as the SDK hands it out. Only its string form is used here. */
export type CidLike = { toString(): string }

export type CommitView = {
  seq: number
  repo: string
  rev: string
  since: string | null
  commit: CidLike
  prevData: CidLike | undefined
  tooBig: unknown
  blobs: unknown[]
  ops: { action: string; path: string; cid: CidLike | null; prev: CidLike | undefined }[]
  /** CIDs of the blocks in the CAR slice, with the root first. */
  root: CidLike
  blockCids: string[]
  carBytes: Uint8Array
}

/** Reads a `#commit` body and its CAR slice. */
export async function commitView(body: Record<string, unknown>): Promise<CommitView> {
  const carBytes = body.blocks as Uint8Array
  const car = await readCarWithRoot(carBytes)
  const ops = (body.ops as Record<string, unknown>[]).map((op) => ({
    action: String(op.action),
    path: String(op.path),
    cid: (op.cid as CidLike | null) ?? null,
    prev: op.prev as CidLike | undefined,
  }))
  return {
    seq: body.seq as number,
    repo: String(body.repo),
    rev: String(body.rev),
    since: (body.since as string | null) ?? null,
    commit: body.commit as CidLike,
    prevData: body.prevData as CidLike | undefined,
    tooBig: body.tooBig,
    blobs: body.blobs as unknown[],
    ops,
    root: car.root,
    blockCids: car.blocks.entries().map((entry) => entry.cid.toString()),
    carBytes,
  }
}
