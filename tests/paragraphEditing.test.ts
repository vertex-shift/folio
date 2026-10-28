import test from "node:test";
import assert from "node:assert/strict";
import { assertParagraphDraft, newParagraphDraft, paragraphChange, paragraphDraftPending,
  paragraphGesture, paragraphPlacement, withParagraphPlacement } from "../src/features/paragraphEditing/model.ts";
import type { DocumentInfo } from "../src/lib/types.ts";
import type { ParagraphBlock } from "../src/features/paragraphEditing/types.ts";
import type { PageGeometry } from "../src/lib/authoring.ts";

const geometry: PageGeometry = { mediaBox: [-20, -30, 600, 800], bounds: [0, 10, 580, 780], originalBounds: [0, 10, 580, 780],
  rotation: 90, userUnit: 1.5, widthPoints: 1155, heightPoints: 870, cropModified: false };
const info = { id: 7, revision: 3, readOnlyReason: null, pages: [{ key: "5:0", rotation: 90, geometry }] } as DocumentInfo;
const block: ParagraphBlock = { id: "source-4-16", path: [4], text: "Genuine source paragraph.", sourceLines: [
  { text: "Genuine source", baseline: [60, 75], bounds: [60, 62, 90, 15] },
  { text: "paragraph.", baseline: [60, 92], bounds: [60, 80, 60, 15] },
], placement: [60, 60, 200, 80], rotation: 0, size: 12, leading: 17, alignment: "left",
  font: "NotoSans", fontMode: "original", originalFontAvailable: true, fontReason: null, editable: true, reason: null };
const draft = () => newParagraphDraft(info, 0, 12, block);

test("selection alone is not pending, but cleared text and invalid geometry remain real drafts", () => {
  const selected = draft();
  assert.equal(paragraphDraftPending(selected), false);
  for (const changed of [{ ...selected, text: "" }, { ...selected, text: " " }, { ...selected, x: "" },
    { ...selected, height: "bad" }, { ...selected, size: "" }, { ...selected, leading: "18" },
    { ...selected, font: "noto" as const }, { ...selected, alignment: "right" as const }]) {
    assert.equal(paragraphDraftPending(changed), true);
  }
  assert.equal(paragraphDraftPending({ ...selected, size: "12.0", width: "200.00" }), false);
  assert.equal(paragraphChange(info, { ...selected, text: "" }).text, "");
});

test("a paragraph mutation retains exact source identity, epoch, explicit whitespace and orientation contract", () => {
  const selected = { ...draft(), text: "  Café  beta\n\nlast line ", font: "noto" as const, size: "14", leading: "20" };
  const change = paragraphChange(info, selected);
  assert.deepEqual(change, { epoch: 12, page: 0, path: [4], blockId: block.id, expected: block.text,
    text: selected.text, placement: [60, 60, 200, 80], size: 14, leading: 20, alignment: "left", font: "noto" });
  assert.equal(selected.source.rotation, 0, "box coordinates must not be reinterpreted as raw rotated PDF space");
});

test("stale, read-only and unsupported source drafts never become source mutations", () => {
  assert.throws(() => assertParagraphDraft({ ...info, id: 8 }, draft()), /PDF changed/);
  assert.throws(() => paragraphChange({ ...info, revision: 4 }, draft()), /PDF changed/);
  assert.throws(() => paragraphChange({ ...info, readOnlyReason: "Signed source is read-only." }, draft()), /Signed source/);
  assert.throws(() => paragraphChange(info, { ...draft(), source: { ...block, editable: false, reason: "ActualText is read-only." } }), /ActualText/);
  assert.throws(() => paragraphChange(info, { ...draft(), text: "x".repeat(8001) }), /8,000/);
});

test("numeric physical boxes and typography are validated without guessing invalid drafts", () => {
  assert.deepEqual(paragraphPlacement(draft(), geometry), [60, 60, 200, 80]);
  for (const changed of [{ x: "-1" }, { y: "" }, { width: "NaN" }, { height: "0" }, { width: "2000" }]) {
    assert.throws(() => paragraphPlacement({ ...draft(), ...changed }, geometry));
  }
  for (const changed of [{ size: "" }, { size: "1" }, { size: "145" }, { leading: "0" }, { leading: "Infinity" }]) {
    assert.throws(() => paragraphChange(info, { ...draft(), ...changed }));
  }
});

test("dragging and keyboard resizing preserve physical dimensions and stay within the rotated crop", () => {
  const start: [number, number, number, number] = [60, 60, 200, 80];
  assert.deepEqual(paragraphGesture(start, -100, 1000, false, geometry), [0, 790, 200, 80]);
  assert.deepEqual(paragraphGesture(start, 2000, -200, true, geometry), [60, 60, 1095, 0.1]);
  const updated = withParagraphPlacement(draft(), [40.12345, 50.56789, 234.98765, 98.12345]);
  assert.deepEqual([updated.x, updated.y, updated.width, updated.height], ["40.123", "50.568", "234.988", "98.123"]);
  assert.equal(updated.text, block.text);
  assert.equal(updated.epoch, 12);
});
