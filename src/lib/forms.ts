import type { FormChange, FormDraft, FormField, FormSnapshot, FormValue } from "./formsTypes";

export function equalFormValue(left: FormValue | null, right: FormValue | null): boolean {
  if (left === null || right === null) return left === right;
  if (left.type === "text" && right.type === "text") return left.text === right.text;
  if (left.type === "button" && right.type === "button") return left.state === right.state;
  if (left.type === "choice" && right.type === "choice") {
    return left.custom === right.custom && left.indices.length === right.indices.length
      && left.indices.every((value, index) => value === right.indices[index]);
  }
  return false;
}

export function updateFormDraft(snapshot: FormSnapshot, draft: FormDraft | null, id: string, value: FormValue): FormDraft | null {
  if (draft && (draft.documentId !== snapshot.documentId || draft.revision !== snapshot.revision)) {
    throw new Error("The document changed. Keep your form drafts for review, or explicitly discard them before reloading.");
  }
  const field = snapshot.form.fields.find((field) => field.id === id);
  if (!field || field.readOnly || !snapshot.form.fillable || field.value === null) {
    throw new Error(field?.reason ?? snapshot.form.reason ?? "This form field is not fillable.");
  }
  const values = { ...draft?.values };
  if (equalFormValue(field.value, value)) delete values[id];
  else values[id] = value;
  return Object.keys(values).length ? { documentId: snapshot.documentId, revision: snapshot.revision, values } : null;
}

export function validateFormValue(field: FormField, value: FormValue): string | null {
  if (field.readOnly) return field.reason ?? "This field is read-only.";
  if (value.type === "text" && field.kind === "text"
    || value.type === "choice" && field.kind === "choice" && value.custom !== null) {
    const text = value.type === "text" ? value.text : value.custom!;
    if (Array.from(text.replace(/\r\n?/g, "\n")).length > field.maxLength) {
      return `Use at most ${field.maxLength.toLocaleString()} characters.`;
    }
    if (field.required && !text.trim()) return "This required field cannot be cleared.";
    const multiline = field.kind === "text" && field.multiline;
    if (Array.from(text.replace(/\r\n?/g, "\n")).some((character) => {
      const code = character.codePointAt(0)!;
      return (code < 32 || code >= 127 && code <= 159) && !(multiline && character === "\n");
    })) return multiline ? "Only line breaks, not other control characters, are supported." : "Use one line without control characters.";
    if (value.type === "choice" && (!field.editableChoice || value.indices.length)) return "Choose an option or enter custom combo-box text, not both.";
    if (value.type === "choice" && field.options.filter((option) => option.exportValue === value.custom).length > 1) {
      return "Custom text matches duplicate export values. Choose an explicit option instead.";
    }
    return null;
  }
  if (value.type === "button" && (field.kind === "checkbox" || field.kind === "radio")) {
    if (value.state !== null && !field.options.some((option) => option.id === value.state)) return "Choose an existing export state.";
    if (value.state === null && (field.required || field.noToggleToOff)) return "This field requires a selection.";
    return null;
  }
  if (value.type === "choice" && field.kind === "choice") {
    if (value.indices.some((index, position) => !Number.isInteger(index) || index < 0 || index >= field.options.length
      || position > 0 && index <= value.indices[position - 1])
      || !field.multiple && value.indices.length > 1) return "Choose valid, unique options.";
    if (field.required && (!value.indices.length || value.indices.every((index) => !field.options[index].exportValue))) {
      return "This required field needs a nonempty selection.";
    }
    return null;
  }
  return "The value does not match this form field's type.";
}

export function formChanges(snapshot: FormSnapshot | null, draft: FormDraft | null, current: {
  id: number; revision: number;
}): { changes: FormChange[]; errors: Record<string, string> } {
  if (!snapshot || !draft || !Object.keys(draft.values).length) throw new Error("Change a form field before applying.");
  if (snapshot.documentId !== current.id || snapshot.revision !== current.revision
    || draft.documentId !== current.id || draft.revision !== current.revision) {
    throw new Error("The PDF changed after these form drafts were started. They are kept for review; discard them explicitly before reloading.");
  }
  if (!snapshot.form.fillable) throw new Error(snapshot.form.reason ?? "This PDF does not permit form filling.");
  const changes: FormChange[] = [];
  const errors: Record<string, string> = {};
  for (const [id, value] of Object.entries(draft.values)) {
    const field = snapshot.form.fields.find((field) => field.id === id);
    const error = field ? validateFormValue(field, value) : "This field is no longer in the PDF.";
    if (error) errors[id] = error;
    else changes.push({ id, value });
  }
  return { changes, errors };
}
