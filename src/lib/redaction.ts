import type { PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import type { PdfRect, RedactionPlan } from "./types.ts";

export const MAX_REDACTION_MARKS = 500;
const MAX_PNG_BYTES = 64 * 1024 * 1024;

export function validateRedactionRect(rect: PdfRect, bounds: readonly number[]): PdfRect {
  if (bounds.length !== 4 || !bounds.every(Number.isFinite) || bounds[2] <= bounds[0] || bounds[3] <= bounds[1]
    || !rect.every(Number.isFinite) || rect[2] <= rect[0] || rect[3] <= rect[1]) {
    throw new Error("Enter a redaction area with finite coordinates and a positive width and height.");
  }
  if (rect[0] < bounds[0] - 0.01 || rect[1] < bounds[1] - 0.01
    || rect[2] > bounds[2] + 0.01 || rect[3] > bounds[3] + 0.01) {
    throw new Error("Keep the entire redaction area inside the visible page.");
  }
  const clipped: PdfRect = [Math.max(bounds[0], rect[0]), Math.max(bounds[1], rect[1]),
    Math.min(bounds[2], rect[2]), Math.min(bounds[3], rect[3])];
  if (clipped[2] <= clipped[0] || clipped[3] <= clipped[1]) {
    throw new Error("The redaction area must cover part of the visible page.");
  }
  return clipped;
}

export function redactionFromFields(fields: readonly string[], bounds: readonly number[]): PdfRect {
  if (fields.length !== 4 || fields.some((value) => !value.trim())) {
    throw new Error("Enter all four coordinates before adding a redaction area.");
  }
  const [left, bottom, width, height] = fields.map(Number);
  if (width <= 0 || height <= 0) {
    throw new Error("Redaction width and height must be greater than zero.");
  }
  return validateRedactionRect([left, bottom, left + width, bottom + height], bounds);
}

export async function renderRedactionPages(
  pdf: PDFDocumentProxy,
  plan: RedactionPlan,
  upload: (jobId: number, page: number, png: Uint8Array) => Promise<void>,
  progress: (page: number, count: number) => void,
  signal: AbortSignal,
): Promise<void> {
  if (pdf.numPages !== plan.pages.length) {
    throw new Error("The renderer and redaction plan disagree on the page count. Reopen this PDF.");
  }
  for (const [index, target] of plan.pages.entries()) {
    signal.throwIfAborted();
    progress(index + 1, plan.pages.length);
    const page = await pdf.getPage(index + 1);
    const viewport = page.getViewport({ scale: plan.dpi / 72 });
    if (page.rotate !== target.rotation || Math.abs(page.userUnit - target.userUnit) > 0.0001
      || page.view.some((value, index) => Math.abs(value - target.sourceBounds[index]) > 0.01)
      || Math.abs(viewport.width - target.widthPoints * plan.dpi / 72) > 0.1
      || Math.abs(viewport.height - target.heightPoints * plan.dpi / 72) > 0.1) {
      throw new Error(`Page ${index + 1} has inconsistent rendering geometry. No redacted copy was created.`);
    }
    const canvas = document.createElement("canvas");
    canvas.width = target.pixelWidth;
    canvas.height = target.pixelHeight;
    let render: RenderTask | undefined;
    const cancel = () => render?.cancel();
    signal.addEventListener("abort", cancel, { once: true });
    try {
      signal.throwIfAborted();
      const context = canvas.getContext("2d", { alpha: false, colorSpace: "srgb" });
      if (!context) throw new Error("The page canvas is unavailable. No redacted copy was created.");
      render = page.render({ canvas, canvasContext: context, viewport, background: "#ffffff" });
      await render.promise;
      signal.throwIfAborted();
      context.globalAlpha = 1;
      context.globalCompositeOperation = "source-over";
      context.fillStyle = "#000000";
      for (const mask of target.masks) context.fillRect(mask.x, mask.y, mask.width, mask.height);
      const png = await new Promise<Blob>((resolve, reject) => {
        canvas.toBlob((blob) => blob ? resolve(blob)
          : reject(new Error(`Page ${index + 1} could not be encoded. No redacted copy was created.`)), "image/png");
      });
      signal.throwIfAborted();
      if (png.size > MAX_PNG_BYTES) throw new Error(`Page ${index + 1} exceeds the 64 MiB rendered-page limit. Try 150 DPI.`);
      await upload(plan.jobId, index, new Uint8Array(await png.arrayBuffer()));
      signal.throwIfAborted();
    } finally {
      signal.removeEventListener("abort", cancel);
      canvas.width = 0;
      canvas.height = 0;
    }
  }
}
