import type { DocumentInfo, PdfRect } from "./types.ts";
import type { OcrLanguage } from "./ocrLanguages.ts";

export interface OcrOptions {
  pages: number[];
  dpi: 150 | 300;
  languages: OcrLanguage[];
  layout: "horizontal_print";
  segmentation: "auto" | "single_column";
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
  sourceEpoch: number;
  epoch: number;
  dpi: 150 | 300;
  languages: OcrLanguage[];
  layout: "horizontal_print";
  segmentation: "auto" | "single_column";
  pages: OcrPagePlan[];
}

export interface OcrWord {
  text: string;
  bounds: PdfRect;
  baseline: [number, number, number, number];
  line: number;
  lineBounds: PdfRect;
  direction: "ltr" | "rtl";
  separator: string;
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
  languages: OcrLanguage[];
}

export interface OcrProgress {
  stage: "preparing" | "loading" | "rendering" | "segmenting" | "recognizing" | "staging" | "committing";
  page: number;
  completed: number;
  total: number;
  fraction: number;
}
