import { parsePageRange, rgbFromHex } from "./pages.ts";
import type { DocumentInfo, Edit, PdfRect } from "./types.ts";

export interface PageGeometry {
  mediaBox: PdfRect;
  bounds: PdfRect;
  originalBounds: PdfRect;
  rotation: number;
  userUnit: number;
  widthPoints: number;
  heightPoints: number;
  cropModified: boolean;
}

export interface ImageAsset {
  assetId: number;
  width: number;
  height: number;
  previewPng: number[];
}

export interface ImagePlacement {
  page: number;
  placement: [number, number, number, number];
  opacity: number;
}

export type AuthoringKind = "image" | "crop" | "watermark" | "numbers";
export type NumberPosition = "top_left" | "top_center" | "top_right" | "bottom_left" | "bottom_center" | "bottom_right";

interface BaseDraft {
  kind: AuthoringKind;
  documentId: number;
  revision: number;
  page: number;
  dirty: boolean;
  scope: "current" | "all" | "range";
  range: string;
}

export interface ImageDraft extends BaseDraft {
  kind: "image";
  asset: Omit<ImageAsset, "previewPng"> | null;
  previewUrl: string | null;
  name: string;
  x: string;
  y: string;
  width: string;
  height: string;
  opacity: string;
  lockAspect: boolean;
}

export interface CropDraft extends BaseDraft {
  kind: "crop";
  margins: [string, string, string, string];
}

export interface WatermarkDraft extends BaseDraft {
  kind: "watermark";
  text: string;
  size: string;
  color: string;
  opacity: string;
  angle: string;
}

export interface NumberDraft extends BaseDraft {
  kind: "numbers";
  start: string;
  prefix: string;
  suffix: string;
  position: NumberPosition;
  size: string;
  color: string;
  margin: string;
}

export type AuthoringDraft = ImageDraft | CropDraft | WatermarkDraft | NumberDraft;
export type AuthoringAction = "apply" | "remove_crop" | "reset_crop" | "remove_stamps";

export const MAX_IMAGE_BYTES = 16 * 1024 * 1024;

export function createAuthoringDraft(kind: AuthoringKind, info: DocumentInfo, page: number): AuthoringDraft {
  const base = { documentId: info.id, revision: info.revision, page, dirty: false,
    scope: "current" as const, range: String(page + 1) };
  if (kind === "image") return { ...base, kind, asset: null, previewUrl: null, name: "",
    x: "36", y: "36", width: "240", height: "160", opacity: "100", lockAspect: true };
  if (kind === "crop") return { ...base, kind, margins: ["0", "0", "0", "0"] };
  if (kind === "watermark") return { ...base, kind, text: "", size: "48", color: "#626d7e", opacity: "25", angle: "45" };
  return { ...base, kind, start: "1", prefix: "", suffix: "", position: "bottom_center", size: "12", color: "#293241", margin: "24" };
}

export function authoringPending(draft: AuthoringDraft | null): boolean {
  return Boolean(draft?.dirty || draft?.kind === "image" && draft.asset);
}

export function assertAuthoringRevision(draft: AuthoringDraft, info: DocumentInfo): void {
  if (draft.documentId !== info.id || draft.revision !== info.revision || !info.pages[draft.page]) {
    throw new Error("This page-authoring draft belongs to an older document revision. Discard it and review the current PDF.");
  }
}

function field(value: string, min: number, max: number, label: string): number {
  const number = Number(value);
  if (!value.trim() || !Number.isFinite(number) || number < min || number > max) {
    throw new Error(`${label} must be between ${min} and ${max}.`);
  }
  return number;
}

export function authoringPages(draft: AuthoringDraft, info: DocumentInfo): number[] {
  assertAuthoringRevision(draft, info);
  if (draft.scope === "current") return [draft.page];
  if (draft.scope === "all") return Array.from({ length: info.pageCount }, (_, page) => page);
  return parsePageRange(draft.range, info.pageCount);
}

export function imagePlacement(draft: ImageDraft, geometry: PageGeometry): ImagePlacement {
  const x = field(draft.x, 0, geometry.widthPoints, "Left");
  const y = field(draft.y, 0, geometry.heightPoints, "Top");
  const width = field(draft.width, 0.1, geometry.widthPoints, "Image width");
  const height = field(draft.height, 0.1, geometry.heightPoints, "Image height");
  if (x + width > geometry.widthPoints + 0.001 || y + height > geometry.heightPoints + 0.001) {
    throw new Error("Keep the complete image inside the visible page. Reduce its size or adjust its left/top position.");
  }
  return { page: draft.page, placement: [x, y, width, height], opacity: field(draft.opacity, 1, 100, "Opacity (%)") / 100 };
}

