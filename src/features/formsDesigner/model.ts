import { decimal } from "../../lib/authoring.ts";
import type { PageGeometry } from "../../lib/authoring.ts";
import type { DocumentInfo, PdfRect } from "../../lib/types.ts";
import type { ControlKind, DesignChange, DesignerDraft, DesignerSnapshot, FieldDefinition, WidgetDraft, WidgetPlacement } from "./types.ts";

export const controlLabels: Record<ControlKind, string> = {
  text: "Text field", checkbox: "Checkbox", radio: "Radio group",
  list: "List box", combo: "Combo box", signature: "Unsigned signature field",
};

export function buttonState(value: string): string {
  return Array.from(new TextEncoder().encode(value), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function defaultDefinition(kind: ControlKind, name = "field1"): DesignerDraft["definition"] {
  const options = kind === "checkbox" ? [{ label: "Yes", exportValue: "Yes" }]
    : ["radio", "list", "combo"].includes(kind) ? [{ label: "First option", exportValue: "First" }, { label: "Second option", exportValue: "Second" }] : [];
  return { kind, name, label: "", required: false, readOnly: false, multiline: false, password: false, comb: false,
    maxLength: "4000", multiple: false, editableChoice: false, noToggleToOff: false,
    options, defaultValue: null, fontSize: "0", alignment: 0 };
}

export function newWidget(info: DocumentInfo, page: number, kind: ControlKind, option: number | null = null): WidgetDraft {
  const geometry = info.pages[page].geometry;
  const small = kind === "radio" || kind === "checkbox";
  const width = Math.min(small ? 20 : 200, geometry.widthPoints);
  const height = Math.min(small ? 20 : kind === "list" ? 96 : 32, geometry.heightPoints);
  return { id: null, page: String(page + 1), x: decimal(Math.min(36, geometry.widthPoints - width)),
    y: decimal(Math.min(36 + (option ?? 0) * 30, geometry.heightPoints - height)),
    width: decimal(width), height: decimal(height), option };
}

export function newDesignerDraft(info: DocumentInfo, page: number, kind: ControlKind, snapshot: DesignerSnapshot | null): DesignerDraft {
  const names = new Set(snapshot?.designer.fields.map(({ field }) => field.name));
  let suffix = 1;
  while (names.has(`field${suffix}`)) suffix++;
  const definition = defaultDefinition(kind, `field${suffix}`);
  return { documentId: info.id, revision: info.revision, fieldId: null, definition,
    widgets: kind === "radio" ? definition.options.map((_, option) => newWidget(info, page, kind, option)) : [newWidget(info, page, kind)],
    removedWidgets: [], deleteField: false, dirty: false };
}

export function rectToPlacement(rect: PdfRect, geometry: PageGeometry): [number, number, number, number] {
  const [left, bottom, right, top] = geometry.bounds, unit = geometry.userUnit;
  const point = (x: number, y: number): [number, number] => {
    switch (geometry.rotation) {
      case 0: return [(x - left) * unit, (top - y) * unit];
      case 90: return [(y - bottom) * unit, (x - left) * unit];
      case 180: return [(right - x) * unit, (y - bottom) * unit];
      case 270: return [(top - y) * unit, (right - x) * unit];
      default: throw new Error("This page has unsupported rotation.");
    }
  };
  const a = point(rect[0], rect[1]), b = point(rect[2], rect[3]);
  return [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(a[0] - b[0]), Math.abs(a[1] - b[1])];
}

export function editDesignerDraft(info: DocumentInfo, snapshot: DesignerSnapshot, id: string): DesignerDraft {
  assertSnapshot(info, snapshot);
  const selected = snapshot.designer.fields.find(({ field }) => field.id === id);
  if (!selected?.editable || !selected.definition) throw new Error(selected?.reason ?? "This field cannot be safely authored.");
  const definition = structuredClone(selected.definition);
  return { documentId: info.id, revision: info.revision, fieldId: id,
    definition: { ...definition, maxLength: String(definition.maxLength), fontSize: String(definition.fontSize) },
    widgets: selected.field.widgets.map((widget) => {
      const [x, y, width, height] = rectToPlacement(widget.rect, info.pages[widget.page].geometry);
      return { id: widget.id, page: String(widget.page + 1), x: decimal(x), y: decimal(y), width: decimal(width), height: decimal(height), option: null };
    }),
    removedWidgets: [], deleteField: false, dirty: false };
}

function number(value: string, label: string, min: number, max: number, integer = false): number {
  const parsed = Number(value);
  if (!value.trim() || !Number.isFinite(parsed) || parsed < min || parsed > max || integer && !Number.isSafeInteger(parsed)) {
    throw new Error(`${label} must be ${integer ? "a whole number " : ""}between ${min} and ${max}. Your draft is kept.`);
  }
  return parsed;
}

export function widgetPlacement(widget: WidgetDraft, info: DocumentInfo): WidgetPlacement {
  const page = number(widget.page, "Widget page", 1, info.pageCount, true) - 1;
  const geometry = info.pages[page].geometry;
  const x = number(widget.x, "Left (pt)", 0, geometry.widthPoints), y = number(widget.y, "Top (pt)", 0, geometry.heightPoints);
  const width = number(widget.width, "Width (pt)", 0.1, geometry.widthPoints);
  const height = number(widget.height, "Height (pt)", 0.1, geometry.heightPoints);
  if (x + width > geometry.widthPoints + 0.001 || y + height > geometry.heightPoints + 0.001
      || width / geometry.userUnit < 4 || height / geometry.userUnit < 4) {
    throw new Error("Keep the whole widget inside the crop and at least 4 PDF units wide and high. Your draft is kept.");
  }
  return { page, placement: [x, y, width, height], option: widget.option };
}

export function assertSnapshot(info: Pick<DocumentInfo, "id" | "revision">, snapshot: DesignerSnapshot | null): asserts snapshot is DesignerSnapshot {
  if (!snapshot || info.id !== snapshot.documentId || info.revision !== snapshot.revision) {
    throw new Error("The PDF changed. Reload the designer before editing; existing drafts are kept.");
  }
}

export function designChanges(info: DocumentInfo, snapshot: DesignerSnapshot | null, draft: DesignerDraft | null): DesignChange[] {
  assertSnapshot(info, snapshot);
  if (!draft || draft.documentId !== info.id || draft.revision !== info.revision) {
    throw new Error("This form-design draft is stale. Its settings are kept; discard it explicitly before editing the new revision.");
  }
  if (!snapshot.designer.authorable) throw new Error(snapshot.designer.reason ?? "General editing is not authorized.");
  if (draft.fieldId && draft.deleteField) return [{ type: "remove_field", id: draft.fieldId }];
  const definition: FieldDefinition = { ...draft.definition,
    maxLength: number(draft.definition.maxLength, "Maximum characters", 1, 4000, true),
    fontSize: number(draft.definition.fontSize, "Font size (PDF units, 0 = automatic)", 0, 256),
  };
  if (!definition.name.trim() || definition.name.includes(".") || Array.from(definition.name).length > 256) {
    throw new Error("Use a nonempty partial field name up to 256 characters without a period. Parent namespaces are preserved.");
  }
  const widgets = draft.widgets.map((widget) => widgetPlacement(widget, info));
  if (!widgets.length) throw new Error("Keep a widget or explicitly remove the logical field.");
  if (!draft.fieldId) return [{ type: "create", definition, widgets }];
  const previous = editDesignerDraft(info, snapshot, draft.fieldId);
  const changes: DesignChange[] = [];
  if (JSON.stringify(previous.definition) !== JSON.stringify(draft.definition)) {
    changes.push({ type: "update", id: draft.fieldId, definition });
  }
  draft.widgets.forEach((widget, index) => {
    const initial = previous.widgets.find((item) => item.id === widget.id);
    if (!widget.id || !initial) throw new Error("Adding widgets to an existing field is not supported. Create a new repeated field instead.");
    if (JSON.stringify(initial) !== JSON.stringify(widget)) {
      changes.push({ type: "place_widget", id: draft.fieldId!, widgetId: widget.id, widget: { ...widgets[index], option: null } });
    }
  });
  changes.push(...draft.removedWidgets.map((widgetId): DesignChange => ({ type: "remove_widget", id: draft.fieldId!, widgetId })));
  if (!changes.length) throw new Error("No form-design changes to apply. Discard the draft or change a property or placement.");
  if (changes.length > 64) throw new Error("Apply at most 64 form-design changes in one transaction.");
  return changes;
}
