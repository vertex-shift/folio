import assert from "node:assert/strict";
import test from "node:test";
import { adjacentTab, unsavedTabs } from "../src/lib/tabs.ts";

test("tab navigation follows open document identities and wraps at both ends", () => {
  const ids = [3, 9, 14];
  assert.equal(adjacentTab(ids, 3, 1), 9);
  assert.equal(adjacentTab(ids, 14, 1), 3);
  assert.equal(adjacentTab(ids, 3, -1), 14);
  assert.equal(adjacentTab(ids, 14, -1), 9);
});

test("tab navigation handles the empty workspace, a single tab, and a closed identity", () => {
  assert.equal(adjacentTab([], null, 1), null);
  assert.equal(adjacentTab([9], 9, 1), 9);
  assert.equal(adjacentTab([9], 9, -1), 9);
  assert.equal(adjacentTab([3, 9], null, 1), 3);
  assert.equal(adjacentTab([3, 9], 7, -1), 9);
});

test("quit review includes inactive dirty documents and unapplied marks, without duplicating tabs", () => {
  const documents = [{ id: 1, dirty: false }, { id: 2, dirty: true }, { id: 3, dirty: false }];
  assert.deepEqual(unsavedTabs(documents, new Set([2, 3])), [documents[1], documents[2]]);
  assert.deepEqual(unsavedTabs(documents, new Set()), [documents[1]]);
  assert.deepEqual(unsavedTabs([{ id: 1, dirty: false }], new Set()), []);
});
