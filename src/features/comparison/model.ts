import { diffArrays } from "diff";
import type { PageGeometry } from "../../lib/authoring.ts";
import type { DocumentInfo } from "../../lib/types.ts";

export const LIMITS = {
  pages: 100, sourceBytes: 64 * 1024 * 1024, pagePixels: 8_000_000,
  totalPixels: 100_000_000, dimension: 8192, pageTextBytes: 256 * 1024,
  textBytes: 2 * 1024 * 1024, pageTokens: 20_000, tokens: 100_000,
  spans: 4000, reportBytes: 4 * 1024 * 1024, previewBytes: 32 * 1024 * 1024,
  previewDimension: 1200,
} as const;

export interface ComparisonSettings { dpi: 72 | 96 | 150; tolerance: number }
export interface SourceRef { documentId: number; revision: number }
export interface ComparisonSource extends SourceRef {
  name: string; pageCount: number; sourceBytes: number;
  passwordProtected: boolean; pages: PageGeometry[];
}
export interface PagePlan { index: number; width: number; height: number }
export interface ComparisonPlan {
  comparisonId: number; before: ComparisonSource; after: ComparisonSource;
  settings: ComparisonSettings; pages: PagePlan[]; scope: string; suggestedName: string;
}
export interface SourceState { document: DocumentInfo | null; pending: boolean; busy: boolean }
export type SpanKind = "equal" | "added" | "removed";
export interface TextSpan { kind: SpanKind; text: string }
export interface TextResult { beforeTokens: number; afterTokens: number; spans: TextSpan[] }
export interface PixelResult {
  width: number; height: number; changed: number; total: number;
  maxChannelDelta: number; bounds: [number, number, number, number] | null;
}
export interface PageResult { index: number; text: TextResult; pixels: PixelResult }
export interface PagePreview { before: Blob | null; after: Blob | null; mask: Blob }
export interface ComparisonResult { plan: ComparisonPlan; pages: PageResult[]; previews: PagePreview[] }
export interface ComparisonProgress { page: number; count: number; stage: "snapshots" | "text" | "pixels" | "finishing" }
export type GeometryChange = "media_box" | "effective_crop" | "rotation" | "user_unit" | "physical_size";

export function validateSettings(settings: ComparisonSettings): void {
  if (![72, 96, 150].includes(settings.dpi) || !Number.isInteger(settings.tolerance)
    || settings.tolerance < 0 || settings.tolerance > 64) {
    throw new Error("Choose 72, 96 or 150 DPI and an RGB tolerance from 0 to 64.");
  }
}

export function assertComparisonCurrent(
  plan: { before: SourceRef; after: SourceRef }, current: (id: number) => SourceState,
): void {
  for (const [label, source] of [["Before", plan.before], ["After", plan.after]] as const) {
    const state = current(source.documentId);
    if (!state.document || state.document.id !== source.documentId || state.document.revision !== source.revision) {
      throw new Error(`${label} PDF changed or closed. Compare the current PDFs again.`);
    }
    if (state.pending) throw new Error(`${label} PDF has pending writing, form, metadata, page-authoring, redaction or gesture drafts. Apply or discard them before comparing.`);
    if (state.busy) throw new Error(`${label} PDF is busy. Finish its current action before comparing.`);
    if (!state.document.permissions.copyAllowed || !state.document.copyAllowed) {
      throw new Error(`${label} PDF does not permit copying/extraction. Reopen it with the owner password.`);
    }
  }
}

export function alignedPages(before: readonly PageGeometry[], after: readonly PageGeometry[], settings: ComparisonSettings): PagePlan[] {
  validateSettings(settings);
  const count = Math.max(before.length, after.length);
  if (!before.length || !after.length || count > LIMITS.pages) throw new Error("Comparison requires two PDFs of 1-100 pages. No pages are silently skipped.");
  let total = 0;
  return Array.from({ length: count }, (_, index) => {
    let width = 0, height = 0;
    for (const geometry of [before[index], after[index]]) {
      if (!geometry) continue;
      const w = Math.ceil(geometry.widthPoints * settings.dpi / 72);
      const h = Math.ceil(geometry.heightPoints * settings.dpi / 72);
      if (!Number.isFinite(w) || !Number.isFinite(h) || w < 1 || h < 1 || w > LIMITS.dimension || h > LIMITS.dimension) {
        throw new Error(`Page ${index + 1} exceeds 8,192 pixels per dimension. Choose lower DPI.`);
      }
      width = Math.max(width, w); height = Math.max(height, h);
    }
    total += width * height;
    if (width * height > LIMITS.pagePixels || total > LIMITS.totalPixels) {
      throw new Error(`Page ${index + 1} exceeds 8 megapixels per aligned pair or 100 megapixels total. Choose lower DPI; quality is not silently lowered.`);
    }
    return { index, width, height };
  });
}

export function geometryChanges(before?: PageGeometry, after?: PageGeometry): GeometryChange[] {
  if (!before || !after) return [];
  const changes: GeometryChange[] = [];
  if (before.mediaBox.some((value, index) => value !== after.mediaBox[index])) changes.push("media_box");
  if (before.bounds.some((value, index) => value !== after.bounds[index])) changes.push("effective_crop");
  if (before.rotation !== after.rotation) changes.push("rotation");
  if (before.userUnit !== after.userUnit) changes.push("user_unit");
  if (before.widthPoints !== after.widthPoints || before.heightPoints !== after.heightPoints) changes.push("physical_size");
  return changes;
}

