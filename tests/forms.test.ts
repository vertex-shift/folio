import assert from "node:assert/strict";
import test from "node:test";
import { equalFormValue, formChanges, updateFormDraft, validateFormValue } from "../src/lib/forms.ts";
import type { FormField, FormSnapshot, FormValue } from "../src/lib/formsTypes.ts";

function textField(overrides: Partial<FormField> = {}): FormField {
  return {
    id: "12:0", name: "person.name", label: "Full name", kind: "text",
    readOnly: false, reason: null, required: false, multiline: false, password: false,
    comb: false, maxLength: 40, multiple: false, combo: false, editableChoice: false, noToggleToOff: false,
    options: [], value: { type: "text", text: "Original" }, widgets: [{ page: 0, rect: [10, 10, 100, 40] }],
    ...overrides,
  };
}

function snapshot(fields: FormField[] = [textField()]): FormSnapshot {
  return { documentId: 1, revision: 3, form: { fields, fillable: true, reason: null, warnings: [] } };
}

test("form values compare their real types, export states, indices, and custom values", () => {
  assert.ok(equalFormValue({ type: "text", text: "" }, { type: "text", text: "" }));
  assert.ok(!equalFormValue({ type: "text", text: "" }, null));
  assert.ok(!equalFormValue({ type: "text", text: "" }, { type: "button", state: null }));
  assert.ok(equalFormValue({ type: "button", state: "4163636570746564" }, { type: "button", state: "4163636570746564" }));
  assert.ok(!equalFormValue({ type: "button", state: "01" }, { type: "button", state: "02" }));
  assert.ok(equalFormValue({ type: "choice", indices: [0, 2], custom: null }, { type: "choice", indices: [0, 2], custom: null }));
  assert.ok(!equalFormValue({ type: "choice", indices: [0, 2], custom: null }, { type: "choice", indices: [2, 0], custom: null }));
  assert.ok(!equalFormValue({ type: "choice", indices: [], custom: "" }, { type: "choice", indices: [], custom: null }));
});

test("workspace-level drafts keep empty replacements, survive unrelated panel/tab work, and clear only when reverted", () => {
  const first = snapshot();
  const draft = updateFormDraft(first, null, "12:0", { type: "text", text: "" });
  assert.deepEqual(draft, { documentId: 1, revision: 3, values: { "12:0": { type: "text", text: "" } } });
  const other = { ...snapshot(), documentId: 2 };
  const otherDraft = updateFormDraft(other, null, "12:0", { type: "text", text: "Other tab" });
  assert.equal(otherDraft?.documentId, 2);
  assert.deepEqual(formChanges(first, draft, { id: 1, revision: 3 }), {
    changes: [{ id: "12:0", value: { type: "text", text: "" } }], errors: {},
  });
  assert.equal(updateFormDraft(first, draft, "12:0", { type: "text", text: "Original" }), null);
  assert.equal(draft?.values["12:0"].type, "text");
});

test("applying never crosses document identities or revisions, and keeps stale drafts for explicit review", () => {
  const source = snapshot();
  const draft = updateFormDraft(source, null, "12:0", { type: "text", text: "Keep my work" });
  for (const current of [{ id: 2, revision: 3 }, { id: 1, revision: 4 }]) {
    assert.throws(() => formChanges(source, draft, current), /kept for review/);
  }
  assert.throws(() => updateFormDraft({ ...source, revision: 4 }, draft, "12:0", { type: "text", text: "Next" }), /document changed/);
  assert.deepEqual(draft?.values["12:0"], { type: "text", text: "Keep my work" });
  assert.throws(() => formChanges(source, null, { id: 1, revision: 3 }), /before applying/);
});

