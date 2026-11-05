import type { PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import type { OcrPagePlan, OcrPageResult, OcrPlan, OcrProgress, OcrWord } from "./ocrTypes.ts";
import { LocalOcrWorker } from "./ocrWorker.ts";
import { selectedOcrLanguages } from "./ocrLanguages.ts";
import { OCR_RESOLUTIONS } from "./ocrTypes.ts";
import type { OcrDpi } from "./ocrTypes.ts";
import type { DocumentInfo } from "./types.ts";
export { LocalOcrWorker } from "./ocrWorker.ts";

export const MAX_OCR_PAGES = 50;
export const MAX_OCR_BANDS = 128;
const MAX_OCR_PIXELS = 24_000_000;
const MAX_OCR_DIMENSION = 16_384;

export function ocrResolutionIssue(pages: DocumentInfo["pages"], selection: readonly number[], dpi: OcrDpi): string | null {
  const fits = (index: number, resolution: OcrDpi) => {
    const geometry = pages[index]?.geometry;
    if (!geometry || !geometry.bounds.every(Number.isFinite) || !Number.isFinite(geometry.userUnit)
      || geometry.userUnit <= 0 || ![0, 90, 180, 270].includes(geometry.rotation)) {
      throw new Error("A selected page has invalid OCR geometry.");
    }
    const scale = (resolution / 72) * geometry.userUnit;
    const width = Math.ceil((geometry.bounds[2] - geometry.bounds[0]) * scale);
    const height = Math.ceil((geometry.bounds[3] - geometry.bounds[1]) * scale);
    if (width <= 0 || height <= 0) throw new Error("A selected page has invalid OCR bounds.");
    return width <= MAX_OCR_DIMENSION && height <= MAX_OCR_DIMENSION && width * height <= MAX_OCR_PIXELS;
  };
  const oversized = selection.find((index) => !fits(index, dpi));
  if (oversized === undefined) return null;
  const lower = OCR_RESOLUTIONS.find((resolution) => resolution < dpi && selection.every((index) => fits(index, resolution)));
  return `Page ${oversized + 1} exceeds the 24-megapixel or 16,384-pixel OCR limit at ${dpi} DPI. ${
    lower ? `Choose ${lower} DPI for this page range.` : "Even 72 DPI exceeds the limit; use a smaller source page."
  } The original PDF is not resized or resampled.`;
}

export async function horizontalOcrBands(pixels: Uint8ClampedArray, width: number, height: number, dpi: number,
  signal: AbortSignal): Promise<{ top: number; height: number }[]> {
  signal.throwIfAborted();
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width <= 0 || height <= 0
    || width > MAX_OCR_DIMENSION || height > MAX_OCR_DIMENSION || width * height > MAX_OCR_PIXELS
    || pixels.length !== width * height * 4 || !OCR_RESOLUTIONS.some((value) => value === dpi)) {
    throw new Error("The single-column OCR image has invalid or excessive geometry.");
  }
  const gap = Math.max(3, Math.round(dpi / 40));
  const bands: [number, number][] = [];
  let start = -1, last = -1, ink = 0, nonWhite = 0;
  for (let y = 0; y < height; y++) {
    if (y % 64 === 0) { await new Promise((resolve) => setTimeout(resolve, 0)); signal.throwIfAborted(); }
    let count = 0;
    for (let x = 0, offset = y * width * 4; x < width; x++, offset += 4) {
      const luminance = pixels[offset] * 54 + pixels[offset + 1] * 183 + pixels[offset + 2] * 19;
      if (luminance < 192 * 256) count++;
      if (luminance < 250 * 256) nonWhite++;
    }
    ink += count;
    if (count === 0) continue;
    if (start < 0) start = y;
    else if (y - last > gap) { bands.push([start, last + 1]); start = y; }
    last = y;
  }
  if (start >= 0) bands.push([start, last + 1]);
  if (bands.length > MAX_OCR_BANDS) throw new Error("Single-column OCR exceeds 128 horizontal ink bands. Choose fewer pages or automatic layout; no text was added.");
  if (ink > width * height * 0.4) throw new Error("Single-column OCR requires clean, light-background printed scans. Choose automatic layout for this image; no text was added.");
  if (!bands.length && nonWhite) throw new Error("Single-column OCR could not resolve the faint scan into ink bands. Improve contrast or choose automatic layout; no text was added.");
  signal.throwIfAborted();
  return bands.map(([top, bottom], index) => {
    const padding = Math.ceil(Math.max(dpi / 24, (bottom - top) * 0.3));
    const before = index ? Math.ceil((bands[index - 1][1] + top) / 2) : 0;
    const after = index + 1 < bands.length ? Math.floor((bottom + bands[index + 1][0]) / 2) : height;
    const begin = Math.max(before, top - padding);
    return { top: begin, height: Math.min(after, bottom + padding) - begin };
  });
}

