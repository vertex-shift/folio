import test from "node:test";
import assert from "node:assert/strict";
import { assertImageDraft, draftPlacement, gesturePlacement, imageChange, imageDraftPending, imageGeometryChanged, inspectionKeys, newImageDraft, withPlacement } from "../src/features/imageEditing/model.ts";
import type { ImageOccurrence } from "../src/features/imageEditing/types.ts";
import type { DocumentInfo } from "../src/lib/types.ts";

const info: DocumentInfo = {
  id: 42, revision: 7, name: "Synthetic images.pdf", pageCount: 1, byteLength: 1000, dirty: false,
  canUndo: false, canRedo: false, readOnlyReason: null, passwordProtected: false, copyAllowed: true, annotations: [],
  permissions: { ownerAuthenticated: true, printAllowed: true, highQualityPrintAllowed: true, copyAllowed: true,
    modifyAllowed: true, annotateAllowed: true, fillFormsAllowed: true, assembleAllowed: true },
  pages: [{ key: "2:0", rotation: 90, geometry: { mediaBox: [-10, -20, 400, 600], bounds: [20, 30, 380, 570],
    originalBounds: [20, 30, 380, 570], userUnit: 2, rotation: 90, widthPoints: 1080, heightPoints: 720, cropModified: false } }],
};
const source: ImageOccurrence = { path: [4, 2, 8], name: "SharedImage", pixelWidth: 3, pixelHeight: 2,
  placement: [20.123456, 30, 160, 90], editable: true, reason: null };

test("selecting source images preserves exact footprint without making an accidental placement draft", () => {
  const draft = newImageDraft(info, 0, source);
  assert.equal(draft.x, "20.123456");
  assert.equal(imageDraftPending(draft), false);
  assert.equal(imageGeometryChanged({ ...draft, y: "30.00" }), false);
  assert.equal(imageDraftPending({ ...draft, width: "" }), true);
  assert.equal(imageDraftPending({ ...draft, remove: true }), true);
  assert.throws(() => imageChange(info, draft), /Choose a replacement/);
  assert.throws(() => newImageDraft(info, 2, source), /page/);
});

test("inspection caches retain only eight recent page snapshots and do not merge revisions or documents", () => {
  let keys: string[] = [];
  for (let page = 0; page < 20; page++) keys = inspectionKeys(keys, `42:7:0:${page}`);
  assert.deepEqual(keys, Array.from({ length: 8 }, (_, index) => `42:7:0:${index + 12}`));
  keys = inspectionKeys(keys, "42:7:0:12");
  assert.equal(keys.at(-1), "42:7:0:12");
  assert.equal(keys.length, 8);
  keys = inspectionKeys(keys, "43:7:0:12");
  assert.ok(keys.includes("42:7:0:12") && keys.includes("43:7:0:12"));
  keys = inspectionKeys(keys, "42:8:1:12");
  assert.equal(keys.at(-1), "42:8:1:12");
});

test("replacement and removal target the exact nested invocation, not a global image resource", () => {
  const draft = newImageDraft(info, 0, source);
  const replacement = { assetId: 12, width: 2, height: 3, name: "Synthetic portrait.png", previewUrl: "blob:synthetic" };
  assert.deepEqual(imageChange(info, { ...draft, replacement }), { action: "replace", page: 0, path: [4, 2, 8], assetId: 12, placement: null });
  assert.deepEqual(imageChange(info, { ...draft, replacement, x: "40" }),
    { action: "replace", page: 0, path: [4, 2, 8], assetId: 12, placement: [40, 30, 160, 90] });
  assert.deepEqual(imageChange(info, { ...draft, replacement, remove: true, width: "" }), { action: "remove", page: 0, path: [4, 2, 8] });
  const tiny = newImageDraft(info, 0, { ...source, placement: [20, 30, 0.05, 0.04] });
  assert.deepEqual(imageChange(info, { ...tiny, replacement }), { action: "replace", page: 0, path: [4, 2, 8], assetId: 12, placement: null });
  assert.throws(() => imageChange(info, { ...tiny, x: "21" }), /at least 0.1 pt/);
});

test("image placements validate actual rotated physical bounds instead of silently coercing invalid fields", () => {
  const draft = newImageDraft(info, 0, source), geometry = info.pages[0].geometry;
  assert.deepEqual(draftPlacement(withPlacement(draft, [40, 60, 100, 80]), geometry), [40, 60, 100, 80]);
  assert.deepEqual(imageChange(info, { ...draft, y: "40" }), { action: "place", page: 0, path: [4, 2, 8], placement: [20.123456, 40, 160, 90] });
  for (const patch of [{ width: "" }, { width: "0.09" }, { height: "-1" }, { x: "-1" },
    { y: "Infinity" }, { x: "1000" }, { y: "700" }, { width: "NaN" }]) {
    assert.throws(() => imageChange(info, { ...draft, ...patch }));
  }
});

test("stale and noneditable image drafts remain protected without guessing a mutation", () => {
  const draft = { ...newImageDraft(info, 0, source), x: "40" };
  assert.throws(() => assertImageDraft({ ...info, id: 43 }, draft), /PDF changed/);
  assert.throws(() => imageChange({ ...info, revision: 8 }, draft), /PDF changed/);
  assert.throws(() => imageChange({ ...info, readOnlyReason: "Signed source is read-only." }, draft), /Signed source/);
  const unsupported = { ...source, editable: false, reason: "Stencil masks are read-only." };
  assert.throws(() => imageChange(info, { ...newImageDraft(info, 0, unsupported), remove: true }), /Stencil masks/);
  const unplaced = newImageDraft(info, 0, { ...unsupported, placement: null });
  assert.equal(imageDraftPending(unplaced), false);
});

test("move and corner-resize gestures clamp only their live interaction to the visible physical page", () => {
  const geometry = info.pages[0].geometry;
  assert.deepEqual(gesturePlacement([20, 30, 160, 90], 10, -20, false, geometry), [30, 10, 160, 90]);
  assert.deepEqual(gesturePlacement([20, 30, 160, 90], -200, 2000, false, geometry), [0, 630, 160, 90]);
  assert.deepEqual(gesturePlacement([20, 30, 160, 90], 20, -10, true, geometry), [20, 30, 180, 80]);
  assert.deepEqual(gesturePlacement([20, 30, 160, 90], 2000, -2000, true, geometry), [20, 30, 1060, 0.1]);
});
