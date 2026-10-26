import assert from "node:assert/strict";
import test from "node:test";
import {
  bookmarkMove, metadataChanged, suggestedAttachmentName, validateCopyProtection,
} from "../src/lib/documentUtilitiesModel.ts";
import type { BookmarkInfo, CopyProtection, MetadataDraft } from "../src/lib/documentUtilitiesModel.ts";

test("metadata deletion, whitespace edits and inactive-tab drafts remain pending until applied or discarded", () => {
  const original = { title: "Original", author: "Author", subject: "", keywords: "review" };
  const draft: MetadataDraft = { documentId: 7, revision: 3, original, fields: { ...original } };
  assert.equal(metadataChanged(null), false);
  assert.equal(metadataChanged(draft), false);
  assert.equal(metadataChanged({ ...draft, fields: { ...original, title: "" } }), true);
  assert.equal(metadataChanged({ ...draft, fields: { ...original, author: "Author " } }), true);
});

test("suggested attachment downloads discard paths, control characters and Windows device names", () => {
  for (const [value, expected] of [
    ["../../資料.txt", "資料.txt"], ["C:\\private\\safe.txt", "safe.txt"],
    ["NUL.txt", "_NUL.txt"], ["con", "_con"], ["..", "attachment.bin"],
    ["bad:\u0000?.txt", "bad___.txt"], ["  café.txt. ", "café.txt"],
  ]) assert.equal(suggestedAttachmentName(value), expected);
  assert.equal(Array.from(suggestedAttachmentName("é".repeat(200))).length, 120);
  assert.equal(new TextEncoder().encode(suggestedAttachmentName("📄".repeat(120))).length, 240);
});

function bookmark(id: string, parent: string | null, position: number, childCount = 0): BookmarkInfo {
  return { id, parent, position, childCount, title: id, depth: parent ? 1 : 0,
    page: 0, namedDestination: null, url: null, targetDescription: "Page 1" };
}

test("bookmark organization uses post-removal indices and valid sibling/parent destinations", () => {
  const entries = [bookmark("a", null, 0, 1), bookmark("child", "a", 0), bookmark("b", null, 1), bookmark("c", null, 2)];
  assert.equal(bookmarkMove(entries, entries[0], "up"), null);
  assert.equal(bookmarkMove(entries, entries[0], "in"), null);
  assert.equal(bookmarkMove(entries, entries[0], "out"), null);
  assert.deepEqual(bookmarkMove(entries, entries[2], "up"), { parent: null, at: 0 });
  assert.deepEqual(bookmarkMove(entries, entries[2], "down"), { parent: null, at: 2 });
  assert.deepEqual(bookmarkMove(entries, entries[2], "in"), { parent: "a", at: 1 });
  assert.deepEqual(bookmarkMove(entries, entries[1], "out"), { parent: null, at: 1 });
  assert.equal(bookmarkMove(entries, entries[3], "down"), null);
});

test("protection inputs enforce UTF-8 byte limits without silently truncating Unicode passwords", () => {
  const base: CopyProtection = {
    mode: "protect", openingPassword: "", ownerPassword: "owner",
    permissions: { print: "low", copy: false, edit: false, annotate: false, fillForms: true, assemble: false },
  };
  assert.doesNotThrow(() => validateCopyProtection(base));
  assert.doesNotThrow(() => validateCopyProtection({ mode: "unprotect" }));
  assert.doesNotThrow(() => validateCopyProtection({ ...base, ownerPassword: "é".repeat(63) }));
  assert.throws(() => validateCopyProtection({ ...base, ownerPassword: "é".repeat(64) }), /UTF-8/);
  assert.throws(() => validateCopyProtection({ ...base, openingPassword: "owner" }), /different/);
  assert.throws(() => validateCopyProtection({ ...base, ownerPassword: "" }), /nonempty/);
});