export function shiftOcrWords(words: OcrWord[], top: number, lineOffset: number): OcrWord[] {
  return words.map((word) => ({ ...word, line: word.line + lineOffset,
    bounds: [word.bounds[0], word.bounds[1] + top, word.bounds[2], word.bounds[3] + top],
    lineBounds: [word.lineBounds[0], word.lineBounds[1] + top, word.lineBounds[2], word.lineBounds[3] + top],
    baseline: [word.baseline[0], word.baseline[1] + top, word.baseline[2], word.baseline[3] + top],
  }));
}

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
  for (const block of array(data.blocks)) {
    const blockData = record(block);
    if (blockData.blocktype === 7) throw new Error("Vertical OCR blocks are not supported. Use a horizontal printed scan; no text was added.");
    for (const paragraph of array(blockData.paragraphs)) {
      const paragraphData = record(paragraph);
      const ltr = paragraphData.is_ltr;
      if (ltr !== true && ltr !== false && ltr !== 0 && ltr !== 1) {
        throw new Error("The OCR engine returned no supported horizontal reading direction. No text was added.");
      }
      const direction = ltr === false || ltr === 0 ? "rtl" : "ltr";
      for (const line of array(paragraphData.lines)) {
        const row = record(line);
        if (typeof row.text !== "string") throw new Error("The OCR engine returned a line without its logical text. No text was added.");
        const lineBox = record(row.bbox);
        const lineBounds: [number, number, number, number] = [
          coordinate(lineBox.x0), coordinate(lineBox.y0), coordinate(lineBox.x1), coordinate(lineBox.y1),
        ];
        if (lineBounds[0] < 0 || lineBounds[1] < 0 || lineBounds[2] > page.pixelWidth || lineBounds[3] > page.pixelHeight
          || lineBounds[2] <= lineBounds[0] || lineBounds[3] <= lineBounds[1]) {
          throw new Error("The OCR engine returned an invalid line box. No text was added.");
        }
        const baseline = record(row.baseline);
        const x0 = coordinate(baseline.x0), y0 = coordinate(baseline.y0);
        const x1 = coordinate(baseline.x1), y1 = coordinate(baseline.y1);
        if (baseline.has_baseline === false || x1 <= x0 || Math.abs((y1 - y0) / (x1 - x0)) > 0.25) {
          throw new Error(`Page ${page.page + 1} has unsupported OCR text orientation. Deskew or rotate the scan first; no text was added.`);
        }
        const y = (x: number) => y0 + (x - x0) * (y1 - y0) / (x1 - x0);
        const lineWords = array(row.words);
        let textOffset = 0;
        for (const [index, item] of lineWords.entries()) {
          const word = record(item);
          if (typeof word.text !== "string" || !word.text || /\s/u.test(word.text)) throw new Error("The OCR engine returned an empty or unsupported word. No text was added.");
          const start = row.text.indexOf(word.text, textOffset);
          if (start < 0 || row.text.slice(textOffset, start).trim()) {
            throw new Error("The OCR engine returned inconsistent logical line/word text. No characters were discarded and no text was added.");
          }
          const next = index + 1 < lineWords.length ? record(lineWords[index + 1]).text : undefined;
          if (next !== undefined && typeof next !== "string") throw new Error("The OCR engine returned an incomplete line. No text was added.");
          const end = start + word.text.length;
          const nextStart = typeof next === "string" ? row.text.indexOf(next, end) : row.text.length;
          if (nextStart < end || row.text.slice(end, nextStart).trim()) {
            throw new Error("The OCR engine returned text outside its word geometry. No characters were discarded and no text was added.");
          }
          const separator = row.text.slice(end, nextStart);
          if (!/^[ \t\r\n]{0,256}$/.test(separator)) throw new Error("The OCR engine returned unsupported line separators. No text was added.");
          textOffset = nextStart;
          const box = record(word.bbox);
          const left = coordinate(box.x0), top = coordinate(box.y0);
          const right = coordinate(box.x1), bottom = coordinate(box.y1);
          if (left < 0 || top < 0 || right > page.pixelWidth || bottom > page.pixelHeight || right <= left || bottom <= top) {
            throw new Error(`Page ${page.page + 1} has OCR words outside its rendered bounds. No text was added.`);
          }
          words.push({ text: word.text, bounds: [left, top, right, bottom],
            baseline: [left, y(left), right, y(right)], line: lineIndex, lineBounds, direction, separator });
          if (words.length > 10_000) throw new Error(`Page ${page.page + 1} exceeds the 10,000-word OCR limit. No text was added.`);
        }
        lineIndex++;
      }
    }
  }
  if (!words.length && data.text.trim()) throw new Error("The OCR engine returned text without word geometry. No text was added.");
  if (words.map((word) => word.text).join("") !== data.text.replace(/\s/gu, "")) {
    throw new Error("The OCR engine returned page text that does not match its logical word geometry. No text was added.");
  }
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
  const languages = selectedOcrLanguages(plan.languages);
  if (!OCR_RESOLUTIONS.includes(plan.dpi)) throw new Error("The OCR plan has an unsupported scan resolution.");
  if (plan.layout !== "horizontal_print") throw new Error("Offline OCR supports horizontal printed text only.");
  if (plan.segmentation !== "auto" && plan.segmentation !== "single_column") throw new Error("The OCR plan has an unsupported text layout.");
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
  let bandIndex = 0, bandCount = 1;
  try {
    worker = new LocalOcrWorker(controller.signal, (fraction) => progress({
      stage: "recognizing", page: currentPage, completed, total: pages.length, fraction: (bandIndex + fraction) / bandCount,
    }));
    progress({ stage: "loading", page: currentPage, completed, total: pages.length, fraction: 0 });
    await worker.initialize(plan.dpi, languages, plan.segmentation);
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
        if (plan.segmentation === "single_column") progress({ stage: "segmenting", page: currentPage, completed, total: pages.length, fraction: 0 });
        const bands = plan.segmentation === "single_column"
          ? await horizontalOcrBands(context.getImageData(0, 0, canvas.width, canvas.height).data, canvas.width, canvas.height, plan.dpi, controller.signal)
          : [{ top: 0, height: canvas.height }];
        bandCount = Math.max(1, bands.length); bandIndex = 0;
        const words: OcrWord[] = [];
        let lineOffset = 0, pngBytes = 0;
        for (const band of bands) {
          controller.signal.throwIfAborted();
          const image = plan.segmentation === "single_column" ? document.createElement("canvas") : canvas;
          try {
            if (image !== canvas) {
              image.width = canvas.width; image.height = band.height;
              const cropped = image.getContext("2d", { alpha: false, colorSpace: "srgb" });
              if (!cropped) throw new Error("A local OCR line canvas could not be created. No text was added.");
              cropped.drawImage(canvas, 0, band.top, canvas.width, band.height, 0, 0, canvas.width, band.height);
            }
            const blob = await awaitOcrStep(new Promise<Blob>((resolve, reject) => {
              image.toBlob((value) => value ? resolve(value) : reject(new Error("The OCR scan could not be encoded. No text was added.")), "image/png");
            }), controller.signal);
            pngBytes += blob.size;
            if (pngBytes > 64 * 1024 * 1024) throw new Error("The rendered OCR page inputs exceed 64 MiB. Choose a lower scan resolution; no text was added.");
            controller.signal.throwIfAborted();
            progress({ stage: "recognizing", page: currentPage, completed, total: pages.length, fraction: bandIndex / bandCount });
            const output = await worker.recognize(await awaitOcrStep(blob.arrayBuffer(), controller.signal));
            controller.signal.throwIfAborted();
            const recognized = recognitionWords(output, { ...target, pixelHeight: band.height });
            words.push(...shiftOcrWords(recognized, band.top, lineOffset));
            if (words.length > 10_000) throw new Error(`Page ${target.page + 1} exceeds the 10,000-word OCR limit. No text was added.`);
            if (recognized.length) lineOffset += recognized[recognized.length - 1].line + 1;
            bandIndex++;
          } finally {
            if (image !== canvas) { image.width = 0; image.height = 0; }
          }
        }
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