export function pagePresence(plan: ComparisonPlan, index: number): "matched" | "added" | "removed" {
  return index >= plan.before.pageCount ? "added" : index >= plan.after.pageCount ? "removed" : "matched";
}

export function pageChanged(plan: ComparisonPlan, page: PageResult): boolean {
  return pagePresence(plan, page.index) !== "matched" || page.pixels.changed > 0
    || page.text.spans.some((span) => span.kind !== "equal")
    || geometryChanges(plan.before.pages[page.index], plan.after.pages[page.index]).length > 0;
}

export function textTokens(text: string): string[] {
  if (new TextEncoder().encode(text).length > LIMITS.pageTextBytes) {
    throw new Error("A page exceeds the 256 KiB extracted-text limit. No text was truncated.");
  }
  const tokens: string[] = [];
  for (const match of text.matchAll(/\s+|[\p{L}\p{N}\p{M}_]+|[^\s]/gu)) {
    if (tokens.length === LIMITS.pageTokens) throw new Error("A page exceeds 20,000 text tokens. No text was truncated.");
    tokens.push(match[0]);
  }
  return tokens;
}

export function compareText(before: string, after: string): TextResult {
  const left = textTokens(before), right = textTokens(after);
  if (before === after || !before || !after) {
    const kind: SpanKind = before === after ? "equal" : before ? "removed" : "added";
    return { beforeTokens: left.length, afterTokens: right.length, spans: before || after ? [{ kind, text: before || after }] : [] };
  }
  const changes = diffArrays(left, right, { maxEditLength: LIMITS.spans, timeout: 1500 });
  if (!changes) throw new Error("The text changes exceed the bounded diff complexity/time limit. No incomplete comparison was reported.");
  if (changes.length > LIMITS.spans) throw new Error("The text comparison exceeds 4,000 spans. No changes were truncated.");
  return { beforeTokens: left.length, afterTokens: right.length,
    spans: changes.map((change): TextSpan => ({
      kind: change.added ? "added" : change.removed ? "removed" : "equal",
      text: change.value.join(""),
    })).filter((span) => span.text.length > 0) };
}

export function comparePixels(
  before: Uint8ClampedArray, after: Uint8ClampedArray, width: number, height: number, tolerance: number,
): { pixels: PixelResult; mask: Uint8ClampedArray } {
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width < 1 || height < 1
    || width > LIMITS.dimension || height > LIMITS.dimension || width * height > LIMITS.pagePixels
    || before.length !== width * height * 4 || after.length !== before.length
    || !Number.isInteger(tolerance) || tolerance < 0 || tolerance > 64) {
    throw new Error("The pixel comparison has invalid dimensions, buffers or tolerance.");
  }
  const mask = new Uint8ClampedArray(before.length);
  let changed = 0, maxChannelDelta = 0, minX = width, minY = height, maxX = -1, maxY = -1;
  for (let offset = 0; offset < before.length; offset += 4) {
    const delta = Math.max(Math.abs(before[offset] - after[offset]),
      Math.abs(before[offset + 1] - after[offset + 1]), Math.abs(before[offset + 2] - after[offset + 2]));
    maxChannelDelta = Math.max(maxChannelDelta, delta);
    if (delta <= tolerance) continue;
    changed++;
    const pixel = offset / 4, x = pixel % width, y = Math.floor(pixel / width);
    minX = Math.min(minX, x); minY = Math.min(minY, y); maxX = Math.max(maxX, x); maxY = Math.max(maxY, y);
    mask[offset] = 190; mask[offset + 1] = 20; mask[offset + 2] = 100; mask[offset + 3] = 190;
  }
  return { pixels: { width, height, changed, total: width * height, maxChannelDelta,
    bounds: changed ? [minX, minY, maxX - minX + 1, maxY - minY + 1] : null }, mask };
}

export class ComparisonBudget {
  textBytes = 0;
  tokens = 0;
  spans = 0;
  previewBytes = 0;

  addTextBytes(bytes: number): void {
    this.textBytes += bytes;
    if (this.textBytes > LIMITS.textBytes) throw new Error("Comparison exceeds 2 MiB of extracted text across both PDFs. No text was truncated.");
  }

  addText(result: TextResult): void {
    this.tokens += result.beforeTokens + result.afterTokens;
    this.spans += result.spans.length;
    if (this.tokens > LIMITS.tokens || this.spans > LIMITS.spans) {
      throw new Error("Comparison exceeds 100,000 text tokens or 4,000 spans. No differences were truncated.");
    }
  }

  addPreview(bytes: number): void {
    this.previewBytes += bytes;
    if (this.previewBytes > LIMITS.previewBytes) throw new Error("Comparison previews exceed 32 MiB. Choose lower DPI or smaller PDFs.");
  }
}

export function encodeResults(pages: readonly PageResult[]): Uint8Array {
  const bytes = new TextEncoder().encode(JSON.stringify(pages));
  if (bytes.length > LIMITS.reportBytes) throw new Error("Comparison results exceed the 4 MiB report limit. No truncated report was exported.");
  return bytes;
}
