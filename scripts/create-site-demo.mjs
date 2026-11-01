import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { PDFDocument, StandardFonts, rgb } from "pdf-lib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pdf = await PDFDocument.create();
const regular = await pdf.embedFont(StandardFonts.Helvetica);
const bold = await pdf.embedFont(StandardFonts.HelveticaBold);
const blue = rgb(0.133, 0.353, 0.902);
const ink = rgb(0.16, 0.196, 0.255);
const muted = rgb(0.36, 0.41, 0.49);
const pale = rgb(0.925, 0.949, 1);
const fixedDate = new Date("2026-11-01T00:00:00Z");

pdf.setTitle("A little less paperwork - Folio sample");
pdf.setAuthor("Folio contributors");
pdf.setCreator("Folio original sample document");
pdf.setSubject("Original fictional content for trying Folio's PDF tools.");
pdf.setCreationDate(fixedDate);
pdf.setModificationDate(fixedDate);

function page(title, index) {
  const result = pdf.addPage([612, 792]);
  result.drawText("folio", { x: 48, y: 734, font: bold, size: 19, color: blue });
  result.drawText("FIELD NOTES", { x: 431, y: 738, font: regular, size: 10, color: muted });
  result.drawLine({ start: { x: 48, y: 710 }, end: { x: 564, y: 710 }, thickness: 1, color: pale });
  result.drawText(title, { x: 48, y: 647, font: bold, size: 32, color: ink });
  result.drawText("Sample document. Fictional content, created for Folio.", {
    x: 48, y: 38, font: regular, size: 9, color: muted,
  });
  result.drawText(String(index), { x: 552, y: 38, font: regular, size: 10, color: blue });
  return result;
}

function paragraph(target, lines, y, size = 15) {
  for (const line of lines) {
    if (regular.widthOfTextAtSize(line, size) > 516) throw new Error("A sample line exceeds the page.");
    target.drawText(line, { x: 48, y, font: regular, size, color: ink });
    y -= size * 1.65;
  }
  return y;
}

const first = page("A little less paperwork.", 1);
first.drawText("Make space for the work that matters.", {
  x: 48, y: 605, font: regular, size: 18, color: blue,
});
paragraph(first, [
  "A PDF can be a proposal, a reading list, a rough idea or a",
  "finished piece of work. The tools around it should help you",
  "move forward, not ask you to start another subscription.",
], 546);
first.drawRectangle({ x: 48, y: 346, width: 516, height: 94, color: pale });
first.drawText("Your files stay on your computer.", {
  x: 68, y: 398, font: bold, size: 20, color: blue,
});
first.drawText("Read. Leave a note. Make a change. Save a copy.", {
  x: 68, y: 371, font: regular, size: 14, color: ink,
});
paragraph(first, [
  "Try this page: highlight a sentence, add a note, or replace",
  "a supported source text run. Then save and reopen a copy.",
  "You should be able to see the change in a real PDF.",
], 289);

const second = page("Open tools. Clear choices.", 2);
paragraph(second, [
  "Folio is an independent, open-source PDF workspace.",
  "Its source is yours to inspect, learn from and improve.",
  "No Acrobat installation is needed.",
], 579);
for (const [index, title] of ["Read and review", "Edit and organize", "Save and share"].entries()) {
  const y = 444 - index * 112;
  second.drawText(title, { x: 48, y, font: bold, size: 20, color: blue });
  second.drawLine({ start: { x: 48, y: y - 20 }, end: { x: 564, y: y - 20 }, thickness: 1, color: pale });
}

const third = page("Before you share.", 3);
paragraph(third, [
  "Keep an original. Share a deliberately saved copy.",
  "Review edits, permissions and any unsaved changes.",
  "An annotation over a word does not remove that word.",
  "Cropping a page does not erase hidden content.",
], 573);
third.drawRectangle({ x: 48, y: 341, width: 516, height: 104, color: pale });
paragraph(third, [
  "Folio's applied redaction exports a separate image-only PDF.",
  "It removes original objects, but also loses searchable text",
  "and other document structure. Read the limits first.",
], 411, 14);

const fourth = page("Notes, not noise.", 4);
paragraph(fourth, [
  "A small checklist for an ordinary day:",
], 582);
for (const [index, title] of [
  "Read the draft.",
  "Highlight the part worth remembering.",
  "Leave a useful note.",
  "Put the pages in the right order.",
  "Save a copy with a clear name.",
].entries()) {
  const y = 516 - index * 62;
  fourth.drawRectangle({ x: 48, y: y - 3, width: 15, height: 15, borderColor: blue, borderWidth: 1.2 });
  fourth.drawText(title, { x: 79, y, font: regular, size: 16, color: ink });
}

await mkdir(resolve(root, "site/assets"), { recursive: true });
await writeFile(resolve(root, "site/assets/try-folio.pdf"), await pdf.save({ useObjectStreams: false }));
console.log("Created the original, fictional four-page Folio sample PDF.");
