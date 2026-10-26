import {
  PDFDocument, PDFName, PDFNumber, StandardFonts, concatTransformationMatrix,
  degrees, drawObject, popGraphicsState, pushGraphicsState, rgb,
} from "pdf-lib";
import { loadPdf } from "../src/lib/pdf";
import { renderArchivePages } from "../src/features/preflight/render";
import { archiveBridge } from "../src/features/preflight/bridge";
import type { ArchivePlan } from "../src/features/preflight/model";

function verify(condition: boolean, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

export async function archiveFixtures() {
  const pdf = await PDFDocument.create();
  const font = await pdf.embedFont(StandardFonts.Helvetica);
  const stamp = document.createElement("canvas");
  stamp.width = 12; stamp.height = 12;
  const pixels = stamp.getContext("2d")!.createImageData(12, 12);
  for (let y = 0; y < 12; y++) for (let x = 0; x < 12; x++) {
    const offset = (y * 12 + x) * 4;
    pixels.data.set([x * 20, 128, y * 20, [0, 64, 128, 255][(x + y) % 4]], offset);
  }
  stamp.getContext("2d")!.putImageData(pixels, 0, 0);
  const blob = await new Promise<Blob>((resolve) => stamp.toBlob((value) => resolve(value!), "image/png"));
  const image = await pdf.embedPng(await blob.arrayBuffer());
  stamp.width = stamp.height = 0;
  for (const [index, unit] of [1, 1.5, 2, 0.75].entries()) {
    const page = pdf.addPage([320, 220]);
    page.setMediaBox(-20, -10, 320, 220);
    page.setCropBox(10, 20, 240, 144);
    page.setRotation(degrees(index * 90));
    if (index !== 0) page.node.set(PDFName.of("UserUnit"), PDFNumber.of(unit));
    page.drawRectangle({ x: 10, y: 20, width: 240, height: 144, color: rgb(1, 1, 1) });
    page.drawRectangle({ x: 12, y: 22, width: 18, height: 18, color: rgb(1, 0, 0) });
    page.drawRectangle({ x: 230, y: 22, width: 18, height: 18, color: rgb(0, 1, 0) });
    page.drawRectangle({ x: 12, y: 144, width: 18, height: 18, color: rgb(0, 0, 1) });
    page.drawText(`ORIGINAL ARCHIVE FIXTURE ${index + 1}`, { x: 38, y: 122, font, size: 9 });
    page.drawText("Text and vectors must not survive as objects.", { x: 38, y: 106, font, size: 8 });
    page.drawImage(image, { x: 38, y: 58, width: 36, height: 36 });
    const appearance = pdf.context.register(pdf.context.flateStream("q 0.15 0.3 0.75 rg 0 0 36 16 re f Q", {
      Type: "XObject", Subtype: "Form", BBox: [0, 0, 36, 16], Resources: {},
    }));
    const widget = pdf.context.register(pdf.context.obj({
      Type: "Annot", Subtype: "Stamp", Rect: [180, 40, 216, 56], F: 4, AP: { N: appearance },
      Contents: "Synthetic persistent annotation appearance", Name: "Approved",
    }));
    page.node.addAnnot(widget);
    const form = pdf.getForm().createTextField(`Synthetic.Field.${index + 1}`);
    form.setText(`Visible ${index + 1}`);
    form.addToPage(page, { x: 90, y: 60, width: 72, height: 18, borderWidth: 1 });
    form.setFontSize(9);
    const nested = pdf.context.register(pdf.context.flateStream("q 16 0 0 16 0 0 cm /NestedImage Do Q", {
      Type: "XObject", Subtype: "Form", BBox: [0, 0, 16, 16], Resources: { XObject: { NestedImage: image.ref } },
    }));
    page.node.setXObject(PDFName.of("NestedForm"), nested);
    page.pushOperators(pushGraphicsState(), concatTransformationMatrix(1, 0, 0, 1, 176, 80),
      drawObject("NestedForm"), popGraphicsState());
  }
  pdf.getForm().updateFieldAppearances(font);
  // A deliberately misleading declaration/flag is an input observation, not conformance.
  const xml = '<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:a="http://www.aiim.org/pdfa/ns/id/" xmlns:u="http://www.aiim.org/pdfua/ns/id/" a:part="2" a:conformance="B"><u:part>1</u:part></rdf:Description></rdf:RDF></x:xmpmeta>';
  const metadata = pdf.context.register(pdf.context.stream(new TextEncoder().encode(xml), { Type: "Metadata", Subtype: "XML" }));
  pdf.catalog.set(PDFName.of("Metadata"), metadata);
  pdf.catalog.set(PDFName.of("MarkInfo"), pdf.context.obj({ Marked: true }));
  pdf.catalog.set(PDFName.of("StructTreeRoot"), pdf.context.obj({ Type: "StructTreeRoot", K: [] }));
  pdf.catalog.set(PDFName.of("OpenAction"), pdf.context.obj({ S: "JavaScript", JS: "(inert original synthetic code)" }));
  await pdf.attach(new TextEncoder().encode("ORIGINAL ATTACHMENT MUST NOT BE COPIED"), "original-private-marker.txt", { mimeType: "text/plain" });
  pdf.setTitle("ORIGINAL METADATA MUST NOT BE COPIED");
  pdf.setCreationDate(new Date("2020-01-01T00:00:00Z"));
  pdf.setModificationDate(new Date("2020-01-01T00:00:00Z"));
  const source = await pdf.save({ useObjectStreams: false });
  const boundary = await PDFDocument.create();
  boundary.addPage([360.000001, 72]);
  const xfa = await PDFDocument.load(source);
  xfa.getForm().acroForm.dict.set(PDFName.of("XFA"), xfa.context.obj([]));
  return { source: Array.from(source), boundary: Array.from(await boundary.save({ useObjectStreams: false })),
    xfa: Array.from(await xfa.save({ useObjectStreams: false, updateFieldAppearances: false })) };
}

export async function compareArchivePixels(source: number[], copy: number[], dpi: 150 | 300) {
  const sourceLoading = loadPdf(new Uint8Array(source)), copyLoading = loadPdf(new Uint8Array(copy));
  const result = [];
  try {
    const original = await sourceLoading.promise, archive = await copyLoading.promise;
    verify(original.numPages === archive.numPages, "An archival output silently omitted source pages.");
    for (let index = 1; index <= original.numPages; index++) {
      const sourcePage = await original.getPage(index), copyPage = await archive.getPage(index);
      const base = sourcePage.getViewport({ scale: 1 }), archived = copyPage.getViewport({ scale: 1 });
      verify(Math.abs(base.width - archived.width) < 0.001 && Math.abs(base.height - archived.height) < 0.001,
        `Archive page ${index} changed actual physical dimensions.`);
      verify(copyPage.rotate === 0 && copyPage.userUnit === 1, "The archive carried source rotation/UserUnit instead of a fresh upright raster page.");
      const canvases = [];
      try {
        for (const page of [sourcePage, copyPage]) {
          const viewport = page.getViewport({ scale: dpi / 72 });
          const canvas = document.createElement("canvas");
          canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
          canvases.push(canvas);
          const context = canvas.getContext("2d", { alpha: false, colorSpace: "srgb" })!;
          await page.render({ canvas, canvasContext: context, viewport, background: "#ffffff", intent: "display" }).promise;
        }
        const sourceViewport = sourcePage.getViewport({ scale: dpi / 72 }), copyViewport = copyPage.getViewport({ scale: dpi / 72 });
        // An upright archive changes multiplication order for rotated UserUnit
        // pages. Permit only IEEE rounding noise, never a real subpixel size change.
        verify(Math.abs(sourceViewport.width - copyViewport.width) < 1e-9
          && Math.abs(sourceViewport.height - copyViewport.height) < 1e-9, "Save/reopen changed physical raster extents.");
        const sourcePixels = [canvases[0].width, canvases[0].height], archivePixels = [canvases[1].width, canvases[1].height];
        const width = Math.max(...canvases.map((canvas) => canvas.width)), height = Math.max(...canvases.map((canvas) => canvas.height));
        const padded = canvases.map((canvas) => {
          const target = document.createElement("canvas"); target.width = width; target.height = height;
          const context = target.getContext("2d", { alpha: false, colorSpace: "srgb" })!;
          context.fillStyle = "white"; context.fillRect(0, 0, width, height); context.drawImage(canvas, 0, 0);
          const data = context.getImageData(0, 0, width, height).data;
          target.width = target.height = 0;
          return data;
        });
        const [left, right] = padded;
        let changed = 0, changedOver16 = 0, maxDelta = 0, sum = 0;
        for (let offset = 0; offset < left.length; offset += 4) {
          const delta = Math.max(...[0, 1, 2].map((channel) => Math.abs(left[offset + channel] - right[offset + channel])));
          if (delta) changed++;
          if (delta > 16) changedOver16++;
          maxDelta = Math.max(maxDelta, delta); sum += delta;
        }
        verify(changedOver16 / (width * height) < 0.005, `Archive page ${index} differs on more than 0.5% of pixels by over 16 channel levels.`);
        verify(sum / (width * height) < 1, `Archive page ${index} has excessive mean maximum-channel error.`);
        verify((await copyPage.getTextContent()).items.length === 0, "Searchable source text survived archival export.");
        verify((await copyPage.getAnnotations()).length === 0, "Interactive source annotations survived archival export.");
        result.push({ page: index, sourceRotation: sourcePage.rotate, sourceUserUnit: sourcePage.userUnit, sourceCrop: sourcePage.view,
          physicalPoints: [base.width, base.height], sourcePixels, archivePixels, width, height, changed, changedOver16, maxDelta, meanMaxChannelDelta: sum / (width * height) });
      } finally {
        for (const canvas of canvases) canvas.width = canvas.height = 0;
        sourcePage.cleanup(); copyPage.cleanup();
      }
    }
  } finally { await Promise.all([sourceLoading.destroy(), copyLoading.destroy()]); }
  return result;
}

export async function cancellationAndGeometryChecks(source: number[], plan: ArchivePlan) {
  const loading = loadPdf(new Uint8Array(source));
  const results: string[] = [];
  try {
    const pdf = await loading.promise;
    for (const when of ["before-page", "during-render", "after-stage", "source-stale"] as const) {
      const abort = new AbortController();
      let staged = 0, invalidated = false, rejected = false;
      if (when === "before-page") abort.abort();
      try {
        await renderArchivePages(pdf, plan, async () => {
          staged++;
          if (when === "after-stage") abort.abort();
        }, () => {
          if (when === "during-render") window.setTimeout(() => abort.abort(), 0);
          if (when === "source-stale") invalidated = true;
        }, abort.signal, () => { if (invalidated) throw new Error("Synthetic source changed."); });
      } catch { rejected = true; }
      verify(rejected, `Archive ${when} did not reject.`);
      if (when === "before-page" || when === "source-stale") verify(staged === 0, "A canceled/stale source uploaded pixels.");
      results.push(when);
    }

  } finally { await loading.destroy(); }
  return results;
}

export async function renderAndDownloadArchive(plan: ArchivePlan) {
  const abort = new AbortController();
  const loading = loadPdf(await archiveBridge.bytes(plan.archiveId, abort.signal));
  try {
    await renderArchivePages(await loading.promise, plan, archiveBridge.stage, () => {}, abort.signal, () => {});
    await archiveBridge.check(plan.archiveId);
    return await archiveBridge.save(plan, abort.signal, () => {});
  } finally {
    await loading.destroy();
    await archiveBridge.cancel(plan.archiveId);
  }
}
