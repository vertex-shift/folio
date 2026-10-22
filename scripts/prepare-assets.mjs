import { cpSync, existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { PDFDocument, PDFName, PDFString, StandardFonts, rgb } from "pdf-lib";
import { collectNotices } from "./collect-notices.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
function unchanged(source, destination) {
  return existsSync(destination) && statSync(destination).isFile()
    && statSync(source).size === statSync(destination).size
    && readFileSync(source).equals(readFileSync(destination));
}
mkdirSync(`${root}public/vendor/pdfjs`, { recursive: true });
for (const folder of ["cmaps", "standard_fonts", "wasm", "iccs"]) {
  cpSync(
    `${root}node_modules/pdfjs-dist/${folder}`,
    `${root}public/vendor/pdfjs/${folder}`,
    { recursive: true, filter: (source, destination) => statSync(source).isDirectory() || !unchanged(source, destination) },
  );
}

const pdf = await PDFDocument.create();
pdf.setTitle("Folio quick start");
pdf.setAuthor("Folio");
pdf.setSubject("An original example document for the Folio PDF workspace");
pdf.setProducer("Folio sample generator / pdf-lib");
pdf.setCreationDate(new Date("2026-01-01T00:00:00Z"));
pdf.setModificationDate(new Date("2026-01-01T00:00:00Z"));

const regular = await pdf.embedFont(StandardFonts.Helvetica);
const bold = await pdf.embedFont(StandardFonts.HelveticaBold);
const ink = rgb(0.12, 0.17, 0.24);
const muted = rgb(0.37, 0.42, 0.49);
const blue = rgb(0.13, 0.34, 0.88);
const line = rgb(0.86, 0.89, 0.92);
const tint = rgb(0.94, 0.96, 1);

function page(number) {
  const result = pdf.addPage([612, 792]);
  result.drawText("folio", { x: 54, y: 738, size: 22, font: bold, color: ink });
  result.drawText("A LITTLE LESS PAPERWORK", {
    x: 365, y: 743, size: 8, font: regular, color: muted,
  });
  result.drawLine({ start: { x: 54, y: 719 }, end: { x: 558, y: 719 }, color: line });
  result.drawLine({ start: { x: 54, y: 60 }, end: { x: 558, y: 60 }, color: line });
  result.drawText("Folio quick start  /  Example document", {
    x: 54, y: 40, size: 9, font: regular, color: muted,
  });
  result.drawText(`${number} / 3`, {
    x: 534, y: 40, size: 9, font: regular, color: muted,
  });
  return result;
}

function text(target, value, x, y, size = 11, color = muted, font = regular) {
  target.drawText(value, { x, y, size, color, font, lineHeight: size * 1.6 });
}

function section(target, title, body, y) {
  text(target, title, 54, y, 14, ink, bold);
  text(target, body, 54, y - 25, 11);
}

const first = page(1);
text(first, "Meet your new\npaperwork space.", 54, 647, 39, ink, bold);
text(first,
  "Read closely. Leave a thought. Put your pages in order.\n" +
  "A familiar PDF workspace, with a little more breathing room.",
  54, 534, 12);
first.drawRectangle({ x: 54, y: 405, width: 504, height: 76, color: tint });
text(first, "Start with something worth highlighting.", 72, 454, 12, blue, bold);
text(first,
  "Select a passage, then choose Highlight. Your mark becomes\n" +
  "a real PDF annotation that travels with the document.",
  72, 432, 10, ink);
section(first, "Make yourself at home",
  "Open a PDF from your device. Use the page rail to move around,\n" +
  "or search for a word to jump straight to what matters.", 360);
section(first, "A note, right where you need it",
  "Choose Add a note and click on the page. Your comments stay\n" +
  "connected to the document, not tucked away in another app.", 255);
text(first,
  "This is an editable example. Try a highlight, add a note, or turn the page.",
  54, 112, 10);

const second = page(2);
text(second, "A closer look.", 54, 650, 36, ink, bold);
text(second,
  "A good review is a conversation with the document.\n" +
  "Keep the important parts easy to find.",
  54, 591, 12);
section(second, "Find the right passage",
  "Press Command+F to search this document. Search reads the PDF\n" +
  "text layer; scanned pages without text need OCR, which is not in\n" +
  "this first release.", 510);
section(second, "Highlight with intention",
  "Choose the highlighter and drag across text. You can also select\n" +
  "text with the cursor, then use the small Highlight action.\n" +
  "Your highlights are saved as standard PDF annotations.", 383);
section(second, "Leave a useful note",
  "Choose Add a note, click a location, and write your comment.\n" +
  "Open Comments to revisit your notes or remove a mark.", 255);
second.drawRectangle({ x: 54, y: 107, width: 504, height: 58, color: tint });
text(second, "Your original stays yours.", 71, 143, 11, blue, bold);
text(second, "Save a copy to keep the source file unchanged.", 71, 124, 10, ink);

const third = page(3);
text(third, "Everything in order.", 54, 650, 36, ink, bold);
text(third,
  "Documents do not always arrive in the right order.\n" +
  "Make the small changes that make the whole thing work.",
  54, 591, 12);
section(third, "Move a page",
  "Open Organize pages. Drag a thumbnail to a new position, or use\n" +
  "the Move earlier and Move later buttons for keyboard access.", 510);
section(third, "Turn things around",
  "Rotate the current page in either direction. Rotation is saved\n" +
  "inside the PDF, so the next reader sees the same orientation.", 400);
section(third, "Keep only what you need",
  "Remove a page after confirming the change. Undo is available\n" +
  "within working history. Folio will not remove the final page.", 290);
third.drawRectangle({ x: 54, y: 116, width: 504, height: 69, color: tint });
text(third, "Ready for your own document?", 72, 158, 13, blue, bold);
text(third, "Use Open PDF, or drop a PDF into the window.", 72, 136, 11, ink);

const outlineRoot = pdf.context.nextRef();
const pages = pdf.getPages();
const outlineItems = ["Getting started", "A closer look", "Everything in order"].map((title, index) =>
  pdf.context.obj({ Title: PDFString.of(title), Parent: outlineRoot,
    Dest: [pages[index].ref, PDFName.of("Fit")] }));
const outlineRefs = outlineItems.map((item) => pdf.context.register(item));
outlineItems.forEach((item, index) => {
  if (index > 0) item.set(PDFName.of("Prev"), outlineRefs[index - 1]);
  if (index + 1 < outlineRefs.length) item.set(PDFName.of("Next"), outlineRefs[index + 1]);
});
pdf.context.assign(outlineRoot, pdf.context.obj({
  Type: PDFName.of("Outlines"), Count: outlineRefs.length,
  First: outlineRefs[0], Last: outlineRefs.at(-1),
}));
pdf.catalog.set(PDFName.of("Outlines"), outlineRoot);
first.node.addAnnot(pdf.context.register(pdf.context.obj({
  Type: PDFName.of("Annot"), Subtype: PDFName.of("Link"),
  Rect: [54, 104, 558, 126], Border: [0, 0, 0],
  Dest: [second.ref, PDFName.of("Fit")],
})));
text(third, "Folio on GitHub", 406, 40, 9, blue);
third.node.addAnnot(pdf.context.register(pdf.context.obj({
  Type: PDFName.of("Annot"), Subtype: PDFName.of("Link"),
  Rect: [403, 35, 482, 49], Border: [0, 0, 0],
  A: { S: PDFName.of("URI"), URI: PDFString.of("https://github.com/vertex-shift/folio") },
})));

mkdirSync(`${root}src-tauri/resources`, { recursive: true });
const sample = await pdf.save({ useObjectStreams: false });
const samplePath = `${root}src-tauri/resources/folio-guide.pdf`;
if (!existsSync(samplePath) || !readFileSync(samplePath).equals(Buffer.from(sample))) {
  writeFileSync(samplePath, sample);
}
collectNotices(root);
console.log("Prepared PDF.js assets and the original three-page Folio example.");
