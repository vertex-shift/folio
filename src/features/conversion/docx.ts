import type { PDFDocumentProxy } from "pdfjs-dist";
import type { TextContent } from "pdfjs-dist/types/src/display/api";
import type { OutputPlan, OutputProgress } from "../../lib/output.ts";
import { mapText } from "../../lib/search.ts";
import { MAX_DOCX_PAGE_TEXT_BYTES, MAX_DOCX_TEXT_BYTES, validateDocxText } from "./model.ts";

export interface DocxTextSummary {
  byteLength: number;
  pagesWithText: number;
}

export async function extractDocxPages(
  pdf: PDFDocumentProxy,
  plan: OutputPlan,
  stage: (jobId: number, index: number, bytes: Uint8Array) => Promise<void>,
  progress: (value: OutputProgress) => void,
  signal: AbortSignal,
  check: () => void,
): Promise<DocxTextSummary> {
  if (plan.kind !== "docx") throw new Error("This output plan is not a DOCX document-content conversion.");
  const encoder = new TextEncoder();
  let totalBytes = 0;
  let pagesWithText = 0;
  for (const [index, sourcePage] of plan.pages.entries()) {
    signal.throwIfAborted();
    check();
    progress({ page: index + 1, count: plan.pages.length });
    const page = await pdf.getPage(sourcePage + 1);
    const stream: ReadableStream<TextContent> = page.streamTextContent();
    const reader = stream.getReader();
    const parts: Uint8Array[] = [];
    let pageBytes = 0;
    let hasText = false;
    let finished = false;
    let cancelFailure: unknown;
    const cancel = () => { void reader.cancel().catch((error: unknown) => { cancelFailure = error; }); };
    signal.addEventListener("abort", cancel, { once: true });
    try {
      for (;;) {
        signal.throwIfAborted();
        const { value, done } = await reader.read();
        signal.throwIfAborted();
        check();
        if (done) { finished = true; break; }
        const text = mapText(value.items.filter((item) => "str" in item)).text;
        validateDocxText(text);
        const bytes = encoder.encode(text);
        pageBytes += bytes.length;
        totalBytes += bytes.length;
        if (pageBytes > MAX_DOCX_PAGE_TEXT_BYTES || totalBytes > MAX_DOCX_TEXT_BYTES) {
          throw new Error("DOCX text is limited to 256 KiB per page and 8 MiB total. Export fewer or other pages; no text was truncated.");
        }
        hasText ||= Boolean(text.trim());
        parts.push(bytes);
      }
      const bytes = new Uint8Array(pageBytes);
      let offset = 0;
      for (const part of parts) { bytes.set(part, offset); offset += part.length; }
      signal.throwIfAborted();
      check();
      await stage(plan.jobId, index, bytes);
      signal.throwIfAborted();
      check();
      if (hasText) pagesWithText++;
    } finally {
      signal.removeEventListener("abort", cancel);
      try {
        if (!finished) await reader.cancel();
        if (cancelFailure) throw new Error("DOCX extraction stream cleanup failed.", { cause: cancelFailure });
      } finally {
        reader.releaseLock();
        page.cleanup();
      }
    }
  }
  if (!pagesWithText) throw new Error("No editable text was found on these pages. Image-only scans require a separate OCR operation.");
  return { byteLength: totalBytes, pagesWithText };
}
