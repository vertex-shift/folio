import type { PdfRect } from "./types";

export type FormValue =
  | { type: "text"; text: string }
  | { type: "button"; state: string | null }
  | { type: "choice"; indices: number[]; custom: string | null };

export interface FormOption {
  id: string;
  label: string;
  exportValue: string;
}

export interface FormWidget {
  page: number;
  rect: PdfRect;
}

export interface FormField {
  id: string;
  name: string;
  label: string;
  kind: "text" | "checkbox" | "radio" | "choice" | "signature" | "unsupported";
  readOnly: boolean;
  reason: string | null;
  required: boolean;
  multiline: boolean;
  password: boolean;
  comb: boolean;
  maxLength: number;
  multiple: boolean;
  combo: boolean;
  editableChoice: boolean;
  noToggleToOff: boolean;
  options: FormOption[];
  value: FormValue | null;
  widgets: FormWidget[];
}

export interface FormInfo {
  fields: FormField[];
  fillable: boolean;
  reason: string | null;
  warnings: string[];
}

export interface FormSnapshot {
  documentId: number;
  revision: number;
  form: FormInfo;
}

export interface FormDraft {
  documentId: number;
  revision: number;
  values: Record<string, FormValue>;
}

export interface FormChange {
  id: string;
  value: FormValue;
}
