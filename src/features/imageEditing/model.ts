import type { PageGeometry } from "../../lib/authoring.ts";
import type { DocumentInfo } from "../../lib/types.ts";
import type { ImageChange, ImageEditDraft, ImageOccurrence, ImagePlacement } from "./types.ts";

export const imagePath = (path: number[]) => path.join(".");
const fields = ["x", "y", "width", "height"] as const;

export function newImageDraft(info: DocumentInfo, page: number, source: ImageOccurrence): ImageEditDraft {
  if (!info.pages[page]) throw new Error("This image page is no longer available.");
  const [x, y, width, height] = source.placement?.map(String) ?? ["", "", "", ""];
  return { documentId: info.id, revision: info.revision, page, source, x, y, width, height, remove: false, replacement: null };
}

export function imageGeometryChanged(draft: ImageEditDraft): boolean {
  return fields.some((field, index) => {
    const original = draft.source.placement?.[index];
    if (original === undefined) return draft[field] !== "";
    return !draft[field].trim() || !Number.isFinite(Number(draft[field])) || Number(draft[field]) !== original;
  });
}

export function imageDraftPending(draft: ImageEditDraft | null): boolean {
  return Boolean(draft && (draft.remove || draft.replacement || imageGeometryChanged(draft)));
}

export function assertImageDraft(info: DocumentInfo, draft: ImageEditDraft | null): asserts draft is ImageEditDraft {
  if (!draft) throw new Error("Select a source image occurrence first.");
  if (info.id !== draft.documentId || info.revision !== draft.revision || !info.pages[draft.page]) {
    throw new Error("The PDF changed. Your image draft is kept; discard it and inspect the current revision.");
  }
}

export function draftPlacement(draft: ImageEditDraft, geometry: PageGeometry): ImagePlacement {
  const values = fields.map((field) => {
    const text = draft[field];
    if (!text.trim() || !Number.isFinite(Number(text))) throw new Error("Enter finite X, Y, width and height values in physical points.");
    return Number(text);
  });
  const [x, y, width, height] = values;
  if (x < 0 || y < 0 || width < 0.1 || height < 0.1
    || x + width > geometry.widthPoints + 0.001 || y + height > geometry.heightPoints + 0.001) {
    throw new Error("Keep the complete image inside the visible page, with width and height of at least 0.1 pt.");
  }
  return [x, y, width, height];
}

export function imageChange(info: DocumentInfo, draft: ImageEditDraft | null): ImageChange {
  assertImageDraft(info, draft);
  if (info.readOnlyReason) throw new Error(info.readOnlyReason);
  if (!draft.source.editable) throw new Error(draft.source.reason ?? "This image occurrence cannot be edited safely.");
  if (!imageDraftPending(draft)) throw new Error("Choose a replacement, move or resize the image, or mark it for removal.");
  const { page, source: { path } } = draft;
  if (draft.remove) return { action: "remove", page, path };
  const placement = imageGeometryChanged(draft) ? draftPlacement(draft, info.pages[page].geometry) : null;
  if (draft.replacement) return { action: "replace", page, path, assetId: draft.replacement.assetId, placement };
  if (!placement) throw new Error("Move or resize the image before applying.");
  return { action: "place", page, path, placement };
}

export function withPlacement(draft: ImageEditDraft, placement: ImagePlacement): ImageEditDraft {
  const [x, y, width, height] = placement.map((value) => String(Math.round(value * 1000) / 1000));
  return { ...draft, x, y, width, height };
}

export function gesturePlacement(start: ImagePlacement, dx: number, dy: number, resize: boolean, geometry: PageGeometry): ImagePlacement {
  const [x, y, width, height] = start;
  return resize
    ? [x, y, Math.max(0.1, Math.min(geometry.widthPoints - x, width + dx)), Math.max(0.1, Math.min(geometry.heightPoints - y, height + dy))]
    : [Math.max(0, Math.min(geometry.widthPoints - width, x + dx)), Math.max(0, Math.min(geometry.heightPoints - height, y + dy)), width, height];
}
