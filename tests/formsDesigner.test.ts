import assert from "node:assert/strict";
import test from "node:test";
import { buttonState, designChanges, editDesignerDraft, newDesignerDraft, rectToPlacement, widgetPlacement } from "../src/features/formsDesigner/model.ts";
import type { DesignerSnapshot } from "../src/features/formsDesigner/types.ts";
import type { DocumentInfo } from "../src/lib/types.ts";
import type { PageGeometry } from "../src/lib/authoring.ts";

function geometry(rotation = 0, userUnit = 1.5): PageGeometry {
  const width = 420 * userUnit, height = 540 * userUnit;
  return { mediaBox: [-80, -40, 640, 800], bounds: [-30, 70, 390, 610], originalBounds: [-30, 70, 390, 610],
    rotation, userUnit, widthPoints: rotation % 180 ? height : width, heightPoints: rotation % 180 ? width : height, cropModified: false };
}
function info(): DocumentInfo {
  return { id: 4, revision: 3, name: "Synthetic.pdf", pageCount: 4, byteLength: 1000, dirty: false, canUndo: false, canRedo: false,
    readOnlyReason: null, passwordProtected: false, copyAllowed: true, annotations: [],
    permissions: { ownerAuthenticated: true, printAllowed: true, highQualityPrintAllowed: true, copyAllowed: true, modifyAllowed: true, annotateAllowed: true, fillFormsAllowed: true, assembleAllowed: true },
    pages: [0, 90, 180, 270].map((rotation) => ({ key: `${rotation + 1}:0`, rotation, geometry: geometry(rotation) })) };
}
function snapshot(): DesignerSnapshot {
  return { documentId: 4, revision: 3, designer: { authorable: true, reason: null, warnings: [], fields: [] } };
}

test("creation contracts use canonical types, opaque button states and page-specific placements", () => {
  const source = info(), current = snapshot();
  const draft = newDesignerDraft(source, 2, "radio", current);
  const changes = designChanges(source, current, draft);
  assert.equal(changes[0].type, "create");
  if (changes[0].type !== "create") return;
  assert.equal(changes[0].widgets.length, 2);
  assert.equal(changes[0].widgets[0].page, 2);
  assert.equal(changes[0].widgets[0].option, 0);
  assert.equal(changes[0].widgets[1].option, 1);
  assert.equal(buttonState("Yes"), "596573");
  assert.equal(draft.dirty, false);
  assert.equal(draft.revision, source.revision);
});

test("inverse PDF geometry respects all four rotations, offset crops and physical UserUnit", () => {
  const rect: [number, number, number, number] = [0, 100, 120, 130];
  const expected = [
    [45, 720, 180, 45], [45, 45, 45, 180], [405, 45, 180, 45], [720, 405, 45, 180],
  ];
  [0, 90, 180, 270].forEach((rotation, index) => assert.deepEqual(rectToPlacement(rect, geometry(rotation)), expected[index]));
});

test("widget placement rejects invalid numbers, old/out-of-range pages, cropped values and undersized PDF units", () => {
  const source = info(), draft = newDesignerDraft(source, 0, "text", snapshot());
  const widget = draft.widgets[0];
  assert.equal(widgetPlacement(widget, source).page, 0);
  for (const invalid of [{ page: "0" }, { page: "5" }, { x: "" }, { x: "NaN" }, { width: "1" }, { y: "800" }, { height: "Infinity" }]) {
    assert.throws(() => widgetPlacement({ ...widget, ...invalid }, source));
  }
});

test("stale and cross-document design drafts fail without destroying user settings", () => {
  const source = info(), current = snapshot(), draft = newDesignerDraft(source, 1, "text", current);
  draft.definition.name = "preserve-my-input";
  draft.definition.defaultValue = { type: "text", text: "Keep my value" };
  draft.dirty = true;
  for (const other of [{ ...source, id: 9 }, { ...source, revision: 4 }]) {
    assert.throws(() => designChanges(other, current, draft), /changed/);
  }
  assert.throws(() => designChanges(source, current, { ...draft, revision: 0 }), /kept/);
  assert.equal(draft.definition.name, "preserve-my-input");
  assert.deepEqual(draft.definition.defaultValue, { type: "text", text: "Keep my value" });
});

test("existing logical IDs and widget IDs remain separate across property and single-widget edits", () => {
  const source = info(), current = snapshot();
  const definition = newDesignerDraft(source, 0, "text", current).definition;
  current.designer.fields.push({
    editable: true, reason: null, definition: { ...definition, maxLength: 40, fontSize: 10 },
    field: { id: "12:0", name: "person.name", label: "Full name", kind: "text", readOnly: false, reason: null,
      required: false, multiline: false, password: false, comb: false, maxLength: 40, multiple: false, combo: false, editableChoice: false,
      noToggleToOff: false, options: [], value: { type: "text", text: "Current" },
      widgets: [{ id: "13:0", page: 0, rect: [0, 100, 120, 130] }, { id: "14:0", page: 1, rect: [0, 100, 120, 130] }] },
  });
  const draft = editDesignerDraft(source, current, "12:0");
  draft.definition.label = "New label";
  draft.widgets[1].x = "60";
  const changes = designChanges(source, current, draft);
  assert.deepEqual(changes.map((change) => change.type), ["update", "place_widget"]);
  assert.equal(changes[1].type === "place_widget" && changes[1].widgetId, "14:0");
  assert.equal(changes[1].type === "place_widget" && changes[1].id, "12:0");
  assert.equal(draft.widgets[0].id, "13:0");
  draft.removedWidgets.push("13:0");
  draft.widgets.shift();
  assert.equal(designChanges(source, current, draft).at(-1)?.type, "remove_widget");
  draft.deleteField = true;
  assert.deepEqual(designChanges(source, current, draft), [{ type: "remove_field", id: "12:0" }]);
});

test("invalid numeric or naming drafts remain intact rather than defaulting silently", () => {
  const source = info(), current = snapshot(), draft = newDesignerDraft(source, 0, "text", current);
  draft.definition.maxLength = "";
  assert.throws(() => designChanges(source, current, draft), /Maximum characters/);
  assert.equal(draft.definition.maxLength, "");
  draft.definition.maxLength = "4000";
  draft.definition.name = "invalid.partial";
  assert.throws(() => designChanges(source, current, draft), /partial field name/);
  assert.equal(draft.definition.name, "invalid.partial");
});
