import { PDFDocument, PDFName, PDFNumber, StandardFonts, degrees, rgb } from "pdf-lib";
import { loadPdf } from "../src/lib/pdf";
import { runComparison } from "../src/features/comparison/engine";
import { alignedPages, geometryChanges, pageChanged, pagePresence } from "../src/features/comparison/model";
import type { ComparisonPlan } from "../src/features/comparison/model";
import type { PageGeometry } from "../src/lib/authoring";

function verify(condition: boolean, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

export async function comparisonFixtures() {
  async function create(after: boolean) {
    const pdf = await PDFDocument.create();
    const font = await pdf.embedFont(StandardFonts.Helvetica);
    const geometry: PageGeometry[] = [];
    for (let index = 0; index < (after ? 5 : 6); index++) {
      const page = pdf.addPage([360, 240]);
      page.drawText(index === 1 ? `Total: ${after ? "15" : "12"}` : `Original synthetic comparison page ${index + 1}`, { font, size: 12, x: 24, y: 160 });
      page.drawRectangle({ x: 24, y: 36, width: 40, height: 20,
        color: index === 2 && after ? rgb(0.1, 0.2, 0.8) : rgb(0.8, 0.15, 0.1) });
      if (index === 3) {
        page.setMediaBox(-20, -10, 420, 260);
        page.setCropBox(10, 20, after ? 300 : 340, after ? 180 : 200);
        page.setRotation(degrees(after ? 270 : 90));
        page.node.set(PDFName.of("UserUnit"), PDFNumber.of(after ? 1.5 : 2));
      }
      if (index === 4) {
        const field = pdf.getForm().createTextField("Synthetic.appearance");
        field.setText(after ? "Approved" : "Pending");
        field.addToPage(page, { x: 100, y: 35, width: 65, height: 16, borderWidth: 0 });
        field.setFontSize(9);
        pdf.getForm().updateFieldAppearances(font);
      }
      const media = page.getMediaBox(), crop = page.getCropBox();
      const bounds: [number, number, number, number] = [
        Math.max(media.x, crop.x), Math.max(media.y, crop.y),
        Math.min(media.x + media.width, crop.x + crop.width), Math.min(media.y + media.height, crop.y + crop.height),
      ];
      const unit = index === 3 ? after ? 1.5 : 2 : 1;
      const rotation = page.getRotation().angle;
      let width = (bounds[2] - bounds[0]) * unit, height = (bounds[3] - bounds[1]) * unit;
      if (rotation % 180) [width, height] = [height, width];
      geometry.push({ mediaBox: [media.x, media.y, media.x + media.width, media.y + media.height], bounds,
        originalBounds: bounds, rotation, userUnit: unit, widthPoints: width, heightPoints: height, cropModified: false });
    }
    pdf.setTitle(after ? "Synthetic After" : "Synthetic Before");
    return { bytes: Array.from(await pdf.save()), geometry };
  }
  const before = await create(false), after = await create(true);
  const fileOnly = await PDFDocument.load(new Uint8Array(before.bytes));
  fileOnly.setTitle("Different metadata, same visible pages");
  await fileOnly.attach(new TextEncoder().encode("Original synthetic attachment, never part of the compared page scope."), "not-compared.txt",
    { mimeType: "text/plain", description: "Fixture-only unexamined content" });
  return { before, after, fileOnly: { bytes: Array.from(await fileOnly.save()), geometry: before.geometry } };
}

export async function checkComparisonRenderer() {
  const fixtures = await comparisonFixtures();
  const beforeBytes = new Uint8Array(fixtures.before.bytes), afterBytes = new Uint8Array(fixtures.after.bytes);
  const hash = async (bytes: Uint8Array) => Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes)))).join(",");
  const beforeHash = await hash(beforeBytes), afterHash = await hash(afterBytes);
  const source = (documentId: number, value: typeof fixtures.before) => ({
    documentId, revision: 0, name: `${documentId}.pdf`, pageCount: value.geometry.length,
    sourceBytes: value.bytes.length, passwordProtected: false, pages: value.geometry,
  });
  const plan: ComparisonPlan = { comparisonId: 1, before: source(1, fixtures.before), after: source(2, fixtures.after),
    settings: { dpi: 72, tolerance: 0 }, pages: alignedPages(fixtures.before.geometry, fixtures.after.geometry, { dpi: 72, tolerance: 0 })
      .map((page) => ({ height: page.height, index: page.index, width: page.width })),
    scope: "Page-index text, pixels and geometry only; not document identity.", suggestedName: "comparison.json" };
  const operations = {
    bytes: async (side: "before" | "after") => new Uint8Array(side === "before" ? beforeBytes : afterBytes),
    check: () => {}, verify: async () => {},
  };
  const result = await runComparison(plan, operations, new AbortController().signal, () => {});
  verify(result.pages.length === 6, "An unmatched trailing page was omitted.");
  verify(!pageChanged(plan, result.pages[0]), "Equal page text/pixels/geometry were reported as changed.");
  verify(result.pages[1].text.spans.some((span) => span.kind === "removed" && span.text === "12")
    && result.pages[1].text.spans.some((span) => span.kind === "added" && span.text === "15"), "Embedded text changes were not real token differences.");
  verify(result.pages[2].pixels.changed > 0 && result.pages[2].text.spans.every((span) => span.kind === "equal"), "Color-only changes were missed.");
  verify(geometryChanges(plan.before.pages[3], plan.after.pages[3]).join(",") === "effective_crop,rotation,user_unit,physical_size", "Crop/rotation/UserUnit changes were missed.");
  verify(result.pages[4].pixels.changed > 0 && result.pages[4].text.spans.every((span) => span.kind === "equal"), "AcroForm appearance-only changes were missed.");
  verify(pagePresence(plan, 5) === "removed", "A removed page was not labeled at its unmatched index.");

  const loadingBefore = loadPdf(new Uint8Array(beforeBytes)), loadingAfter = loadPdf(new Uint8Array(afterBytes));
  const independent: { page: number; changed: number; total: number }[] = [];
  try {
    const before = await loadingBefore.promise, after = await loadingAfter.promise;
    for (const target of plan.pages) {
      const canvases: HTMLCanvasElement[] = [];
      try {
        for (const pdf of [before, after]) {
          const canvas = document.createElement("canvas");
          canvas.width = target.width; canvas.height = target.height;
          canvases.push(canvas);
          const context = canvas.getContext("2d", { alpha: false })!;
          context.fillStyle = "white"; context.fillRect(0, 0, target.width, target.height);
          if (target.index < pdf.numPages) {
            const page = await pdf.getPage(target.index + 1);
            await page.render({ canvas, viewport: page.getViewport({ scale: 1 }), background: "#ffffff" }).promise;
            page.cleanup();
          }
        }
        const left = canvases[0].getContext("2d")!.getImageData(0, 0, target.width, target.height).data;
        const right = canvases[1].getContext("2d")!.getImageData(0, 0, target.width, target.height).data;
        let changed = 0;
        for (let offset = 0; offset < left.length; offset += 4) {
          if (left[offset] !== right[offset] || left[offset + 1] !== right[offset + 1] || left[offset + 2] !== right[offset + 2]) changed++;
        }
        verify(changed === result.pages[target.index].pixels.changed, `Independent PDF.js pixel counts disagree on page ${target.index + 1}.`);
        independent.push({ page: target.index + 1, changed, total: target.width * target.height });
      } finally { for (const canvas of canvases) canvas.width = canvas.height = 0; }
    }
  } finally { await Promise.all([loadingBefore.destroy(), loadingAfter.destroy()]); }

  const reversed: ComparisonPlan = { ...plan, before: plan.after, after: plan.before,
    pages: alignedPages(plan.after.pages, plan.before.pages, plan.settings) };
  const added = await runComparison(reversed, {
    ...operations, bytes: async (side) => new Uint8Array(side === "before" ? afterBytes : beforeBytes),
  }, new AbortController().signal, () => {});
  verify(pagePresence(reversed, added.pages[5].index) === "added", "Added pages were not reported.");

  const fileOnlyPlan = { ...plan, after: source(3, fixtures.fileOnly), pages: alignedPages(plan.before.pages, fixtures.fileOnly.geometry, plan.settings) };
  const fileOnly = await runComparison(fileOnlyPlan, {
    ...operations, bytes: async (side) => new Uint8Array(side === "before" ? beforeBytes : fixtures.fileOnly.bytes),
  }, new AbortController().signal, () => {});
  verify(fileOnly.pages.every((page) => !pageChanged(fileOnlyPlan, page)), "Out-of-scope metadata/attachments changed compared page results.");
  verify(await hash(beforeBytes) === beforeHash && await hash(afterBytes) === afterHash, "Comparison mutated source buffers.");

  const cancellations: string[] = [];
  for (const stage of ["snapshots", "text", "pixels"] as const) {
    const abort = new AbortController();
    let canceled = false;
    try {
      await runComparison(plan, operations, abort.signal, (progress) => { if (progress.stage === stage) abort.abort(); });
    } catch (error) { canceled = error instanceof DOMException && error.name === "AbortError"; }
    verify(canceled, `Cancellation failed during ${stage}.`);
    cancellations.push(stage);
  }
  {
    const abort = new AbortController();
    let timer: number | undefined, canceled = false;
    try {
      await runComparison(plan, operations, abort.signal, (progress) => {
        if (progress.stage === "text" && timer === undefined) timer = window.setTimeout(() => abort.abort(), 0);
      });
    } catch (error) { canceled = error instanceof DOMException && error.name === "AbortError"; }
    finally { window.clearTimeout(timer); }
    verify(canceled, "Cancellation failed with an extraction stream already in flight.");
    cancellations.push("in-flight extraction");
  }
  let changedDuringSnapshot = false, staleRejected = false;
  try {
    await runComparison(plan, {
      ...operations,
      bytes: async (side) => { const bytes = await operations.bytes(side); changedDuringSnapshot = true; return bytes; },
      check: () => { if (changedDuringSnapshot) throw new Error("After PDF changed or closed."); },
    }, new AbortController().signal, () => {});
  } catch (error) { staleRejected = error instanceof Error && error.message.includes("changed or closed"); }
  verify(staleRejected, "A source changed during asynchronous snapshot work without invalidating the comparison.");
  const mismatch = { ...plan, before: { ...plan.before, pages: plan.before.pages.map((geometry, index) => index === 0 ? { ...geometry, rotation: 180 } : geometry) } };
  let mismatchedGeometryRejected = false;
  try { await runComparison(mismatch, operations, new AbortController().signal, () => {}); }
  catch (error) { mismatchedGeometryRejected = error instanceof Error && error.message.includes("unmatched renderer/core geometry"); }
  verify(mismatchedGeometryRejected, "Mismatched source layouts were silently declared equal.");
  return { independent, added: added.pages.length, appearancePixels: result.pages[4].pixels.changed,
    geometry: geometryChanges(plan.before.pages[3], plan.after.pages[3]),
    fileOnlyNoObservedDifferences: true, sourceBuffersUnchanged: true, cancellations, staleRejected, mismatchedGeometryRejected };
}
