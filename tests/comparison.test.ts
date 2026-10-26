import assert from "node:assert/strict";
import test from "node:test";
import {
  alignedPages, assertComparisonCurrent, comparePixels, compareText, ComparisonBudget, encodeResults,
  geometryChanges, LIMITS, pageChanged, pagePresence, textTokens, validateSettings,
} from "../src/features/comparison/model.ts";
import type { ComparisonPlan, PageResult, SourceState } from "../src/features/comparison/model.ts";
import type { PageGeometry } from "../src/lib/authoring.ts";
import type { DocumentInfo } from "../src/lib/types.ts";

const geometry: PageGeometry = { mediaBox: [-20, -10, 400, 250], bounds: [10, 20, 350, 220],
  originalBounds: [10, 20, 350, 220], rotation: 90, userUnit: 2, widthPoints: 400, heightPoints: 680, cropModified: false };
const blankPage: PageResult = { index: 0, text: { beforeTokens: 0, afterTokens: 0, spans: [] },
  pixels: { width: 1, height: 1, changed: 0, total: 1, maxChannelDelta: 0, bounds: null } };
const source = (id: number, count = 1) => ({ documentId: id, revision: 3, name: `${id}.pdf`,
  pageCount: count, sourceBytes: 1000, passwordProtected: false, pages: Array.from({ length: count }, () => geometry) });
const plan: ComparisonPlan = { comparisonId: 1, before: source(1), after: source(2),
  settings: { dpi: 96, tolerance: 0 }, pages: [{ index: 0, width: 534, height: 907 }], scope: "bounded", suggestedName: "report.json" };
const document = (id: number): DocumentInfo => ({
  id, revision: 3, name: `${id}.pdf`, pageCount: 1, pages: [{ key: "1:0", rotation: 90, geometry }],
  byteLength: 1000, dirty: true, canUndo: true, canRedo: false, readOnlyReason: "Digitally signed PDF: read-only.",
  copyAllowed: true, passwordProtected: true, annotations: [],
  permissions: { copyAllowed: true, printAllowed: true, highQualityPrintAllowed: true,
    fillFormsAllowed: false, ownerAuthenticated: false, modifyAllowed: false, annotateAllowed: false, assembleAllowed: false },
});

test("text differences preserve extracted Unicode, punctuation, whitespace and line endings", () => {
  const result = compareText("Before café.\nCost 10", "After café!\nCost 12");
  const before = result.spans.filter((span) => span.kind !== "added").map((span) => span.text).join("");
  const after = result.spans.filter((span) => span.kind !== "removed").map((span) => span.text).join("");
  assert.equal(before, "Before café.\nCost 10");
  assert.equal(after, "After café!\nCost 12");
  assert.ok(result.spans.some((span) => span.kind === "removed" && span.text.includes("10")));
  assert.ok(result.spans.some((span) => span.kind === "added" && span.text.includes("12")));
  assert.ok(compareText("a b", "a  b").spans.some((span) => span.kind !== "equal"));
  assert.deepEqual(compareText("", ""), { beforeTokens: 0, afterTokens: 0, spans: [] });
});

test("text byte/token boundaries are exact and never truncate", () => {
  assert.equal(textTokens("é".repeat(LIMITS.pageTextBytes / 2)).length, 1);
  assert.throws(() => textTokens("é".repeat(LIMITS.pageTextBytes / 2) + "a"), /256 KiB/);
  assert.equal(textTokens("a ".repeat(LIMITS.pageTokens / 2)).length, LIMITS.pageTokens);
  assert.throws(() => textTokens("a ".repeat(LIMITS.pageTokens / 2) + "a"), /20,000/);
  assert.throws(() => compareText("a ".repeat(3000), "b ".repeat(3000)), /complexity\/time/);
});

test("pixel comparison uses RGB max-channel deltas strictly above tolerance, not percentage guesses", () => {
  const before = new Uint8ClampedArray([255, 255, 255, 255, 0, 10, 20, 255, 7, 8, 9, 255, 0, 0, 0, 255]);
  const after = new Uint8ClampedArray([251, 255, 255, 255, 0, 15, 20, 255, 7, 8, 9, 255, 12, 0, 0, 255]);
  const result = comparePixels(before, after, 2, 2, 4);
  assert.deepEqual(result.pixels, { width: 2, height: 2, total: 4, changed: 2, maxChannelDelta: 12, bounds: [1, 0, 1, 2] });
  assert.equal(result.mask[3], 0);
  assert.equal(result.mask[7], 190);
  assert.equal(comparePixels(before, after, 2, 2, 12).pixels.changed, 0);
  assert.equal(comparePixels(before, after, 2, 2, 0).pixels.changed, 3);
  assert.deepEqual(Array.from(before), [255, 255, 255, 255, 0, 10, 20, 255, 7, 8, 9, 255, 0, 0, 0, 255]);
  assert.throws(() => comparePixels(before, after.subarray(4), 2, 2, 0), /invalid/);
  assert.throws(() => comparePixels(before, after, 2, 2, -1), /invalid/);
});