export function fittedImage(draft: ImageDraft, geometry: PageGeometry): ImageDraft {
  if (!draft.asset) throw new Error("Choose a PNG or JPEG first.");
  const scale = Math.min(240 / draft.asset.width, 240 / draft.asset.height,
    geometry.widthPoints * 0.8 / draft.asset.width, geometry.heightPoints * 0.8 / draft.asset.height);
  const width = draft.asset.width * scale, height = draft.asset.height * scale;
  return { ...draft, dirty: true, width: decimal(width), height: decimal(height),
    x: decimal((geometry.widthPoints - width) / 2), y: decimal((geometry.heightPoints - height) / 2) };
}

export function decimal(value: number): string {
  return String(Math.round(value * 1000) / 1000);
}

export function imageDimension(draft: ImageDraft, dimension: "width" | "height", value: string): ImageDraft {
  const next = { ...draft, [dimension]: value, dirty: true };
  const number = Number(value);
  if (draft.lockAspect && draft.asset && value.trim() && Number.isFinite(number) && number > 0) {
    if (dimension === "width") next.height = decimal(number * draft.asset.height / draft.asset.width);
    else next.width = decimal(number * draft.asset.width / draft.asset.height);
  }
  return next;
}

export function authoringEdit(draft: AuthoringDraft, info: DocumentInfo, action: AuthoringAction = "apply"): Edit {
  const pages = authoringPages(draft, info);
  if (draft.kind === "image") throw new Error("Use the selected-image placement command.");
  if (draft.kind === "crop") {
    if (action === "remove_crop") return { type: "remove_crop", pages };
    if (action === "reset_crop") return { type: "reset_crop", pages };
    if (action !== "apply") throw new Error("Choose a crop operation.");
    const margins = draft.margins.map((value) => field(value, 0, 1_000_000_000, "Crop margin"));
    const [top, right, bottom, left] = margins;
    if (!margins.some((value) => value > 0)) throw new Error("Set a crop margin or drag the area you want to keep.");
    for (const page of pages) {
      const geometry = info.pages[page].geometry;
      if (geometry.widthPoints - left - right < 1 || geometry.heightPoints - top - bottom < 1) {
        throw new Error(`Crop margins must leave at least one physical point of width and height on page ${page + 1}.`);
      }
    }
    return { type: "crop", pages, margins: [top, right, bottom, left] };
  }
  if (action === "remove_stamps") return { type: "remove_authoring", pages,
    kind: draft.kind === "watermark" ? "watermark" : "page_number" };
  if (action !== "apply") throw new Error("Choose an operation for this authoring tool.");
  if (draft.kind === "watermark") {
    if (!draft.text.trim() || [...draft.text].length > 160 || /[\r\n\t]/.test(draft.text)) {
      throw new Error("Enter a single watermark line of 1 to 160 characters.");
    }
    return { type: "watermark", pages, settings: { text: draft.text,
      size: field(draft.size, 6, 144, "Font size (pt)"), color: rgbFromHex(draft.color),
      opacity: field(draft.opacity, 1, 100, "Opacity (%)") / 100, angle: field(draft.angle, -90, 90, "Angle (degrees)") } };
  }
  const start = field(draft.start, 0, 999_999, "Starting number");
  if (!Number.isSafeInteger(start) || start + pages.length - 1 > 999_999) {
    throw new Error("Page numbers must be whole numbers between 0 and 999,999.");
  }
  if ([...draft.prefix].length > 80 || [...draft.suffix].length > 80 || /[\r\n\t]/.test(draft.prefix + draft.suffix)) {
    throw new Error("Use single-line prefixes and suffixes of at most 80 characters each.");
  }
  return { type: "page_numbers", pages, settings: { start, prefix: draft.prefix, suffix: draft.suffix,
    position: draft.position, size: field(draft.size, 6, 144, "Font size (pt)"),
    color: rgbFromHex(draft.color), margin: field(draft.margin, 0, 720, "Edge margin (pt)") } };
}
