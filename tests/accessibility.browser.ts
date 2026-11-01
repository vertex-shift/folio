import { loadPdf } from "../src/lib/pdf";
import type { PDFDocumentProxy } from "pdfjs-dist";

interface CheckedNode {
  role: string;
  alt: string | null;
  language: string | null;
  text: string;
  contentIds: string[];
}

export async function semanticEvidence(bytes: number[]) {
  const task = loadPdf(new Uint8Array(bytes));
  const pages = [];
  try {
    const pdf = await task.promise;
    const metadata = await pdf.getMetadata();
    for (let pageNumber = 1; pageNumber <= pdf.numPages; pageNumber++) {
      const page = await pdf.getPage(pageNumber);
      const tree = await page.getStructTree();
      const text = await page.getTextContent({ includeMarkedContent: true });
      const stack: (string | null)[] = [], textById = new Map<string, string>();
      for (const item of text.items) {
        if ("str" in item) {
          for (const id of stack) if (id) textById.set(id, (textById.get(id) ?? "") + item.str + (item.hasEOL ? "\n" : " "));
        } else if (item.type === "beginMarkedContent" || item.type === "beginMarkedContentProps") stack.push(item.id ?? null);
        else if (item.type === "endMarkedContent") stack.pop();
      }
      if (stack.length) throw new Error("Unbalanced actual PDF.js marked-content items.");
      const nodes: CheckedNode[] = [];
      function visit(node: NonNullable<typeof tree>) {
        const contentIds: string[] = [];
        function ids(value: NonNullable<typeof tree>) {
          for (const child of value.children) {
            if ("role" in child) ids(child);
            else if (child.type === "content") contentIds.push(child.id);
          }
        }
        ids(node);
        if (!["Root", "Document"].includes(node.role)) nodes.push({
          role: node.role, alt: "alt" in node && typeof node.alt === "string" ? node.alt : null,
          language: "lang" in node && typeof node.lang === "string" ? node.lang : null,
          contentIds, text: contentIds.map((id) => textById.get(id) ?? "").join(" ").replace(/\s+/gu, " ").trim(),
        });
        for (const child of node.children) if ("role" in child) visit(child);
      }
      if (tree) visit(tree);
      pages.push({ page: pageNumber, language: text.lang, tree, nodes,
        searchableText: text.items.filter((item) => "str" in item).map((item) => item.str).join(" ").replace(/\s+/gu, " ").trim() });
      page.cleanup();
    }
    return { title: "Title" in metadata.info && typeof metadata.info.Title === "string" ? metadata.info.Title : null,
      pdfUaClaim: metadata.metadata?.get("pdfuaid:part") ?? null, pages };
  } finally { await task.destroy(); }
}

async function pixels(pdf: PDFDocumentProxy, index: number) {
  const page = await pdf.getPage(index), viewport = page.getViewport({ scale: 1.5 });
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) throw new Error("The independent PDF.js canvas is unavailable.");
  try {
    await page.render({ canvas, viewport }).promise;
    return { width: canvas.width, height: canvas.height, data: context.getImageData(0, 0, canvas.width, canvas.height).data };
  } finally { canvas.width = 0; canvas.height = 0; page.cleanup(); }
}

export async function compareSemanticPixels(source: number[], output: number[]) {
  const first = loadPdf(new Uint8Array(source)), second = loadPdf(new Uint8Array(output));
  try {
    const [a, b] = await Promise.all([first.promise, second.promise]);
    if (a.numPages !== b.numPages) throw new Error("Tagging changed the actual page count.");
    const pages = [];
    for (let index = 1; index <= a.numPages; index++) {
      const [left, right] = await Promise.all([pixels(a, index), pixels(b, index)]);
      if (left.width !== right.width || left.height !== right.height) throw new Error("Tagging changed actual page dimensions.");
      let changedPixels = 0;
      for (let offset = 0; offset < left.data.length; offset += 4) {
        if (left.data[offset] !== right.data[offset] || left.data[offset + 1] !== right.data[offset + 1]
          || left.data[offset + 2] !== right.data[offset + 2] || left.data[offset + 3] !== right.data[offset + 3]) changedPixels++;
      }
      pages.push({ page: index, width: left.width, height: left.height, changedPixels });
    }
    return pages;
  } finally { await Promise.all([first.destroy(), second.destroy()]); }
}