test("alignment preserves physical rotation/crop/UserUnit dimensions and reports geometry independently of pixels", () => {
  assert.deepEqual(alignedPages([geometry], [geometry], { dpi: 150, tolerance: 0 }),
    [{ index: 0, width: 834, height: 1417 }]);
  assert.deepEqual(geometryChanges(geometry, { ...geometry }), []);
  assert.deepEqual(geometryChanges(geometry, { ...geometry, rotation: 270, userUnit: 1, bounds: [0, 0, 400, 250] }),
    ["effective_crop", "rotation", "user_unit"]);
  const samePixelsDifferentGeometry = { ...plan, after: { ...source(2), pages: [{ ...geometry, rotation: 270 }] } };
  assert.equal(pageChanged(samePixelsDifferentGeometry, blankPage), true);
  assert.equal(pageChanged(plan, blankPage), false);
  assert.equal(pagePresence({ ...plan, after: source(2, 2) }, 1), "added");
  assert.equal(pagePresence({ ...plan, before: source(1, 2) }, 1), "removed");
});

test("page, union-pixel and total-pixel budgets are inclusive with no silent downsampling", () => {
  const size = (widthPoints: number, heightPoints: number) => ({ ...geometry, widthPoints, heightPoints });
  const settings = { dpi: 72, tolerance: 0 } as const;
  assert.equal(alignedPages([size(4000, 2000)], [size(4000, 2000)], settings)[0].width, 4000);
  assert.throws(() => alignedPages([size(4000.01, 2000)], [size(4000, 2000)], settings), /8 megapixels/);
  assert.equal(alignedPages([size(8192, 1)], [size(1, 1)], settings)[0].width, 8192);
  assert.throws(() => alignedPages([size(8192.01, 1)], [size(1, 1)], settings), /8,192/);
  assert.throws(() => alignedPages([size(4000, 1000)], [size(1000, 4000)], settings), /8 megapixels/);
  assert.equal(alignedPages(Array(100).fill(size(1000, 1000)), [size(1, 1)], settings).length, 100);
  assert.throws(() => alignedPages(Array(101).fill(size(1, 1)), [size(1, 1)], settings), /1-100/);
  const total = Array(13).fill(size(4000, 2000));
  total[12] = size(2000, 2000);
  assert.equal(alignedPages(total, [size(1, 1)], settings).length, 13);
  total[12] = size(2000.01, 2000);
  assert.throws(() => alignedPages(total, [size(1, 1)], settings), /100 megapixels/);
  for (const value of [NaN, Infinity, -1, 0]) assert.throws(() => alignedPages([size(value, 10)], [geometry], settings));
  assert.throws(() => validateSettings({ dpi: 96, tolerance: 65 }));
  assert.doesNotThrow(() => validateSettings({ dpi: 150, tolerance: 64 }));
});

test("global text/token/span/preview budgets accept exact limits then reject one unit over", () => {
  const bytes = new ComparisonBudget(); bytes.addTextBytes(LIMITS.textBytes);
  assert.throws(() => bytes.addTextBytes(1), /2 MiB/);
  const tokens = new ComparisonBudget(); tokens.addText({ beforeTokens: LIMITS.tokens, afterTokens: 0, spans: [] });
  assert.throws(() => tokens.addText({ beforeTokens: 1, afterTokens: 0, spans: [] }), /100,000/);
  const spans = new ComparisonBudget();
  spans.addText({ beforeTokens: 0, afterTokens: 0, spans: Array(LIMITS.spans).fill({ kind: "equal", text: "a" }) });
  assert.throws(() => spans.addText({ beforeTokens: 0, afterTokens: 0, spans: [{ kind: "equal", text: "a" }] }), /4,000/);
  const previews = new ComparisonBudget(); previews.addPreview(LIMITS.previewBytes);
  assert.throws(() => previews.addPreview(1), /32 MiB/);
});

test("report transport is bounded by encoded UTF-8 bytes including JSON overhead", () => {
  const page: PageResult = { ...blankPage, text: { beforeTokens: 1, afterTokens: 1, spans: [{ kind: "equal", text: "" }] } };
  const base = encodeResults([page]).length;
  page.text.spans[0].text = "a".repeat(LIMITS.reportBytes - base);
  assert.equal(encodeResults([page]).length, LIMITS.reportBytes);
  page.text.spans[0].text += "a";
  assert.throws(() => encodeResults([page]), /4 MiB/);
});

test("both selected sources must stay open/current and draft-free; signed read-only and protected copy-permitted access are allowed", () => {
  const sources = new Map<number, SourceState>([1, 2].map((id) => [id, { document: document(id), pending: false, busy: false }]));
  const current = (id: number) => sources.get(id) ?? { document: null, pending: false, busy: false };
  assert.doesNotThrow(() => assertComparisonCurrent(plan, current));
  for (const id of [1, 2]) {
    sources.get(id)!.pending = true;
    assert.throws(() => assertComparisonCurrent(plan, current), /pending writing, form, metadata/);
    sources.get(id)!.pending = false; sources.get(id)!.busy = true;
    assert.throws(() => assertComparisonCurrent(plan, current), /busy/);
    sources.get(id)!.busy = false; sources.get(id)!.document!.revision++;
    assert.throws(() => assertComparisonCurrent(plan, current), /changed or closed/);
    sources.get(id)!.document!.revision--;
    sources.get(id)!.document!.permissions.copyAllowed = false;
    assert.throws(() => assertComparisonCurrent(plan, current), /copying\/extraction/);
    sources.get(id)!.document!.permissions.copyAllowed = true;
    const state = sources.get(id)!; sources.delete(id);
    assert.throws(() => assertComparisonCurrent(plan, current), /changed or closed/);
    sources.set(id, state);
  }
});
