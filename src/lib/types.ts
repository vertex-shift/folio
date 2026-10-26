export type Quad = [number, number, number, number, number, number, number, number];
export type Point = [number, number];
export type Color = [number, number, number];
export type PdfRect = [number, number, number, number];

export interface RedactionRegion {
  page: number;
  rect: PdfRect;
}

export interface RedactionMark extends RedactionRegion {
  id: number;
  documentId: number;
  revision: number;
}

export interface RedactionPlan {
  jobId: number;
  documentId: number;
  revision: number;
  dpi: 150 | 300;
  pages: {
    sourceBounds: PdfRect;
    rotation: number;
    userUnit: number;
    widthPoints: number;
    heightPoints: number;
    pixelWidth: number;
    pixelHeight: number;
    masks: { x: number; y: number; width: number; height: number }[];
  }[];
}

export interface AnnotationInfo {
  id: string;
  page: number;
  kind: "note" | "highlight" | "text" | "ink";
  text: string;
  author: string;
  rect: [number, number, number, number];
  color: Color;
  deletable: boolean;
  editable: boolean;
}

export interface DocumentInfo {
  id: number;
  name: string;
  revision: number;
  pageCount: number;
  byteLength: number;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  readOnlyReason: string | null;
  passwordProtected: boolean;
  copyAllowed: boolean;
  pages: { key: string; rotation: number }[];
  annotations: AnnotationInfo[];
}

export interface PasswordChallenge {
  status: "password_required";
  requestId: number;
  name: string;
  incorrect: boolean;
  importing: boolean;
}

export type OpenOutcome = {
  status: "opened";
  document: DocumentInfo;
  importedAt: number | null;
} | PasswordChallenge | { status: "failed"; name: string; message: string };

export interface WorkspaceInfo {
  documents: DocumentInfo[];
  activeId: number | null;
  opening: OpenOutcome[];
}

export interface ExistingTextRun {
  path: number[];
  text: string | null;
  font: string;
  fontSize: number | null;
  origin: [number, number] | null;
  editable: boolean;
  reason: string | null;
}

export type ReplacementFont = "original" | "noto";

export type Edit =
  | { type: "rotate"; page: number; degrees: 90 | -90 | 180 }
  | { type: "move"; from: number; to: number }
  | { type: "delete"; page: number }
  | { type: "highlight"; page: number; quads: Quad[]; color: Color; text: string }
  | { type: "note"; page: number; point: Point; text: string }
  | { type: "remove_annotation"; id: string }
  | { type: "set_annotation_text"; id: string; text: string }
  | { type: "text"; page: number; rect: [number, number, number, number]; text: string; size: number; color: Color }
  | { type: "ink"; page: number; points: Point[]; color: Color; width: number }
  | { type: "blank_page"; at: number; width: number; height: number }
  | { type: "duplicate"; page: number }
  | { type: "replace_text"; page: number; path: number[]; expected: string; replacement: string; font: ReplacementFont };

export type Tool = "select" | "hand" | "highlight" | "note" | "text" | "ink" | "edit-text" | "redact";
export type Mode = "read" | "write" | "comment" | "organize";
export type Panel = "tools" | "search" | "comments" | "bookmarks" | "write" | "edit" | "redact" | "signatures";
export type Zoom = { mode: "page" | "width" } | { mode: "custom"; scale: number };

export interface SearchResult {
  page: number;
  index: number;
  before: string;
  match: string;
  after: string;
}
