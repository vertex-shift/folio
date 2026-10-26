import type { PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import { AnnotationMode } from "pdfjs-dist/legacy/build/pdf.mjs";
import { ARCHIVE_LIMITS, validateArchiveGeometry } from "./model";
import type { ArchivePlan } from "./model";

export async function renderArchivePages(
  pdf: PDFDocumentProxy, plan: ArchivePlan,
  stage: (archiveId: number, index: number, bytes: Uint8Array, signal: AbortSignal) => Promise<void>,
  progress: (page: number, count: number) => void, signal: AbortSignal, check: () => void,
) {
  if (pdf.numPages !== plan.sourcePageCount || !plan.pages.length || plan.pages.length > ARCHIVE_LIMITS.pages) {
    throw new Error("The renderer and archival plan disagree on source pages. No copy was saved.");
  }
  let totalPixels = 0;
  for (const [index, target] of plan.pages.entries()) {
    signal.throwIfAborted(); check();
    progress(index + 1, plan.pages.length);
    const page = await pdf.getPage(target.sourcePage + 1);
    let canvas: HTMLCanvasElement | undefined;
    let render: RenderTask | undefined;
    const cancel = () => render?.cancel();
    signal.addEventListener("abort", cancel, { once: true });
    try {
      signal.throwIfAborted(); check();
      const base = page.getViewport({ scale: 1 });
      validateArchiveGeometry(target, { rotate: page.rotate, userUnit: page.userUnit, view: page.view, width: base.width, height: base.height }, plan.dpi);
      const viewport = page.getViewport({ scale: plan.dpi / 72 });
      if (Math.ceil(viewport.width) !== target.pixelWidth || Math.ceil(viewport.height) !== target.pixelHeight) {
        throw new Error(`Source page ${target.sourcePage + 1} crosses a renderer pixel-rounding boundary. No archival copy was saved; pages are never clipped or downscaled.`);
      }
      totalPixels += target.pixelWidth * target.pixelHeight;
      if (totalPixels > ARCHIVE_LIMITS.totalPixels) throw new Error("The archival job exceeds 100 megapixels. No pages were skipped or downscaled.");
      canvas = document.createElement("canvas");
      canvas.width = target.pixelWidth; canvas.height = target.pixelHeight;
      const context = canvas.getContext("2d", { alpha: false, colorSpace: "srgb" });
      if (!context || typeof context.getContextAttributes !== "function" || context.getContextAttributes().colorSpace !== "srgb") {
        throw new Error("An opaque sRGB archival canvas is unavailable. No copy was saved.");
      }
      render = page.render({
        canvas, canvasContext: context, viewport,
        background: "#ffffff", intent: "display", annotationMode: AnnotationMode.ENABLE,
      });
      await render.promise;
      signal.throwIfAborted(); check();
      const png = await new Promise<Blob>((resolve, reject) => canvas!.toBlob((blob) => blob ? resolve(blob)
        : reject(new Error(`Source page ${target.sourcePage + 1} could not be encoded. No copy was saved.`)), "image/png"));
      signal.throwIfAborted(); check();
      if (png.type !== "image/png" || png.size > ARCHIVE_LIMITS.pngBytes) throw new Error("An archival page cannot be encoded as PNG within 64 MiB.");
      const bytes = new Uint8Array(await png.arrayBuffer());
      signal.throwIfAborted(); check();
      await stage(plan.archiveId, index, bytes, signal);
      signal.throwIfAborted(); check();
    } finally {
      signal.removeEventListener("abort", cancel);
      if (canvas) { canvas.width = 0; canvas.height = 0; }
      page.cleanup();
    }
  }
}
