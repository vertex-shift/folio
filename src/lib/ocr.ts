import type { PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import type { OcrPagePlan, OcrPageResult, OcrPlan, OcrProgress, OcrWord } from "./ocrTypes.ts";
import { LocalOcrWorker } from "./ocrWorker.ts";

export const MAX_OCR_PAGES = 50;

export function awaitOcrStep<T>(operation: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise((resolve, reject) => {
    const abort = () => reject(signal.reason);
    signal.addEventListener("abort", abort, { once: true });
    operation.then((value) => {
      signal.removeEventListener("abort", abort);
      if (signal.aborted) reject(signal.reason); else resolve(value);
    }, (error: unknown) => {
      signal.removeEventListener("abort", abort);
      reject(error);
    });
    if (signal.aborted) {
      signal.removeEventListener("abort", abort);
      reject(signal.reason);
    }
  });
}

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("The OCR engine returned an unsupported result structure. No text was added.");
  }
  return value as Record<string, unknown>;
}

function array(value: unknown): unknown[] {
  if (!Array.isArray(value)) throw new Error("The OCR engine did not return complete word geometry. No text was added.");
  return value;
}

function coordinate(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new Error("The OCR engine returned invalid coordinates. No text was added.");
  return value;
}

export function recognitionWords(output: unknown, page: OcrPagePlan): OcrWord[] {
  const data = record(output);
  if (typeof data.text !== "string") throw new Error("The OCR engine returned no text result. No text was added.");
  if (data.blocks === null && !data.text.trim()) return [];
  const words: OcrWord[] = [];
  let lineIndex = 0;
  for (const block of array(data.blocks)) for (const paragraph of array(record(block).paragraphs)) {
    const paragraphData = record(paragraph);
    if (paragraphData.is_ltr === false) throw new Error("Right-to-left OCR is not supported by the bundled English model. No text was added.");
    for (const line of array(paragraphData.lines)) {
      const row = record(line);
      const baseline = record(row.baseline);
      const x0 = coordinate(baseline.x0), y0 = coordinate(baseline.y0);
      const x1 = coordinate(baseline.x1), y1 = coordinate(baseline.y1);
      if (baseline.has_baseline === false || x1 <= x0 || Math.abs((y1 - y0) / (x1 - x0)) > 0.25) {
        throw new Error(`Page ${page.page + 1} has unsupported OCR text orientation. Deskew or rotate the scan first; no text was added.`);
      }
      const y = (x: number) => y0 + (x - x0) * (y1 - y0) / (x1 - x0);
      for (const item of array(row.words)) {
        const word = record(item);
        if (typeof word.text !== "string" || !word.text.trim()) throw new Error("The OCR engine returned an empty word. No text was added.");
        const box = record(word.bbox);
        const left = coordinate(box.x0), top = coordinate(box.y0);
        const right = coordinate(box.x1), bottom = coordinate(box.y1);
        if (left < 0 || top < 0 || right > page.pixelWidth || bottom > page.pixelHeight || right <= left || bottom <= top) {
          throw new Error(`Page ${page.page + 1} has OCR words outside its rendered bounds. No text was added.`);
        }
        words.push({ text: word.text.normalize("NFC"), bounds: [left, top, right, bottom],
          baseline: [left, y(left), right, y(right)], line: lineIndex });
        if (words.length > 10_000) throw new Error(`Page ${page.page + 1} exceeds the 10,000-word OCR limit. No text was added.`);
      }
      lineIndex++;
    }
  }
  if (!words.length && data.text.trim()) throw new Error("The OCR engine returned text without word geometry. No text was added.");
  return words;
}

