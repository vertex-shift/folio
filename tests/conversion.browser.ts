import { PDFDocument, StandardFonts } from "pdf-lib";
import { loadPdf } from "../src/lib/pdf";
import { mapText } from "../src/lib/search";
import { extractDocxPages } from "../src/features/conversion/docx";
import type { OutputPlan } from "../src/lib/output";
import type { TextContent } from "pdfjs-dist/types/src/display/api";

function verify(condition: boolean, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

export async function syntheticConversionInputs() {
  const image = document.createElement("canvas");
  image.width = 96; image.height = 72;
  const context = image.getContext("2d");
  verify(context !== null, "The synthetic image canvas is unavailable.");
  context.fillStyle = "#ff0000"; context.fillRect(0, 0, 48, 36);
  context.fillStyle = "#00ff00"; context.fillRect(48, 0, 48, 36);
  context.fillStyle = "#0000ff"; context.fillRect(0, 36, 48, 36);
  context.fillStyle = "#ffff00"; context.fillRect(48, 36, 48, 36);
  const jpeg = image.toDataURL("image/jpeg", 0.95).split(",")[1];
  context.clearRect(48, 0, 48, 36);
  context.fillStyle = "rgba(0,255,0,0.5)"; context.fillRect(48, 0, 48, 36);
  context.clearRect(48, 36, 48, 36);
  const alpha = image.toDataURL("image/png").split(",")[1];
  image.width = 0; image.height = 0;
  const pdf = await PDFDocument.create();
  const font = await pdf.embedFont(StandardFonts.Helvetica);
  const markers = ["83412", "29407", "72045", "49211"];
  for (const [index, marker] of markers.entries()) {
    const page = pdf.addPage([612, 792]);
    page.drawText(`Synthetic page ${index + 1} marker ${marker}`, { x: 48, y: 700, size: 16, font });
    page.drawText(`Literal <w:t>&"quotes" and 'apostrophe'`, { x: 48, y: 650, size: 12, font });
  }
  const blank = await PDFDocument.create();
  blank.addPage([612, 792]);
  return { alpha, jpeg, pdf: Array.from(await pdf.save()), blank: Array.from(await blank.save()) };
}

export async function inspectConversionText(bytes: number[]) {
  const loading = loadPdf(new Uint8Array(bytes));
  try {
    const pdf = await loading.promise;
    const pages: string[] = [];
    const bounds: { width: number; height: number; outOfBounds: number; minLeft: number | null; maxRight: number | null }[] = [];
    for (let index = 0; index < pdf.numPages; index++) {
      const page = await pdf.getPage(index + 1);
      const viewport = page.getViewport({ scale: 1 });
      const text = await page.getTextContent();
      const items = text.items.filter((item) => "str" in item);
      pages.push(mapText(items).text);
      let outOfBounds = 0;
      let minLeft = Infinity, maxRight = -Infinity;
      for (const item of items) {
        const point = viewport.convertToViewportPoint(item.transform[4], item.transform[5]);
        minLeft = Math.min(minLeft, point[0]);
        maxRight = Math.max(maxRight, point[0] + item.width);
        if (point[0] < -0.1 || point[0] > viewport.width + 0.1 || point[1] < -0.1 || point[1] > viewport.height + 0.1) outOfBounds++;
      }
      bounds.push({ width: viewport.width, height: viewport.height, outOfBounds,
        minLeft: Number.isFinite(minLeft) ? minLeft : null, maxRight: Number.isFinite(maxRight) ? maxRight : null });
      page.cleanup();
    }
    return { pages, bounds };
  } finally { await loading.destroy(); }
}

export async function inspectConversionImages(input: { pdf: number[]; images: number[][]; margin: number }) {
  const loading = loadPdf(new Uint8Array(input.pdf));
  try {
    const pdf = await loading.promise;
    verify(pdf.numPages === input.images.length, "Image creation did not produce one page per ordered input.");
    const results = [];
    for (const [index, imageBytes] of input.images.entries()) {
      const page = await pdf.getPage(index + 1);
      const bitmap = await createImageBitmap(new Blob([new Uint8Array(imageBytes)]));
      const viewport = page.getViewport({ scale: 1 });
      const canvas = document.createElement("canvas");
      canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
      const context = canvas.getContext("2d", { willReadFrequently: true });
      verify(context !== null, "The independent PDF render canvas is unavailable.");
      const reference = document.createElement("canvas");
      reference.width = canvas.width; reference.height = canvas.height;
      const expected = reference.getContext("2d", { willReadFrequently: true });
      verify(expected !== null, "The independent original-image canvas is unavailable.");
      try {
        await page.render({ canvas, canvasContext: context, viewport, background: "#ffffff" }).promise;
        expected.fillStyle = "#ffffff"; expected.fillRect(0, 0, reference.width, reference.height);
        const scale = Math.min((viewport.width - input.margin * 2) / bitmap.width, (viewport.height - input.margin * 2) / bitmap.height);
        const width = bitmap.width * scale, height = bitmap.height * scale;
        const left = (viewport.width - width) / 2, top = (viewport.height - height) / 2;
        expected.drawImage(bitmap, left, top, width, height);
        const samples = [];
        for (const [x, y] of [[0.25, 0.25], [0.75, 0.25], [0.25, 0.75], [0.75, 0.75]]) {
          const px = Math.floor(left + width * x), py = Math.floor(top + height * y);
          const actual = [...context.getImageData(px, py, 1, 1).data];
          const wanted = [...expected.getImageData(px, py, 1, 1).data];
          verify(actual.every((channel, position) => Math.abs(channel - wanted[position]) <= 8),
            `Page ${index + 1} orientation/color/alpha differs: ${actual} vs ${wanted}.`);
          samples.push({ actual, expected: wanted });
        }
        const actual = context.getImageData(0, 0, canvas.width, canvas.height).data;
        const wanted = expected.getImageData(0, 0, canvas.width, canvas.height).data;
        let delta = 0, different = 0;
        for (let offset = 0; offset < actual.length; offset += 4) {
          let maximum = 0;
          for (let channel = 0; channel < 3; channel++) {
            const value = Math.abs(actual[offset + channel] - wanted[offset + channel]);
            delta += value; maximum = Math.max(maximum, value);
          }
          if (maximum > 12) different++;
        }
        const meanChannelDelta = delta / (canvas.width * canvas.height * 3);
        const changedFraction = different / (canvas.width * canvas.height);
        verify(meanChannelDelta < 3 && changedFraction < 0.02,
          `Page ${index + 1} differs from independently oriented/composited pixels: ${meanChannelDelta}/${changedFraction}.`);
        results.push({ page: index + 1, sourceWidth: bitmap.width, sourceHeight: bitmap.height,
          pageWidth: viewport.width, pageHeight: viewport.height, samples, meanChannelDelta, changedFraction });
      } finally {
        bitmap.close(); canvas.width = 0; canvas.height = 0; reference.width = 0; reference.height = 0; page.cleanup();
      }
    }
    return results;
  } finally { await loading.destroy(); }
}

export async function checkDocxExtractionCancellation(bytes: number[], blank: number[]) {
  const loading = loadPdf(new Uint8Array(bytes));
  const emptyLoading = loadPdf(new Uint8Array(blank));
  const plan: OutputPlan = { jobId: 1, documentId: 1, revision: 0, kind: "docx", pages: [2, 0, 1],
    dpi: null, quality: null, suggestedName: "Text.docx", sourceBytes: bytes.length, passwordProtected: false };
  try {
    const pdf = await loading.promise;
    const abort = new AbortController();
    let staged = 0, canceled = false, midStreamCanceled = false, empty = false, stale = false;
    try {
      await extractDocxPages(pdf, plan, async () => { staged++; }, () => abort.abort(), abort.signal, () => {});
    } catch (error) { canceled = error instanceof DOMException && error.name === "AbortError"; }
    verify(canceled && staged === 0, "A canceled DOCX extraction staged text.");
    const firstPage = await pdf.getPage(3);
    const originalStream = firstPage.streamTextContent.bind(firstPage);
    const midStreamAbort = new AbortController();
    firstPage.streamTextContent = (parameters) => originalStream(parameters).pipeThrough(new TransformStream<TextContent, TextContent>({
      transform(chunk, controller) { controller.enqueue(chunk); midStreamAbort.abort(); },
    }));
    try {
      await extractDocxPages(pdf, plan, async () => { staged++; }, () => {}, midStreamAbort.signal, () => {});
    } catch (error) { midStreamCanceled = error instanceof DOMException && error.name === "AbortError"; }
    finally { firstPage.streamTextContent = originalStream; }
    verify(midStreamCanceled && staged === 0, "A mid-stream canceled DOCX extraction staged text.");
    try {
      await extractDocxPages(pdf, plan, async () => { staged++; }, () => {}, new AbortController().signal,
        () => { throw new Error("Synthetic stale source"); });
    } catch (error) { stale = error instanceof Error && error.message === "Synthetic stale source"; }
    verify(stale && staged === 0, "A stale DOCX extraction staged text.");
    const blankPdf = await emptyLoading.promise;
    try {
      await extractDocxPages(blankPdf, { ...plan, pages: [0] }, async () => {}, () => {}, new AbortController().signal, () => {});
    } catch (error) { empty = error instanceof Error && error.message.includes("No editable text"); }
    verify(empty, "An empty scan produced success-shaped DOCX text.");
    return { canceled, midStreamCanceled, stale, empty, staged };
  } finally { await loading.destroy(); await emptyLoading.destroy(); }
}
