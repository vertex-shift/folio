export type Quad = [number, number, number, number, number, number, number, number];
export type Point = [number, number];
export type Color = [number, number, number];

export interface AnnotationInfo {
  id: string;
  page: number;
  kind: "note" | "highlight";
  text: string;
  author: string;
  rect: [number, number, number, number];
  color: Color;
  deletable: boolean;
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
  pages: { key: string; rotation: number }[];
  annotations: AnnotationInfo[];
}

export type Edit =
  | { type: "rotate"; page: number; degrees: 90 | -90 | 180 }
  | { type: "move"; from: number; to: number }
  | { type: "delete"; page: number }
  | { type: "highlight"; page: number; quads: Quad[]; color: Color; text: string }
  | { type: "note"; page: number; point: Point; text: string }
  | { type: "remove_annotation"; id: string };

export type Tool = "select" | "hand" | "highlight" | "note";
export type Mode = "read" | "comment" | "organize";
export type Panel = "tools" | "search" | "comments";
export type Zoom = { mode: "page" | "width" } | { mode: "custom"; scale: number };

export interface SearchResult {
  page: number;
  index: number;
  before: string;
  match: string;
  after: string;
}
