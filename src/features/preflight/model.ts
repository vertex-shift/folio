import { parsePageRange } from "../../lib/pages.ts";
import type { PageGeometry } from "../../lib/authoring.ts";

export const ARCHIVE_LIMITS = {
  pages: 100, pagePixels: 24_000_000, totalPixels: 100_000_000, dimension: 16_384,
  pngBytes: 64 * 1024 * 1024, outputBytes: 128 * 1024 * 1024, reportBytes: 2 * 1024 * 1024,
} as const;

export interface SourceRef { documentId: number; revision: number }
export type ObservationState = "fact" | "absent" | "unsupported" | "unchecked";
export interface Finding { category: string; property: string; state: ObservationState; detail: string; page?: number }
export interface TechnicalReport extends SourceRef {
  schema: "folio-technical-preflight";
  schemaVersion: 1;
  inspectionId: number;
  epoch: number;
  name: string;
  passwordProtected: boolean;
  scope: string;
  inputConformance: string;
  findings: Finding[];
  suggestedName: string;
}
export interface ArchiveRequest extends SourceRef {
  pages: number[];
  dpi: 150 | 300;
  acknowledgeLoss: boolean;
  allowUnencrypted: boolean;
  acknowledgeUnsigned: boolean;
}
export interface ArchivePage {
  sourcePage: number;
  geometry: PageGeometry;
  pixelWidth: number;
  pixelHeight: number;
}
export interface ArchivePlan extends SourceRef {
  archiveId: number;
  epoch: number;
  dpi: 150 | 300;
  sourcePageCount: number;
  pages: ArchivePage[];
  suggestedName: string;
  passwordProtected: boolean;
  signedSource: boolean;
  profileSha256: string;
  disclosure: string;
}

export function archivePages(all: boolean, range: string, count: number): number[] {
  if (!Number.isSafeInteger(count) || count < 1) throw new Error("This PDF has no readable source pages.");
  if (range.length > 4096) throw new Error("The archival page range exceeds 4,096 characters.");
  if (all && count > ARCHIVE_LIMITS.pages) throw new Error("Archival copies support at most 100 selected pages. Choose a smaller explicit range; pages are never skipped.");
  const pages = all ? Array.from({ length: count }, (_, index) => index) : parsePageRange(range, count, ARCHIVE_LIMITS.pages);
  if (pages.some((page, index) => index > 0 && pages[index - 1] >= page)) {
    throw new Error("Choose unique archival pages in ascending document order.");
  }
  return pages;
}

export function assertTechnicalCurrent(source: SourceRef, current: { id: number; revision: number } | null, pending: boolean) {
  if (!current || current.id !== source.documentId || current.revision !== source.revision) {
    throw new Error("The technical source changed or closed. Prepare the current PDF again.");
  }
  if (pending) throw new Error("Apply or discard every writing, metadata, form, image, authoring, redaction and live-gesture draft before technical output.");
}

export function validateArchiveGeometry(page: ArchivePage, actual: { rotate: number; userUnit: number; view: readonly number[]; width: number; height: number }, dpi: number) {
  const { geometry, pixelWidth, pixelHeight } = page;
  if (![150, 300].includes(dpi) || actual.rotate !== geometry.rotation || !Number.isFinite(actual.userUnit)
    || Math.abs(actual.userUnit - geometry.userUnit) > 0.0001 || actual.view.length !== 4
    || actual.view.some((value, index) => !Number.isFinite(value) || Math.abs(value - geometry.bounds[index]) > 0.01)
    || !Number.isFinite(actual.width) || !Number.isFinite(actual.height)
    || Math.abs(actual.width - geometry.widthPoints) > 0.01 || Math.abs(actual.height - geometry.heightPoints) > 0.01
    || Math.ceil(actual.width * dpi / 72) !== pixelWidth || Math.ceil(actual.height * dpi / 72) !== pixelHeight
    || !Number.isSafeInteger(pixelWidth) || !Number.isSafeInteger(pixelHeight) || pixelWidth < 1 || pixelHeight < 1
    || pixelWidth !== Math.ceil(geometry.widthPoints * dpi / 72) || pixelHeight !== Math.ceil(geometry.heightPoints * dpi / 72)
    || pixelWidth > ARCHIVE_LIMITS.dimension || pixelHeight > ARCHIVE_LIMITS.dimension || pixelWidth * pixelHeight > ARCHIVE_LIMITS.pagePixels) {
    throw new Error(`Source page ${page.sourcePage + 1} has inconsistent or unsupported archival geometry. No copy was saved.`);
  }
}
