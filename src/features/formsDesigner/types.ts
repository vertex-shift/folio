import type { FormField, FormValue } from "../../lib/formsTypes";

export type ControlKind = "text" | "checkbox" | "radio" | "list" | "combo" | "signature";
export interface OptionDefinition { label: string; exportValue: string }
export interface FieldDefinition {
  kind: ControlKind;
  name: string;
  label: string;
  required: boolean;
  readOnly: boolean;
  multiline: boolean;
  password: boolean;
  comb: boolean;
  maxLength: number;
  multiple: boolean;
  editableChoice: boolean;
  noToggleToOff: boolean;
  options: OptionDefinition[];
  defaultValue: FormValue | null;
  fontSize: number;
  alignment: number;
}
export interface WidgetPlacement {
  page: number;
  placement: [number, number, number, number];
  option: number | null;
}
export interface DesignField {
  field: FormField;
  definition: FieldDefinition | null;
  editable: boolean;
  reason: string | null;
}
export interface DesignerInfo {
  fields: DesignField[];
  authorable: boolean;
  reason: string | null;
  warnings: string[];
}
export interface DesignerSnapshot { documentId: number; revision: number; designer: DesignerInfo }
export type DesignChange =
  | { type: "create"; definition: FieldDefinition; widgets: WidgetPlacement[] }
  | { type: "update"; id: string; definition: FieldDefinition }
  | { type: "place_widget"; id: string; widgetId: string; widget: WidgetPlacement }
  | { type: "remove_widget"; id: string; widgetId: string }
  | { type: "remove_field"; id: string };
export interface WidgetDraft {
  id: string | null;
  page: string;
  x: string;
  y: string;
  width: string;
  height: string;
  option: number | null;
}
export interface DesignerDraft {
  documentId: number;
  revision: number;
  fieldId: string | null;
  definition: Omit<FieldDefinition, "maxLength" | "fontSize"> & { maxLength: string; fontSize: string };
  widgets: WidgetDraft[];
  removedWidgets: string[];
  deleteField: boolean;
  dirty: boolean;
}
export interface FlattenInfo {
  fieldCount: number;
  widgetCount: number;
  unsignedSignatureCount: number;
  warnings: string[];
}
