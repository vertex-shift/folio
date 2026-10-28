import type { ReplacementFont } from "../../lib/types";

export type ParagraphPlacement = [number, number, number, number];
export type ParagraphAlignment = "left" | "center" | "right";

export interface ParagraphBlock {
  id: string;
  path: number[];
  text: string | null;
  sourceLines: { text: string; baseline: [number, number]; bounds: ParagraphPlacement }[];
  placement: ParagraphPlacement | null;
  rotation: number;
  size: number;
  leading: number;
  alignment: ParagraphAlignment;
  font: string;
  fontMode: ReplacementFont;
  originalFontAvailable: boolean;
  fontReason: string | null;
  editable: boolean;
  reason: string | null;
}

export interface ParagraphPage {
  epoch: number;
  blocks: ParagraphBlock[];
  warnings: string[];
}

export interface ParagraphDraft {
  documentId: number;
  revision: number;
  epoch: number;
  page: number;
  source: ParagraphBlock;
  text: string;
  x: string;
  y: string;
  width: string;
  height: string;
  size: string;
  leading: string;
  alignment: ParagraphAlignment;
  font: ReplacementFont;
}

export interface ParagraphChange {
  epoch: number;
  page: number;
  path: number[];
  blockId: string;
  expected: string;
  text: string;
  placement: ParagraphPlacement;
  size: number;
  leading: number;
  alignment: ParagraphAlignment;
  font: ReplacementFont;
}

export interface ParagraphLayout {
  width: number;
  height: number;
  usedWidth: number;
  usedHeight: number;
  lines: {
    text: string;
    start: number;
    end: number;
    hardBreak: boolean;
    baseline: [number, number];
    advance: number;
    inkBounds: ParagraphPlacement | null;
  }[];
  fits: boolean;
  overflow: string | null;
}

export interface ParagraphPreview {
  epoch: number;
  layout: ParagraphLayout;
  renderPdf: number[] | null;
}

export interface ParagraphInspection {
  loading: boolean;
  result: ParagraphPage | null;
  error: string | null;
}

export interface ParagraphPreviewState {
  loading: boolean;
  result: ParagraphPreview | null;
  error: string | null;
}
