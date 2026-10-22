import type { PDFDocumentProxy } from "pdfjs-dist";

export interface Bookmark {
  title: string;
  destination: unknown;
  url: string | null;
  depth: number;
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function flattenBookmarks(value: unknown, depth = 0, result: Bookmark[] = []): Bookmark[] {
  if (value === null) return result;
  if (!Array.isArray(value)) throw new Error("The PDF bookmark list is malformed.");
  if (depth > 32 || result.length > 2000) throw new Error("The PDF bookmark list is too large or deep.");
  for (const item of value) {
    if (!isRecord(item) || typeof item.title !== "string") throw new Error("A PDF bookmark is malformed.");
    result.push({ title: item.title || "Untitled bookmark", destination: item.dest, url: typeof item.url === "string" ? item.url : null, depth });
    if (result.length > 2000) throw new Error("This PDF has more than 2,000 bookmarks.");
    if (item.items !== undefined) flattenBookmarks(item.items, depth + 1, result);
  }
  return result;
}

export async function destinationPage(pdf: Pick<PDFDocumentProxy, "getDestination" | "getPageIndex" | "numPages">, destination: unknown): Promise<number> {
  const resolved: unknown = typeof destination === "string" ? await pdf.getDestination(destination) : destination;
  if (!Array.isArray(resolved) || !resolved.length) throw new Error("This document link has no valid page destination.");
  const reference: unknown = resolved[0];
  let page: number;
  if (typeof reference === "number" && Number.isSafeInteger(reference)) {
    page = reference;
  } else if (isRecord(reference) && typeof reference.num === "number" && typeof reference.gen === "number"
    && Number.isSafeInteger(reference.num) && Number.isSafeInteger(reference.gen) && reference.num > 0 && reference.gen >= 0) {
    page = await pdf.getPageIndex({ num: reference.num, gen: reference.gen });
  } else throw new Error("This document link uses an unsupported page destination.");
  if (!Number.isSafeInteger(page) || page < 0 || page >= pdf.numPages) throw new Error("This document link points outside the PDF.");
  return page;
}

export function safeExternalUrl(value: string): string {
  if (/[\u0000-\u001f\u007f]/.test(value)) throw new Error("The document link contains invalid control characters.");
  let url: URL;
  try { url = new URL(value); } catch { throw new Error("This document link has an invalid address."); }
  if (!["http:", "https:", "mailto:"].includes(url.protocol) || url.username || url.password) {
    throw new Error("Only web and email links without embedded credentials can be opened.");
  }
  return url.href;
}
