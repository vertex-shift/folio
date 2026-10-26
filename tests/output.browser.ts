import { PDFDocument, PDFName, PDFNumber, StandardFonts, degrees, rgb } from "pdf-lib";
import { loadPdf } from "../src/lib/pdf";
import { assertOutputCurrent, rasterGeometry, renderOutputImages, renderOutputText } from "../src/lib/output";
import { PreparedPrintSurface, requestBrowserPrint } from "../src/lib/printSurface";
import type { OutputPlan } from "../src/lib/output";

function verify(condition: boolean, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

export async function checkOutputRenderer() {
  const fixture = await PDFDocument.create();
  const font = await fixture.embedFont(StandardFonts.Helvetica);
  for (let index = 0; index < 24; index++) {
    const page = fixture.addPage([612, 792]);
    page.drawText(`Synthetic page ${index + 1}`, { font, size: 16, x: 36, y: 130 });
    page.drawRectangle({ x: 36, y: 72, width: 24, height: 24, color: rgb(0.85, 0.18, 0.1) });
    if (index === 1) {
      page.setMediaBox(-20, -10, 420, 260);
      page.setCropBox(10, 20, 340, 200);
      page.setRotation(degrees(90));
      page.node.set(PDFName.of("UserUnit"), PDFNumber.of(2));
    } else if (index === 2) {
      page.setCropBox(0, 0, 300, 400);
      page.setRotation(degrees(270));
      page.node.set(PDFName.of("UserUnit"), PDFNumber.of(1.5));
    }
  }
  const bytes = await fixture.save();
  const loading = loadPdf(bytes);
  const source = { id: 1, revision: 0 };
  const plan: OutputPlan = {
    jobId: 1, documentId: 1, revision: 0, kind: "text", pages: [23, 1, 0],
    dpi: null, quality: null, suggestedName: "Synthetic.txt",
    sourceBytes: bytes.length, passwordProtected: false,
  };
  const signal = new AbortController().signal;
  const check = () => assertOutputCurrent(plan, source, false);
  const progress = () => {};
  let surface: PreparedPrintSurface | undefined;
  try {
    const pdf = await loading.promise;
    const second = rasterGeometry((await pdf.getPage(2)).getViewport({ scale: 1 }), 150, 2);
    const third = rasterGeometry((await pdf.getPage(3)).getViewport({ scale: 1 }), 96, 3);
    verify(second.widthPoints === 400 && second.heightPoints === 680, "Crop/90-degree rotation/UserUnit geometry changed.");
    verify(third.widthPoints === 600 && third.heightPoints === 450, "270-degree rotation geometry changed.");
    const text: string[] = [];
    await renderOutputText(pdf, plan, async (_job, index, bytes) => {
      verify(index === text.length, "Text pages were prepared concurrently or reordered.");
      text.push(new TextDecoder().decode(bytes));
    }, progress, signal, check);
    verify(JSON.stringify(text) === JSON.stringify(["Synthetic page 24", "Synthetic page 2", "Synthetic page 1"]),
      `Wrong selectable text: ${JSON.stringify(text)}`);

    let pngCount = 0;
    let pixel: number[] = [];
    const imagePages = [0, 1, 2, 23];
    await renderOutputImages(pdf, { ...plan, kind: "png", pages: imagePages, dpi: 150 },
      async (_job, index, bytes) => {
        verify(index === pngCount, "Image preparation was not sequential.");
        const sourcePage = await pdf.getPage(imagePages[index] + 1);
        const geometry = rasterGeometry(sourcePage.getViewport({ scale: 1 }), 150, imagePages[index] + 1);
        const point = sourcePage.getViewport({ scale: 150 / 72 }).convertToViewportPoint(48, 84);
        const bitmap = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: "image/png" }));
        try {
          verify(bitmap.width === geometry.pixelWidth && bitmap.height === geometry.pixelHeight, "PNG dimensions do not match the requested DPI.");
          const canvas = document.createElement("canvas");
          canvas.width = bitmap.width; canvas.height = bitmap.height;
          try {
            const context = canvas.getContext("2d");
            verify(context !== null, "Pixel-check canvas unavailable.");
            context.drawImage(bitmap, 0, 0);
            pixel = Array.from(context.getImageData(Math.floor(point[0]), Math.floor(point[1]), 1, 1).data);
            verify(JSON.stringify(pixel) === "[217,46,26,255]", "The PDF rectangle did not become real exported pixels.");
            verify(Array.from(context.getImageData(0, 0, 1, 1).data).every((channel) => channel === 255), "The page background was not opaque white.");
          } finally { canvas.width = 0; canvas.height = 0; }
        } finally { bitmap.close(); }
        pngCount += 1;
      }, progress, signal, check);
    let jpegBytes = 0;
    await renderOutputImages(pdf, { ...plan, kind: "jpeg", pages: [23], dpi: 96, quality: 80 },
      async (_job, _index, bytes) => {
        verify(bytes[0] === 255 && bytes[1] === 216 && bytes.at(-2) === 255 && bytes.at(-1) === 217, "JPEG encoding was mislabeled.");
        const bitmap = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: "image/jpeg" }));
        verify(bitmap.width === 816 && bitmap.height === 1056, "JPEG dimensions changed.");
        bitmap.close();
        jpegBytes = bytes.length;
      }, progress, signal, check);

    surface = new PreparedPrintSurface();
    await renderOutputImages(pdf, { ...plan, kind: "print", pages: [23, 1, 2], dpi: 96 },
      async () => {}, progress, signal, check, (image) => surface!.append(image));
    surface.finish(3);
    verify(document.querySelectorAll(".folio-print-sheet").length === 3, "Print snapshot omitted off-screen pages.");
    surface.markRequested();
    window.dispatchEvent(new Event("beforeprint"));
    let retained = false;
    try { surface.dispose(); } catch { retained = true; }
    verify(retained && Boolean(document.querySelector(".folio-print-surface")), "An open print job was cleaned up prematurely.");
    surface.dispose(true);
    surface = undefined;
    verify(!document.querySelector(".folio-print-surface"), "Explicit print cleanup left a surface behind.");

    const originalPrint = window.print;
    let unsupportedPrint = false;
    try {
      window.print = () => { window.dispatchEvent(new Event("beforeprint")); };
      await requestBrowserPrint();
      window.print = () => {};
      try { await requestBrowserPrint(); }
      catch (error) { unsupportedPrint = error instanceof Error && error.message.includes("did not acknowledge"); }
    } finally { window.print = originalPrint; }
    verify(unsupportedPrint, "A no-op print operation was reported as successful.");

    const blank = await PDFDocument.create();
    blank.addPage([612, 792]);
    const emptyLoading = loadPdf(await blank.save());
    let noText = false;
    try {
      const empty = await emptyLoading.promise;
      try {
        await renderOutputText(empty, { ...plan, pages: [0] }, async () => {}, progress, signal, check);
      } catch (error) { noText = error instanceof Error && error.message.includes("No selectable text"); }
    } finally { await emptyLoading.destroy(); }
    verify(noText, "Empty pages produced a success-shaped text export.");

    const abort = new AbortController();
    let submitted = 0;
    let canceled = false;
    try {
      await renderOutputImages(pdf, { ...plan, kind: "png", dpi: 150 },
        async () => { submitted += 1; }, () => abort.abort(), abort.signal, check);
    } catch (error) { canceled = error instanceof DOMException && error.name === "AbortError"; }
    verify(canceled && submitted === 0, "Cancellation submitted a partial output page.");
    let stale = false;
    try {
      await renderOutputImages(pdf, { ...plan, kind: "png", dpi: 150 }, async () => { submitted += 1; },
        () => { source.revision = 1; }, signal, check);
    } catch (error) { stale = error instanceof Error && error.message.includes("document changed"); }
    verify(stale && submitted === 0, "A stale revision submitted a page.");
    return { pages: pdf.numPages, cropRotationGeometry: [second, third], pngPages: pngCount, pixel, jpegBytes, printPages: 3, retained, unsupportedPrint, noText, canceled, stale };
  } finally {
    surface?.dispose();
    await loading.destroy();
  }
}
