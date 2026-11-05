import assert from "node:assert/strict";
import test from "node:test";
import { createPageLabelDraft, pageLabelChanges, pageLabelsPending, pageLabelText, pageDescription, resolvePageInput } from "../src/lib/pageLabels.ts";
import type { DocumentInfo } from "../src/lib/types.ts";

function document(): DocumentInfo {
  return { id: 7, revision: 3, name: "Synthetic labels.pdf", pageCount: 4, byteLength: 1000,
    dirty: false, canUndo: false, canRedo: false, readOnlyReason: null, passwordProtected: false, copyAllowed: true,
    permissions: { ownerAuthenticated: true, printAllowed: true, highQualityPrintAllowed: true, copyAllowed: true,
      modifyAllowed: true, annotateAllowed: true, fillFormsAllowed: true, assembleAllowed: true },
    annotations: [], pageLabels: null, pages: Array.from({ length: 4 }, (_, index) => ({
      key: `${index + 1}:0`, rotation: 0, geometry: { mediaBox: [0, 0, 420, 600], bounds: [0, 0, 420, 600],
        originalBounds: [0, 0, 420, 600], rotation: 0, userUnit: 1, widthPoints: 420, heightPoints: 600, cropModified: false },
    })) };
}

test("page-label drafts preserve source identity and become pending only after a change", () => {
  const info = document(), draft = createPageLabelDraft(info);
  assert.equal(pageLabelsPending(draft), false);
  assert.equal(draft.custom, false);
  draft.ranges[0].style = "roman_lower";
  draft.ranges.push({ page: "3", style: "decimal", prefix: "Body-", start: "1" });
  assert.equal(pageLabelsPending(draft), true);
  assert.deepEqual(pageLabelChanges(draft, info), [
    { page: 0, style: "roman_lower", prefix: "", start: 1 },
    { page: 2, style: "decimal", prefix: "Body-", start: 1 },
  ]);
  assert.throws(() => pageLabelChanges(draft, { ...info, revision: 4 }), /draft is retained/);
  assert.throws(() => pageLabelChanges(draft, { ...info, id: 8 }), /draft is retained/);
});

test("invalid sections, prefixes and numbering cannot produce a partial label change", () => {
  const info = document(), initial = createPageLabelDraft(info);
  for (const update of [
    { page: "0" }, { page: "5" }, { page: "2" }, { start: "0" }, { start: "" },
    { start: "4294967295" }, { prefix: "\n" }, { prefix: "é".repeat(257) },
  ]) {
    const draft = { ...initial, ranges: [{ ...initial.ranges[0], ...update }] };
    assert.throws(() => pageLabelChanges(draft, info));
    assert.equal(pageLabelsPending(draft), true);
  }
  assert.throws(() => pageLabelChanges({ ...initial, ranges: [] }, info));
  assert.throws(() => pageLabelChanges({ ...initial, ranges: [
    initial.ranges[0], { ...initial.ranges[0], page: "3" }, { ...initial.ranges[0], page: "2" },
  ] }, info), /ascending/);
  const prefixOnly = { ...initial, ranges: [{ ...initial.ranges[0], style: "prefix" as const, start: "", prefix: "Appendix" }] };
  assert.deepEqual(pageLabelChanges(prefixOnly, info), [{ page: 0, style: "prefix", prefix: "Appendix", start: 1 }]);
});

test("exact labels and physical positions use distinct navigation modes without reserved-prefix ambiguity", () => {
  const labels = ["i", "ii", "#2", "1"];
  assert.equal(resolvePageInput("ii", 4, labels), 1);
  assert.equal(resolvePageInput("#2", 4, labels), 2);
  assert.equal(resolvePageInput("1", 4, labels), 3);
  assert.equal(resolvePageInput("1", 4, null), 0);
  assert.equal(resolvePageInput("02", 4, null), 1);
  assert.throws(() => resolvePageInput("II", 4, labels), /case-sensitive/);
  assert.throws(() => resolvePageInput("ii", 4, null), /physical page/);
  assert.throws(() => resolvePageInput("5", 4, null));
  assert.throws(() => resolvePageInput("a", 2, ["a", "a"]), /more than one page/);
  assert.throws(() => resolvePageInput("i", 4, ["i"]), /do not match/);
});

test("readable page descriptions keep logical labels separate from physical page positions", () => {
  const info = document();
  info.pageLabels = { labels: ["i", "ii", "1", ""], ranges: [], issue: null };
  assert.equal(pageLabelText(info, 0), "i");
  assert.equal(pageDescription(info, 2), "page 1 (physical page 3)");
  assert.equal(pageLabelText(info, 3), "4");
  info.pageLabels = { labels: null, ranges: [], issue: "Malformed label tree; physical positions shown." };
  assert.equal(pageLabelText(info, 2), "3");
  assert.equal(pageDescription(info, 2), "page 3");
});
