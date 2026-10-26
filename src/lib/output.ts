import type { PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import type { TextContent } from "pdfjs-dist/types/src/display/api";
import { parsePageRange } from "./pages.ts";
import { mapText } from "./search.ts";
import type { DocumentInfo } from "./types.ts";

export type OutputAction = "print" | "export" | "optimize";
export type OutputKind = "print" | "png" | "jpeg" | "text" | "optimize";
export type OutputDpi = 96 | 150 | 300;
export type ExportFormat = "png" | "jpeg" | "text";

export const MAX_OUTPUT_BYTES = 128 * 1024 * 1024;
export const MAX_OUTPUT_PAGES = 500;
export const MAX_PRINT_PAGES = 200;
export const MAX_PAGE_PIXELS = 24_000_000;
export const MAX_PRINT_PIXELS = 100_000_000;
export const MAX_IMAGE_BYTES = 64 * 1024 * 1024;
export const MAX_TEXT_BYTES = 16 * 1024 * 1024;

export interface OutputRequest {
  documentId: number;
  revision: number;
  kind: OutputKind;
  pages: number[];
  dpi: OutputDpi | null;
  quality: number | null;
  allowUnencrypted: boolean;
}

export interface OutputPlan extends Omit<OutputRequest, "allowUnencrypted"> {
  jobId: number;
  suggestedName: string;
  sourceBytes: number;
  passwordProtected: boolean;
}

export interface OptimizationReport {
  beforeBytes: number;
  candidateBytes: number;
  afterBytes: number;
  reduced: boolean;
  compressedStreams: number;
  removedObjects: number;
}

export interface RasterGeometry {
  widthPoints: number;
  heightPoints: number;
  pixelWidth: number;
  pixelHeight: number;
}

export interface PrintImage extends RasterGeometry {
  blob: Blob;
}

export type OutputProgress = { page: number; count: number };

export function outputPages(all: boolean, range: string, count: number, kind: OutputKind): number[] {
  const limit = kind === "print" ? MAX_PRINT_PAGES : MAX_OUTPUT_PAGES;
  if (!Number.isSafeInteger(count) || count < 1) throw new Error("This PDF has no readable pages.");
  if (all) {
    if (count > limit) throw new Error(`Select a range of no more than ${limit} pages for this output.`);
    return Array.from({ length: count }, (_, index) => index);
  }
  if (range.length > 4096) throw new Error("The page range is too long. Use shorter ascending ranges.");
  return parsePageRange(range, count, limit);
}

export function assertOutputCurrent(plan: Pick<OutputPlan, "documentId" | "revision">, info: Pick<DocumentInfo, "id" | "revision"> | null, pending: boolean) {
  if (!info || info.id !== plan.documentId || info.revision !== plan.revision) {
    throw new Error("The document changed during output preparation. Prepare the current PDF again.");
  }
  if (pending) {
    throw new Error("Apply or cancel pending writing/form drafts and review or clear redaction marks before printing or exporting.");
  }
}

export function rasterGeometry(base: { width: number; height: number }, dpi: OutputDpi, pageNumber: number): RasterGeometry {
  if (![96, 150, 300].includes(dpi) || !Number.isFinite(base.width) || !Number.isFinite(base.height)
    || base.width <= 0 || base.height <= 0) {
    throw new Error(`Page ${pageNumber} has unsupported output geometry.`);
  }
  const pixelWidth = Math.ceil(base.width * dpi / 72);
  const pixelHeight = Math.ceil(base.height * dpi / 72);
  if (pixelWidth > 16_384 || pixelHeight > 16_384 || pixelWidth * pixelHeight > MAX_PAGE_PIXELS) {
    throw new Error(`Page ${pageNumber} exceeds 24 megapixels or 16,384 pixels per dimension at ${dpi} DPI. Choose a lower resolution.`);
  }
  return { widthPoints: base.width, heightPoints: base.height, pixelWidth, pixelHeight };
}

export function printPageRule(geometry: RasterGeometry, index: number): string {
  if (!Number.isSafeInteger(index) || index < 0 || !Number.isFinite(geometry.widthPoints)
    || !Number.isFinite(geometry.heightPoints) || geometry.widthPoints <= 0 || geometry.heightPoints <= 0) {
    throw new Error("The prepared print page has invalid geometry.");
  }
  return `@page folio-output-${index} { size: ${geometry.widthPoints.toFixed(6)}pt ${geometry.heightPoints.toFixed(6)}pt; margin: 0; }`;
}

export async function renderOutputImages(
  pdf: PDFDocumentProxy,
  plan: OutputPlan,
  stage: (jobId: number, index: number, bytes: Uint8Array) => Promise<void>,
  progress: (value: OutputProgress) => void,
  signal: AbortSignal,
  check: () => void,
  onPrintImage?: (image: PrintImage) => Promise<void>,
): Promise<void> {
  if (!plan.dpi || !["print", "png", "jpeg"].includes(plan.kind)) {
    throw new Error("This output plan is not a supported image or print request.");
  }
  let totalBytes = 0;
  let printPixels = 0;
  for (const [index, sourcePage] of plan.pages.entries()) {
    signal.throwIfAborted();
    check();
    progress({ page: index + 1, count: plan.pages.length });
    const page = await pdf.getPage(sourcePage + 1);
    signal.throwIfAborted();
    check();
    const geometry = rasterGeometry(page.getViewport({ scale: 1 }), plan.dpi, sourcePage + 1);
    printPixels += geometry.pixelWidth * geometry.pixelHeight;
    if (plan.kind === "print" && printPixels > MAX_PRINT_PIXELS) {
      throw new Error("The print snapshot exceeds 100 megapixels. Choose a lower resolution or a smaller page range.");
    }
    const viewport = page.getViewport({ scale: plan.dpi / 72 });
    const canvas = document.createElement("canvas");
    canvas.width = geometry.pixelWidth;
    canvas.height = geometry.pixelHeight;
    let render: RenderTask | undefined;
    const cancel = () => render?.cancel();
    signal.addEventListener("abort", cancel, { once: true });
    try {
      signal.throwIfAborted();
      check();
      const context = canvas.getContext("2d", { alpha: false, colorSpace: "srgb" });
      if (!context) throw new Error(`Page ${sourcePage + 1} could not obtain an output canvas.`);
      render = page.render({
        canvas, canvasContext: context, viewport, background: "#ffffff",
        intent: plan.kind === "print" ? "print" : "display",
      });
      await render.promise;
      signal.throwIfAborted();
      check();
      const mime = plan.kind === "jpeg" ? "image/jpeg" : "image/png";
      const blob = await new Promise<Blob>((resolve, reject) => {
        canvas.toBlob((blob) => blob ? resolve(blob)
          : reject(new Error(`Page ${sourcePage + 1} could not be encoded. No output was saved.`)),
        mime, plan.kind === "jpeg" ? (plan.quality ?? 90) / 100 : undefined);
      });
      signal.throwIfAborted();
      if (blob.type !== mime) throw new Error(`This renderer cannot encode ${mime}. No output was saved.`);
      if (blob.size > MAX_IMAGE_BYTES) throw new Error(`Page ${sourcePage + 1} exceeds the 64 MiB image limit. Choose a lower resolution.`);
      totalBytes += blob.size + 160;
      if (totalBytes > MAX_OUTPUT_BYTES) {
        throw new Error("The output exceeds 128 MiB. Choose fewer pages or a lower resolution/quality.");
      }
      check();
      await stage(plan.jobId, index, new Uint8Array(await blob.arrayBuffer()));
      signal.throwIfAborted();
      check();
      if (onPrintImage) await onPrintImage({ blob, ...geometry });
    } finally {
      signal.removeEventListener("abort", cancel);
      canvas.width = 0;
      canvas.height = 0;
      page.cleanup();
    }
  }
}

export async function renderOutputText(
  pdf: PDFDocumentProxy,
  plan: OutputPlan,
  stage: (jobId: number, index: number, bytes: Uint8Array) => Promise<void>,
  progress: (value: OutputProgress) => void,
  signal: AbortSignal,
  check: () => void,
): Promise<void> {
  if (plan.kind !== "text") throw new Error("This output plan is not a text request.");
  const encoder = new TextEncoder();
  let totalBytes = 0;
  let hasText = false;
  for (const [index, sourcePage] of plan.pages.entries()) {
    signal.throwIfAborted();
    check();
    progress({ page: index + 1, count: plan.pages.length });
    const page = await pdf.getPage(sourcePage + 1);
    signal.throwIfAborted();
    check();
    const stream: ReadableStream<TextContent> = page.streamTextContent();
    const reader = stream.getReader();
    const parts: Uint8Array[] = [];
    let pageBytes = 0;
    let finished = false;
    try {
      for (;;) {
        signal.throwIfAborted();
        const { value, done } = await reader.read();
        signal.throwIfAborted();
        check();
        if (done) { finished = true; break; }
        const text = mapText(value.items.filter((item) => "str" in item)).text;
        hasText ||= Boolean(text.trim());
        const bytes = encoder.encode(text);
        totalBytes += bytes.length;
        pageBytes += bytes.length;
        if (totalBytes + index * 3 > MAX_TEXT_BYTES) {
          throw new Error("The selected text exceeds the 16 MiB limit. Export fewer pages.");
        }
        parts.push(bytes);
      }
      const bytes = new Uint8Array(pageBytes);
      let offset = 0;
      for (const part of parts) { bytes.set(part, offset); offset += part.length; }
      signal.throwIfAborted();
      check();
      await stage(plan.jobId, index, bytes);
    } finally {
      if (!finished) await reader.cancel();
      reader.releaseLock();
      page.cleanup();
    }
  }
  if (!hasText) throw new Error("No selectable text was found on these pages. Image-only pages need OCR.");
}
