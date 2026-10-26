import test from "node:test";
import assert from "node:assert/strict";
import { createDraftSynchronizer } from "../src/lib/draftProtection.ts";

test("draft projections are ordered and duplicate state updates share the same acknowledgment", async () => {
  const writes: { documentId: number; pending: boolean; resolve(): void }[] = [];
  const sync = createDraftSynchronizer((documentId, pending) => new Promise<void>((resolve) => {
    writes.push({ documentId, pending, resolve });
  }));
  const dirty = sync(7, true);
  assert.equal(sync(7, true), dirty);
  const cleared = sync(7, false);
  assert.equal(sync(7, false), cleared);
  await Promise.resolve();
  assert.deepEqual(writes.map(({ documentId, pending }) => [documentId, pending]), [[7, true]]);
  writes[0].resolve();
  await dirty;
  await Promise.resolve();
  assert.deepEqual(writes.map(({ documentId, pending }) => [documentId, pending]), [[7, true], [7, false]]);
  writes[1].resolve();
  await cleared;
  assert.equal(sync(7, false), cleared);
  assert.equal(writes.length, 2);
});

test("new drafts cannot be overwritten by an older asynchronous clear", async () => {
  const writes: { pending: boolean; resolve(): void }[] = [];
  const sync = createDraftSynchronizer((_documentId, pending) => new Promise<void>((resolve) => {
    writes.push({ pending, resolve });
  }));
  const first = sync(1, true);
  const second = sync(1, false);
  const third = sync(1, true);
  await Promise.resolve();
  writes[0].resolve();
  await first;
  await Promise.resolve();
  writes[1].resolve();
  await second;
  await Promise.resolve();
  assert.deepEqual(writes.map(({ pending }) => pending), [true, false, true]);
  writes[2].resolve();
  await third;
});

test("document identities are never deduplicated together", async () => {
  const writes: number[] = [];
  const sync = createDraftSynchronizer(async (id) => { writes.push(id); });
  await sync(1, false);
  await sync(2, false);
  assert.deepEqual(writes, [1, 2]);
});

test("a failed write stays visible and a newer state can still be acknowledged", async () => {
  const failure = new Error("Draft protection is unavailable.");
  const writes: boolean[] = [];
  const sync = createDraftSynchronizer(async (_id, pending) => {
    writes.push(pending);
    if (writes.length === 1) throw failure;
  });
  const failed = sync(1, true);
  const recovered = sync(1, false);
  await assert.rejects(failed, (error: unknown) => error === failure);
  await recovered;
  assert.equal(sync(1, false), recovered);
  assert.deepEqual(writes, [true, false]);
});

test("the same projection can retry after its failed acknowledgment", async () => {
  let attempts = 0;
  const sync = createDraftSynchronizer(async () => {
    attempts += 1;
    if (attempts === 1) throw new Error("Retry this draft update.");
  });
  await assert.rejects(sync(1, true), /Retry this draft update/);
  await sync(1, true);
  assert.equal(attempts, 2);
});