test("required, multiline, password, and Unicode character limits are checked without exposing values", () => {
  const required = textField({ required: true });
  assert.equal(validateFormValue(required, { type: "text", text: " " }), "This required field cannot be cleared.");
  assert.equal(validateFormValue(textField({ maxLength: 2, password: true }), { type: "text", text: "🔑🔑" }), null);
  assert.match(validateFormValue(textField({ maxLength: 2 }), { type: "text", text: "abc" })!, /at most 2/);
  assert.match(validateFormValue(textField(), { type: "text", text: "first\nsecond" })!, /one line/);
  assert.equal(validateFormValue(textField({ multiline: true }), { type: "text", text: "first\r\nsecond" }), null);
  assert.match(validateFormValue(textField({ multiline: true }), { type: "text", text: "tab\tvalue" })!, /control/);
  const secret = "private-password-value";
  const error = validateFormValue(textField({ maxLength: 2, password: true }), { type: "text", text: secret });
  assert.ok(error && !error.includes(secret));
});

test("button states are opaque exports, read-only/signature fields cannot become text, and radio no-toggle is enforced", () => {
  const button = textField({ kind: "checkbox", value: { type: "button", state: null }, options: [{ id: "596573", label: "Yes", exportValue: "Yes" }] });
  assert.equal(validateFormValue(button, { type: "button", state: "596573" }), null);
  assert.equal(validateFormValue(button, { type: "button", state: "Yes" }), "Choose an existing export state.");
  assert.match(validateFormValue({ ...button, kind: "radio", noToggleToOff: true }, { type: "button", state: null })!, /requires a selection/);
  const locked = textField({ readOnly: true, reason: "Locked by PDF" });
  assert.throws(() => updateFormDraft(snapshot([locked]), null, locked.id, { type: "text", text: "new" }), /Locked by PDF/);
  assert.match(validateFormValue(textField({ kind: "signature", value: null }), { type: "text", text: "not signing" })!, /does not match/);
});

test("choice drafts persist option indices rather than display labels, and reject duplicate/invalid selections", () => {
  const choice = textField({
    kind: "choice", multiple: true,
    value: { type: "choice", indices: [0], custom: null },
    options: [{ id: "0", label: "Canada", exportValue: "CA" }, { id: "1", label: "United States", exportValue: "US" }],
  });
  const source = snapshot([choice]);
  const value: FormValue = { type: "choice", indices: [0, 1], custom: null };
  const draft = updateFormDraft(source, null, choice.id, value);
  assert.deepEqual(formChanges(source, draft, { id: 1, revision: 3 }).changes, [{ id: choice.id, value }]);
  for (const indices of [[0, 0], [1, 0], [-1], [2], [NaN]]) {
    assert.ok(validateFormValue(choice, { type: "choice", indices, custom: null }));
  }
  assert.ok(validateFormValue(choice, { type: "choice", indices: [], custom: "Custom" }));
  assert.equal(validateFormValue({ ...choice, combo: true, multiple: false, editableChoice: true }, {
    type: "choice", indices: [], custom: "Custom",
  }), null);
  assert.match(validateFormValue({ ...choice, combo: true, multiple: false, editableChoice: true,
    options: choice.options.map((option) => ({ ...option, exportValue: "same" })),
  }, { type: "choice", indices: [], custom: "same" })!, /duplicate export/);
});

test("apply reports field-specific validation and never mutates or clears the draft on failure", () => {
  const source = snapshot([textField({ required: true }), textField({ id: "13:0", name: "person.notes" })]);
  const first = updateFormDraft(source, null, "12:0", { type: "text", text: "" });
  const draft = updateFormDraft(source, first, "13:0", { type: "text", text: "Valid draft" });
  const result = formChanges(source, draft, { id: 1, revision: 3 });
  assert.deepEqual(result.errors, { "12:0": "This required field cannot be cleared." });
  assert.deepEqual(result.changes, [{ id: "13:0", value: { type: "text", text: "Valid draft" } }]);
  assert.deepEqual(draft?.values["12:0"], { type: "text", text: "" });
  assert.deepEqual(draft?.values["13:0"], { type: "text", text: "Valid draft" });
});
