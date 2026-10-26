import type { DocumentInfo, PdfRect } from "./types.ts";

export interface OcrOptions {
  pages: number[];
  dpi: 150 | 300;
  language: "eng";
  existingText: "skip" | "force";
}

export interface OcrPagePlan {
  page: number;
  sourceBounds: PdfRect;
  rotation: number;
  userUnit: number;
  pixelWidth: number;
  pixelHeight: number;
  skipped: boolean;
  existingText: boolean;
}

export interface OcrPlan {
  jobId: number;
  documentId: number;
  revision: number;
  dpi: 150 | 300;
  pages: OcrPagePlan[];
}

export interface OcrWord {
  text: string;
  bounds: PdfRect;
  baseline: [number, number, number, number];
  line: number;
}

export interface OcrPageResult {
  page: number;
  words: OcrWord[];
}

export interface OcrOutcome {
  document: DocumentInfo;
  wordCount: number;
  processedPages: number;
  skippedPages: number[];
  noTextPages: number[];
}

export interface OcrProgress {
  stage: "preparing" | "loading" | "rendering" | "recognizing" | "staging" | "committing";
  page: number;
  completed: number;
  total: number;
  fraction: number;
}
