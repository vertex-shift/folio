import type { PDFDocumentLoadingTask, PDFDocumentProxy, PDFPageProxy, RenderTask } from "pdfjs-dist";
import type { TextContent } from "pdfjs-dist/types/src/display/api";
import { loadPdf } from "../../lib/pdf";
import { mapText } from "../../lib/search";
import { alignedPages, ComparisonBudget, LIMITS } from "./model";
import type { ComparisonPlan, ComparisonProgress, ComparisonResult, PagePreview, PageResult } from "./model";
import type { WorkerInput, WorkerOutput } from "./comparison.worker";

interface Operations {
  bytes(side: "before" | "after", signal: AbortSignal): Promise<Uint8Array>;
  check(): void;
  verify(): Promise<void>;
}

function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const abort = () => reject(signal.reason);
    signal.addEventListener("abort", abort, { once: true });
    promise.then(resolve, reject).finally(() => signal.removeEventListener("abort", abort));
    if (signal.aborted) abort();
  });
}

async function extract(page: PDFPageProxy, signal: AbortSignal, check: () => void, budget: ComparisonBudget): Promise<string> {
  signal.throwIfAborted(); check();
  const stream: ReadableStream<TextContent> = page.streamTextContent({ disableNormalization: true });
  const reader = stream.getReader();
  const parts: string[] = [];
  const encoder = new TextEncoder();
  let bytes = 0, items = 0, finished = false;
  try {
    for (;;) {
      signal.throwIfAborted(); check();
      const chunk = await abortable(reader.read(), signal);
      signal.throwIfAborted(); check();
      if (chunk.done) { finished = true; break; }
      items += chunk.value.items.length;
      if (items > 40_000) throw new Error("A page exceeds 40,000 extraction items. No text was truncated.");
      const text = mapText(chunk.value.items.filter((item) => "str" in item)).text;
      const length = encoder.encode(text).length;
      bytes += length;
      if (bytes > LIMITS.pageTextBytes) throw new Error("A page exceeds 256 KiB extracted text. No text was truncated.");
      budget.addTextBytes(length);
      parts.push(text);
    }
    return parts.join("");
  } finally {
    let timer: number | undefined;
    try {
      if (!finished) {
        const cancellation = reader.cancel(new Error("Comparison text extraction stopped before completion."));
        const timeout = new Promise<never>((_resolve, reject) => {
          timer = window.setTimeout(() => reject(new Error("The comparison text stream did not acknowledge cancellation; its isolated renderer will be destroyed.")), 1500);
        });
        await abortable(Promise.race([cancellation, timeout]), signal);
      }
    }
    finally { window.clearTimeout(timer); reader.releaseLock(); }
  }
}

function canvas(width: number, height: number): HTMLCanvasElement {
  const element = document.createElement("canvas");
  element.width = width; element.height = height;
  return element;
}

async function render(
  page: PDFPageProxy | null, target: HTMLCanvasElement, dpi: number, signal: AbortSignal, check: () => void,
): Promise<Uint8ClampedArray> {
  signal.throwIfAborted(); check();
  const context = target.getContext("2d", { alpha: false, colorSpace: "srgb", willReadFrequently: true });
  if (!context) throw new Error("A comparison canvas could not be created.");
  context.fillStyle = "#ffffff"; context.fillRect(0, 0, target.width, target.height);
  if (page) {
    let task: RenderTask | undefined;
    const cancel = () => task?.cancel();
    signal.addEventListener("abort", cancel, { once: true });
    try {
      task = page.render({ canvas: target, canvasContext: context,
        viewport: page.getViewport({ scale: dpi / 72 }), background: "#ffffff", intent: "display" });
      await task.promise;
      signal.throwIfAborted(); check();
    } finally { signal.removeEventListener("abort", cancel); }
  }
  return context.getImageData(0, 0, target.width, target.height).data;
}

async function preview(source: HTMLCanvasElement, signal: AbortSignal, budget: ComparisonBudget): Promise<Blob> {
  const scale = Math.min(1, LIMITS.previewDimension / Math.max(source.width, source.height));
  const target = canvas(Math.max(1, Math.round(source.width * scale)), Math.max(1, Math.round(source.height * scale)));
  try {
    const context = target.getContext("2d", { colorSpace: "srgb" });
    if (!context) throw new Error("A comparison preview canvas could not be created.");
    context.drawImage(source, 0, 0, target.width, target.height);
    const blob = await abortable(new Promise<Blob>((resolve, reject) => {
      target.toBlob((value) => value ? resolve(value) : reject(new Error("A comparison preview could not be encoded.")), "image/png");
    }), signal);
    signal.throwIfAborted();
    if (blob.type !== "image/png") throw new Error("The renderer did not produce a PNG comparison preview.");
    budget.addPreview(blob.size);
    return blob;
  } finally { target.width = 0; target.height = 0; }
}

function processPage(worker: Worker, input: WorkerInput, signal: AbortSignal): Promise<Extract<WorkerOutput, { ok: true }>> {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const abort = () => { dispose(); reject(signal.reason); };
    const timeout = window.setTimeout(() => {
      dispose();
      reject(new Error("The comparison worker exceeded its 10-second page budget. No incomplete result was reported."));
    }, 10_000);
    const dispose = () => {
      window.clearTimeout(timeout);
      signal.removeEventListener("abort", abort);
      worker.onmessage = null; worker.onerror = null; worker.onmessageerror = null;
    };
    signal.addEventListener("abort", abort, { once: true });
    worker.onmessage = (event: MessageEvent<WorkerOutput>) => {
      dispose();
      if (event.data.ok) resolve(event.data);
      else reject(new Error(event.data.error));
    };
    worker.onerror = (event) => { event.preventDefault(); dispose(); reject(new Error(`Comparison worker failed: ${event.message}`)); };
    worker.onmessageerror = () => { dispose(); reject(new Error("The comparison worker returned unreadable results.")); };
    try { worker.postMessage(input, [input.beforePixels.buffer, input.afterPixels.buffer]); }
    catch (error) { dispose(); reject(error); }
  });
}

