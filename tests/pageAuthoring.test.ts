import test from "node:test";
import assert from "node:assert/strict";
import { assertAuthoringRevision, authoringEdit, authoringPages, authoringPending, createAuthoringDraft,
  fittedImage, imageDimension, imagePlacement } from "../src/lib/authoring.ts";
import type { DocumentInfo } from "../src/lib/types.ts";
import { intersectsPdfRect } from "../src/lib/geometry.ts";

const document: DocumentInfo = { id: 42, revision: 3, name: "Synthetic.pdf", pageCount: 3, byteLength: 1000,
  dirty: false, canUndo: false, canRedo: false, readOnlyReason: null, passwordProtected: false, copyAllowed: true, annotations: [],
  pages: [0, 90, 270].map((rotation, index) => ({ key: String(index), rotation, geometry: {
    mediaBox: [10, -20, 330, 440], bounds: [30, 10, 310, 410], originalBounds: [30, 10, 310, 410], rotation,
    userUnit: index === 1 ? 2 : 1, widthPoints: index === 0 ? 280 : index === 1 ? 800 : 400,
    heightPoints: index === 0 ? 400 : index === 1 ? 560 : 280, cropModified: false,
  } })),
};

test("authoring drafts retain explicit document revision/page targets and protect pending settings", () => {
  const draft = createAuthoringDraft("crop", document, 1);
  assert.equal(authoringPending(draft), false);
  assert.equal(authoringPending({ ...draft, dirty: true }), true);
  assert.deepEqual(authoringPages(draft, document), [1]);
  assert.deepEqual(authoringPages({ ...draft, scope: "all" }, document), [0, 1, 2]);
  assert.deepEqual(authoringPages({ ...draft, scope: "range", range: "3, 1-2" }, document), [2, 0, 1]);
  assert.throws(() => assertAuthoringRevision(draft, { ...document, revision: 4 }));
  assert.throws(() => assertAuthoringRevision(draft, { ...document, id: 43 }));
});

test("cropped pages keep partially visible overlays and exclude off-page links and note targets", () => {
  const bounds = [30, 10, 310, 410];
  assert(intersectsPdfRect([20, 20, 40, 40], bounds));
  assert(intersectsPdfRect([310, 400, 300, 390], bounds));
  assert(!intersectsPdfRect([0, 20, 30, 40], bounds));
  assert(!intersectsPdfRect([100, -20, 120, 10], bounds));
  assert(!intersectsPdfRect([320, 100, 340, 120], bounds));
});
test("image placement uses displayed physical points and preserves source proportions", () => {
  const draft = createAuthoringDraft("image", document, 1);
  assert.equal(draft.kind, "image");
  if (draft.kind !== "image") throw new Error("Expected image draft");
  const selected = fittedImage({ ...draft, asset: { assetId: 7, width: 2, height: 3 } }, document.pages[1].geometry);
  assert.equal(authoringPending(selected), true);
  assert.equal(selected.width, "160");
  assert.equal(selected.height, "240");
  assert.equal(selected.x, "320");
  assert.equal(selected.y, "160");
  assert.deepEqual(imagePlacement(selected, document.pages[1].geometry),
    { page: 1, placement: [320, 160, 160, 240], opacity: 1 });
  const resized = imageDimension(selected, "width", "120");
  assert.equal(resized.height, "180");
  assert.equal(imageDimension({ ...selected, lockAspect: false }, "width", "120").height, "240");
  assert.throws(() => imagePlacement({ ...selected, x: "799" }, document.pages[1].geometry));
  assert.throws(() => imagePlacement({ ...selected, opacity: "0" }, document.pages[1].geometry));
  assert.throws(() => imagePlacement({ ...selected, width: "" }, document.pages[1].geometry));
});

test("crop validates the actual smallest selected page and does not silently clamp margins", () => {
  const draft = createAuthoringDraft("crop", document, 0);
  if (draft.kind !== "crop") throw new Error("Expected crop draft");
  assert.deepEqual(authoringEdit({ ...draft, scope: "all", margins: ["10", "20", "30", "40"] }, document),
    { type: "crop", pages: [0, 1, 2], margins: [10, 20, 30, 40] });
  assert.deepEqual(authoringEdit(draft, document, "remove_crop"), { type: "remove_crop", pages: [0] });
  assert.deepEqual(authoringEdit(draft, document, "reset_crop"), { type: "reset_crop", pages: [0] });
  assert.throws(() => authoringEdit(draft, document));
  assert.throws(() => authoringEdit({ ...draft, scope: "all", margins: ["280", "0", "0", "0"] }, document));
  assert.throws(() => authoringEdit({ ...draft, margins: ["-1", "0", "0", "0"] }, document));
});

test("watermarks and page numbers have bounded explicit settings and selected-range order", () => {
  const watermark = createAuthoringDraft("watermark", document, 0);
  if (watermark.kind !== "watermark") throw new Error("Expected watermark draft");
  assert.equal(authoringEdit({ ...watermark, text: "Résumé", scope: "range", range: "3,1" }, document).type, "watermark");
  assert.deepEqual(authoringEdit(watermark, document, "remove_stamps"),
    { type: "remove_authoring", pages: [0], kind: "watermark" });
  assert.throws(() => authoringEdit(watermark, document));
  assert.throws(() => authoringEdit({ ...watermark, text: "Two\nlines" }, document));
  const numbers = createAuthoringDraft("numbers", document, 0);
  if (numbers.kind !== "numbers") throw new Error("Expected numbering draft");
  const edit = authoringEdit({ ...numbers, prefix: "Page ", suffix: " / 3", start: "7",
    scope: "range", range: "3, 1-2" }, document);
  assert.equal(edit.type, "page_numbers");
  if (edit.type !== "page_numbers") throw new Error("Expected page-number edit");
  assert.deepEqual(edit.pages, [2, 0, 1]);
  assert.equal(edit.settings.start, 7);
  assert.equal(edit.settings.position, "bottom_center");
  assert.throws(() => authoringEdit({ ...numbers, start: "1.5" }, document));
  assert.throws(() => authoringEdit({ ...numbers, scope: "all", start: "999999" }, document));
  assert.throws(() => authoringEdit({ ...numbers, prefix: "x".repeat(81) }, document));
});