export async function recognizeOcrPages(
  pdf: PDFDocumentProxy,
  plan: OcrPlan,
  stage: (jobId: number, result: OcrPageResult, signal?: AbortSignal) => Promise<void>,
  progress: (value: OcrProgress) => void,
  signal: AbortSignal,
): Promise<void> {
  const pages = plan.pages.filter((page) => !page.skipped);
  if (plan.pages.length > MAX_OCR_PAGES || !plan.pages.length || plan.pages.some((page) => page.page >= pdf.numPages)) {
    throw new Error("The OCR plan does not match this PDF or exceeds the 50-page job limit.");
  }
  signal.throwIfAborted();
  if (!pages.length) return;
  const controller = new AbortController();
  const abort = () => controller.abort(signal.reason);
  signal.addEventListener("abort", abort, { once: true });
  const timeout = setTimeout(() => controller.abort(new Error("OCR exceeded its 15-minute job limit. No text was added.")), 15 * 60_000);
  let worker: LocalOcrWorker | undefined;
  let completed = 0;
  let currentPage = pages[0].page;
  try {
    worker = new LocalOcrWorker(controller.signal, (fraction) => progress({
      stage: "recognizing", page: currentPage, completed, total: pages.length, fraction,
    }));
    progress({ stage: "loading", page: currentPage, completed, total: pages.length, fraction: 0 });
    await worker.initialize(plan.dpi);
    for (const target of pages) {
      controller.signal.throwIfAborted();
      currentPage = target.page;
      progress({ stage: "rendering", page: currentPage, completed, total: pages.length, fraction: 0 });
      const page = await awaitOcrStep(pdf.getPage(target.page + 1), controller.signal);
      controller.signal.throwIfAborted();
      const viewport = page.getViewport({ scale: plan.dpi / 72 });
      if (page.rotate !== target.rotation || Math.abs(page.userUnit - target.userUnit) > 0.0001
        || page.view.some((value, index) => Math.abs(value - target.sourceBounds[index]) > 0.01)
        || Math.ceil(viewport.width) !== target.pixelWidth || Math.ceil(viewport.height) !== target.pixelHeight
        || target.pixelWidth * target.pixelHeight > 24_000_000 || target.pixelWidth > 16_384 || target.pixelHeight > 16_384) {
        throw new Error(`Page ${target.page + 1} has inconsistent or unsupported OCR geometry. No text was added.`);
      }
      const canvas = document.createElement("canvas");
      let render: RenderTask | undefined;
      const cancelRender = () => render?.cancel();
      controller.signal.addEventListener("abort", cancelRender, { once: true });
      try {
        controller.signal.throwIfAborted();
        canvas.width = target.pixelWidth; canvas.height = target.pixelHeight;
        const context = canvas.getContext("2d", { alpha: false, colorSpace: "srgb" });
        if (!context) throw new Error("A local OCR page canvas could not be created. No text was added.");
        render = page.render({ canvas, canvasContext: context, viewport, background: "#ffffff", annotationMode: 0 });
        await awaitOcrStep(render.promise, controller.signal);
        controller.signal.throwIfAborted();
        const blob = await awaitOcrStep(new Promise<Blob>((resolve, reject) => {
          canvas.toBlob((value) => value ? resolve(value) : reject(new Error("The OCR scan could not be encoded. No text was added.")), "image/png");
        }), controller.signal);
        if (blob.size > 64 * 1024 * 1024) throw new Error("The rendered OCR page exceeds 64 MiB. Try 150 DPI; no text was added.");
        controller.signal.throwIfAborted();
        progress({ stage: "recognizing", page: currentPage, completed, total: pages.length, fraction: 0 });
        const output = await worker.recognize(await awaitOcrStep(blob.arrayBuffer(), controller.signal));
        controller.signal.throwIfAborted();
        const words = recognitionWords(output, target);
        progress({ stage: "staging", page: currentPage, completed, total: pages.length, fraction: 1 });
        await awaitOcrStep(stage(plan.jobId, { page: target.page, words }, controller.signal), controller.signal);
        controller.signal.throwIfAborted();
        completed++;
      } finally {
        controller.signal.removeEventListener("abort", cancelRender);
        canvas.width = 0; canvas.height = 0;
      }
    }
  } finally {
    worker?.close();
    clearTimeout(timeout);
    signal.removeEventListener("abort", abort);
  }
}