export async function runComparison(
  plan: ComparisonPlan, operations: Operations, signal: AbortSignal, progress: (value: ComparisonProgress) => void,
): Promise<ComparisonResult> {
  const loading: PDFDocumentLoadingTask[] = [];
  let worker: Worker | undefined;
  const check = () => { signal.throwIfAborted(); operations.check(); };
  const verify = async () => { check(); await operations.verify(); check(); };
  const budget = new ComparisonBudget();
  const pages: PageResult[] = [];
  const previews: PagePreview[] = [];
  try {
    const expected = alignedPages(plan.before.pages, plan.after.pages, plan.settings);
    if (expected.length !== plan.pages.length || expected.some((page, index) => {
      const actual = plan.pages[index];
      return page.index !== actual.index || page.width !== actual.width || page.height !== actual.height;
    })) throw new Error("Comparison plan geometry disagrees with its source snapshots.");
    progress({ page: 0, count: plan.pages.length, stage: "snapshots" });
    await verify();
    const documents: PDFDocumentProxy[] = [];
    for (const [side, source] of [["before", plan.before], ["after", plan.after]] as const) {
      check();
      const bytes = await operations.bytes(side, signal);
      check(); await verify();
      if (bytes.length > LIMITS.sourceBytes) throw new Error("A comparison snapshot exceeds 64 MiB.");
      const task = loadPdf(bytes);
      loading.push(task);
      const pdf = await abortable(task.promise, signal);
      check();
      if (pdf.numPages !== source.pageCount) throw new Error("The renderer and comparison plan disagree on page count. Reopen this PDF.");
      documents.push(pdf);
      await verify();
    }
    worker = new Worker(new URL("./comparison.worker.ts", import.meta.url), { type: "module" });
    for (const target of plan.pages) {
      await verify();
      const pageProxies: PDFPageProxy[] = [];
      const canvases: HTMLCanvasElement[] = [];
      try {
        const input: WorkerInput = {
          beforeText: "", afterText: "", beforePixels: new Uint8ClampedArray(),
          afterPixels: new Uint8ClampedArray(), width: target.width, height: target.height, tolerance: plan.settings.tolerance,
        };
        const images: (Blob | null)[] = [];
        for (const [index, source] of [plan.before, plan.after].entries()) {
          check();
          const page = target.index < source.pageCount ? await abortable(documents[index].getPage(target.index + 1), signal) : null;
          if (page) pageProxies.push(page);
          check();
          if (page) {
            const viewport = page.getViewport({ scale: 1 });
            const geometry = source.pages[target.index];
            if (Math.abs(viewport.width - geometry.widthPoints) > 0.01 || Math.abs(viewport.height - geometry.heightPoints) > 0.01
              || page.rotate !== geometry.rotation || Math.abs(page.userUnit - geometry.userUnit) > 0.00001
              || page.view.some((value, i) => Math.abs(value - geometry.bounds[i]) > 0.01)) {
              throw new Error(`Page ${target.index + 1} has unmatched renderer/core geometry. It was not silently aligned or declared equal.`);
            }
          }
          progress({ page: target.index + 1, count: plan.pages.length, stage: "text" });
          const text = page ? await extract(page, signal, check, budget) : "";
          check();
          progress({ page: target.index + 1, count: plan.pages.length, stage: "pixels" });
          const image = canvas(target.width, target.height);
          canvases.push(image);
          const pixels = await render(page, image, plan.settings.dpi, signal, check);
          check();
          images.push(page ? await preview(image, signal, budget) : null);
          check();
          if (index === 0) { input.beforeText = text; input.beforePixels = pixels; }
          else { input.afterText = text; input.afterPixels = pixels; }
        }
        const computed = await processPage(worker, input, signal);
        check(); await verify();
        budget.addText(computed.text);
        const mask = canvas(target.width, target.height);
        canvases.push(mask);
        const context = mask.getContext("2d");
        if (!context) throw new Error("A difference-mask canvas could not be created.");
        context.putImageData(new ImageData(new Uint8ClampedArray(computed.mask), target.width, target.height), 0, 0);
        const maskPreview = await preview(mask, signal, budget);
        check();
        pages.push({ index: target.index, text: computed.text, pixels: computed.pixels });
        previews.push({ before: images[0], after: images[1], mask: maskPreview });
      } finally {
        for (const image of canvases) { image.width = 0; image.height = 0; }
        for (const page of pageProxies) page.cleanup();
      }
    }
    progress({ page: plan.pages.length, count: plan.pages.length, stage: "finishing" });
    await verify();
    return { plan, pages, previews };
  } finally {
    worker?.terminate();
    const cleanup = await Promise.allSettled(loading.map((task) => task.destroy()));
    const failures = cleanup.filter((entry) => entry.status === "rejected").map((entry) => entry.reason);
    if (failures.length) throw new AggregateError(failures, "Comparison PDF.js snapshot cleanup failed.");
  }
}
