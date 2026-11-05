import { pageFromInput } from "./geometry.ts";
import type { DocumentInfo } from "./types.ts";

export const PAGE_LABEL_STYLES = [
  { id: "decimal", name: "Numbers: 1, 2, 3" },
  { id: "roman_lower", name: "Roman: i, ii, iii" },
  { id: "roman_upper", name: "Roman: I, II, III" },
  { id: "letters_lower", name: "Letters: a, b, c" },
  { id: "letters_upper", name: "Letters: A, B, C" },
  { id: "prefix", name: "Prefix only" },
] as const;

export type PageLabelStyle = typeof PAGE_LABEL_STYLES[number]["id"];
export interface PageLabelRange {
  page: number;
  style: PageLabelStyle;
  prefix: string;
  start: number;
}
export interface PageLabelsInfo {
  labels: string[] | null;
  ranges: PageLabelRange[];
  issue: string | null;
}
export interface PageLabelDraftRange {
  page: string;
  style: PageLabelStyle;
  prefix: string;
  start: string;
}
export interface PageLabelDraft {
  documentId: number;
  revision: number;
  ranges: PageLabelDraftRange[];
  initial: PageLabelDraftRange[];
  custom: boolean;
}

export function createPageLabelDraft(info: DocumentInfo): PageLabelDraft {
  if (info.pageCount > 10_000) throw new Error("Page-label editing supports PDFs of at most 10,000 pages.");
  const ranges = info.pageLabels?.ranges.length ? info.pageLabels.ranges
    : [{ page: 0, style: "decimal" as const, prefix: "", start: 1 }];
  if (ranges.length > 256) throw new Error("This PDF has more than 256 label sections. Navigation is available, but this label tree is too large to edit.");
  const initial = ranges.map((range) => ({ page: String(range.page + 1), start: String(range.start),
    style: range.style, prefix: range.prefix }));
  return { documentId: info.id, revision: info.revision, ranges: initial.map((range) => ({ ...range })),
    initial, custom: info.pageLabels !== null };
}

export function pageLabelsPending(draft: PageLabelDraft): boolean {
  return JSON.stringify(draft.ranges) !== JSON.stringify(draft.initial);
}

export function pageLabelChanges(draft: PageLabelDraft, info: DocumentInfo): PageLabelRange[] {
  if (draft.documentId !== info.id || draft.revision !== info.revision) {
    throw new Error("The PDF changed while these page labels were being edited. Your draft is retained; review the current PDF.");
  }
  if (!draft.ranges.length || draft.ranges.length > 256) throw new Error("Keep between 1 and 256 numbering sections.");
  const ranges = draft.ranges.map((range) => {
    const page = pageFromInput(range.page, info.pageCount);
    const start = range.style === "prefix" ? 1 : Number(range.start);
    if (page === null || range.style !== "prefix" && !/^\d+$/.test(range.start)
      || !Number.isSafeInteger(start) || start < 1 || start > 0xffffffff) {
      throw new Error("Each section needs a physical page within this PDF and a positive whole starting number.");
    }
    if (!PAGE_LABEL_STYLES.some((style) => style.id === range.style)
      || new TextEncoder().encode(range.prefix).byteLength > 512 || /[\u0000-\u001f\u007f-\u009f]/u.test(range.prefix)) {
      throw new Error("Choose a supported numbering style and a single-line prefix of no more than 512 UTF-8 bytes.");
    }
    return { page, start: range.style === "prefix" ? 1 : start, style: range.style, prefix: range.prefix };
  });
  if (ranges[0].page !== 0 || ranges.some((range, index) => index > 0 && range.page <= ranges[index - 1].page)) {
    throw new Error("The first section must start at physical page 1; later sections must start on distinct ascending pages.");
  }
  for (const [index, range] of ranges.entries()) {
    const end = ranges[index + 1]?.page ?? info.pageCount;
    if (range.style !== "prefix" && range.start + end - range.page - 1 > 0xffffffff) {
      throw new Error("A page-label numbering sequence exceeds its supported integer range.");
    }
  }
  return ranges;
}

export function pageLabelText(info: Pick<DocumentInfo, "pageLabels"> | null, page: number): string {
  return info?.pageLabels?.labels?.[page] || String(page + 1);
}

export function pageDescription(info: Pick<DocumentInfo, "pageLabels"> | null, page: number): string {
  const label = info?.pageLabels?.labels?.[page];
  return label && label !== String(page + 1) ? `page ${label} (physical page ${page + 1})` : `page ${page + 1}`;
}

export function resolvePageInput(value: string, count: number, labels: readonly string[] | null): number {
  if (labels) {
    if (labels.length !== count) throw new Error("The page labels do not match this PDF. Use physical page navigation.");
    const matches = labels.flatMap((label, index) => label === value && value ? [index] : []);
    if (matches.length === 1) return matches[0];
    if (matches.length > 1) throw new Error("This label belongs to more than one page. Choose Page mode and enter its physical position.");
    throw new Error("No page has that exact label. Labels are case-sensitive; choose Page mode to use physical positions.");
  }
  const page = pageFromInput(value, count);
  if (page === null) throw new Error(`Enter a physical page from 1 to ${count}.`);
  return page;
}
