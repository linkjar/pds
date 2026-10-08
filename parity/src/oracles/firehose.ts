// Oracle O3 (SPEC 17.3, 7.1): what a firehose frame must contain, checked
// frame by frame, and a summary of each event for the transcript.

import assert from 'node:assert/strict'
import * as dagCbor from '@ipld/dag-cbor'
import { readCarWithRoot, verifyCommitSig } from '@atproto/repo'
import { commitView, eventDid } from '../lib/firehose.ts'
import type { Frame } from '../lib/firehose.ts'
import type { Scenario } from '../scenario.ts'
import { signingKey } from './repo.ts'

const MAX_FRAME_BYTES = 5_000_000

type Summary = Record<string, unknown>

/** One event as the transcript records it. `seq` is left out: its value depends on what ran before. */
export async function summarise(frame: Frame): Promise<Summary> {
  if (frame.kind === 'error') return { error: frame.error, message: frame.message }
  const body = frame.body
  switch (frame.type) {
    case '#commit': {
      const view = await commitView(body)
      return {
        type: frame.type,
        repo: view.repo,
        rev: view.rev,
        since: view.since,
        commit: view.commit,
        prevData: view.prevData ?? null,
        ops: view.ops.map((op) => ({ action: op.action, path: op.path, cid: op.cid, prev: op.prev ?? null })),
        blocks: view.blockCids.length,
        blobs: view.blobs,
        tooBig: view.tooBig,
        rebase: body.rebase,
        fields: Object.keys(body).sort(),
      }
    }
    case '#sync': {
      const car = await readCarWithRoot(body.blocks as Uint8Array)
      return { type: frame.type, did: body.did, rev: body.rev, root: car.root, blocks: car.blocks.size, fields: Object.keys(body).sort() }
    }
    default: {
      const { seq: _seq, time: _time, ...rest } = body
      return { type: frame.type, ...rest, fields: Object.keys(body).sort() }
    }
  }
}

export async function summariseAll(frames: Frame[]): Promise<Summary[]> {
  return Promise.all(frames.map((frame) => summarise(frame)))
}

/**
 * Checks every `#commit` and `#sync` of one repository, in stream order:
 * the frame limit, the CAR slice, the signature, and the links between
 * consecutive commits (`since`, `prevData`, the `prev` of each operation).
 */
export async function checkRepoStream(s: Scenario, did: string, frames: Frame[]): Promise<void> {
  const didKey = await signingKey(s, did)
  let previous: { rev: string; data: string } | undefined
  const records = new Map<string, string>()
  let lastSeq = 0
  for (const frame of frames) {
    if (frame.kind !== 'event' || eventDid(frame) !== did) continue
    assert.ok(frame.bytes < MAX_FRAME_BYTES, 'a frame stays under 5 MB (SPEC 7.2)')
    const seq = frame.body.seq as number
    assert.ok(Number.isSafeInteger(seq) && seq > lastSeq, 'seq increases (SPEC 7.1)')
    lastSeq = seq

    if (frame.type === '#sync') {
      const car = await readCarWithRoot(frame.body.blocks as Uint8Array)
      const commit = dagCbor.decode(car.blocks.get(car.root)!) as Record<string, unknown>
      assert.equal(commit.rev, frame.body.rev, '#sync carries the commit of its rev')
      assert.equal(car.blocks.size, 1, '#sync carries the commit block only')
      continue
    }
    if (frame.type !== '#commit') continue

    const view = await commitView(frame.body)
    const car = await readCarWithRoot(view.carBytes)
    const commitBlock = car.blocks.get(car.root)
    assert.ok(commitBlock, 'the commit block is in the slice')
    assert.equal(view.root.toString(), view.commit.toString(), 'the commit is roots[0] (SPEC 7.1)')
    const commit = dagCbor.decode(commitBlock) as Record<string, any>
    assert.equal(commit.did, did)
    assert.equal(commit.rev, view.rev)
    assert.ok(await verifyCommitSig(commit as never, didKey), 'the commit is signed with the key in the PLC document')
    assert.deepEqual(view.blobs, [], '`blobs` is always empty (SPEC 7.1)')
    assert.equal(view.tooBig, false, '`tooBig` is always false (SPEC 7.1)')

    if (previous) {
      assert.equal(view.since, previous.rev, '`since` names the previous revision')
      assert.equal(view.prevData?.toString(), previous.data, '`prevData` is the previous MST root')
    } else {
      assert.equal(view.since, null, 'the first commit has no `since`')
    }
    const present = new Set(view.blockCids)
    for (const op of view.ops) {
      const before = records.get(op.path)
      if (op.action === 'create') {
        assert.equal(before, undefined, `create of a new path ${op.path}`)
        assert.ok(op.cid && present.has(op.cid.toString()), 'a created record is in the slice')
        records.set(op.path, op.cid.toString())
      } else if (op.action === 'update') {
        assert.equal(op.prev?.toString(), before, 'update carries the previous CID')
        assert.ok(op.cid && present.has(op.cid.toString()), 'an updated record is in the slice')
        records.set(op.path, op.cid.toString())
      } else {
        assert.equal(op.action, 'delete')
        assert.equal(op.cid, null)
        assert.equal(op.prev?.toString(), before, 'delete carries the previous CID')
        assert.ok(before && !present.has(before), 'a deleted record body is not in the slice')
        records.delete(op.path)
      }
    }
    previous = { rev: view.rev, data: String(commit.data) }
  }
}
