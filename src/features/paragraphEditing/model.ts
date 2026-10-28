import type { PageGeometry } from "../../lib/authoring.ts";
import type { DocumentInfo } from "../../lib/types.ts";
import type { ParagraphBlock, ParagraphChange, ParagraphDraft, ParagraphPlacement } from "./types.ts";

export const MAX_PARAGRAPH_CHARACTERS = 8_000;
const fields = ["x", "y", "width", "height"] as const;
export const paragraphPath = (path: number[]) => path.join(".");

export function newParagraphDraft(info: DocumentInfo, page: number, epoch: number, source: ParagraphBlock): ParagraphDraft {
  if (!info.pages[page]) throw new Error("This paragraph page is no longer available.");
  const [x, y, width, height] = source.placement?.map(String) ?? ["", "", "", ""];
  return { documentId: info.id, revision: info.revision, epoch, page, source,
    text: source.text ?? "", x, y, width, height, size: String(source.size), leading: String(source.leading),
    alignment: source.alignment, font: source.fontMode };
}

export function paragraphDraftPending(draft: ParagraphDraft | null): boolean {
  return Boolean(draft && (draft.text !== (draft.source.text ?? "") || draft.font !== draft.source.fontMode
    || draft.alignment !== draft.source.alignment
    || changed(draft.size, draft.source.size) || changed(draft.leading, draft.source.leading)
    || fields.some((field, index) => changed(draft[field], draft.source.placement?.[index]))));
}

function changed(value: string, original: number | undefined): boolean {
  return original === undefined ? value !== "" : !value.trim() || !Number.isFinite(Number(value)) || Number(value) !== original;
}

export function assertParagraphDraft(info: DocumentInfo, draft: ParagraphDraft | null): asserts draft is ParagraphDraft {
  if (!draft) throw new Error("Select an existing source paragraph first.");
  if (info.id !== draft.documentId || info.revision !== draft.revision || !info.pages[draft.page]) {
    throw new Error("The PDF changed. Your paragraph draft is kept; discard it and inspect the current revision.");
  }
}

export function paragraphPlacement(draft: ParagraphDraft, geometry: PageGeometry): ParagraphPlacement {
  const values = fields.map((field) => finite(draft[field], "X, Y, width and height"));
  const [x, y, width, height] = values;
  if (x < 0 || y < 0 || width < 0.1 || height < 0.1
    || x + width > geometry.widthPoints + 0.001 || y + height > geometry.heightPoints + 0.001) {
    throw new Error("Keep the entire paragraph box inside the visible page, with width and height of at least 0.1 pt.");
  }
  return [x, y, width, height];
}

function finite(value: string, label: string): number {
  if (!value.trim() || !Number.isFinite(Number(value))) throw new Error(`Enter finite ${label} values in physical points.`);
  return Number(value);
}

export function paragraphChange(info: DocumentInfo, draft: ParagraphDraft | null): ParagraphChange {
  assertParagraphDraft(info, draft);
  if (info.readOnlyReason) throw new Error(info.readOnlyReason);
  if (!draft.source.editable) throw new Error(draft.source.reason ?? "This source paragraph is read-only.");
  if (Array.from(draft.text).length > MAX_PARAGRAPH_CHARACTERS) throw new Error("A paragraph supports at most 8,000 characters.");
  const size = finite(draft.size, "text size"), leading = finite(draft.leading, "leading");
  if (size < 2 || size > 144 || leading < 1 || leading > 720) {
    throw new Error("Choose a text size from 2 to 144 pt and leading from 1 to 720 pt.");
  }
  return { epoch: draft.epoch, page: draft.page, path: draft.source.path, blockId: draft.source.id,
    expected: draft.source.text ?? "", text: draft.text, placement: paragraphPlacement(draft, info.pages[draft.page].geometry),
    size, leading, alignment: draft.alignment, font: draft.font };
}

export function withParagraphPlacement(draft: ParagraphDraft, placement: ParagraphPlacement): ParagraphDraft {
  const [x, y, width, height] = placement.map((value) => String(Math.round(value * 1000) / 1000));
  return { ...draft, x, y, width, height };
}

export function paragraphGesture(start: ParagraphPlacement, dx: number, dy: number, resize: boolean, geometry: PageGeometry): ParagraphPlacement {
  const [x, y, width, height] = start;
  return resize
    ? [x, y, Math.max(0.1, Math.min(geometry.widthPoints - x, width + dx)), Math.max(0.1, Math.min(geometry.heightPoints - y, height + dy))]
    : [Math.max(0, Math.min(geometry.widthPoints - width, x + dx)), Math.max(0, Math.min(geometry.heightPoints - height, y + dy)), width, height];
}
