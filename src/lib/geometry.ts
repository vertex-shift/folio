import type { Quad } from "./types.ts";

export interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export function intersectsPdfRect(rect: readonly number[], bounds: readonly number[]): boolean {
  return Math.max(rect[0], rect[2]) > bounds[0] && Math.min(rect[0], rect[2]) < bounds[2]
    && Math.max(rect[1], rect[3]) > bounds[1] && Math.min(rect[1], rect[3]) < bounds[3];
}

export function selectionQuads(
  rectangles: readonly Rect[],
  page: Rect,
  convert: (x: number, y: number) => number[],
): Quad[] {
  const clipped = rectangles.map((rectangle) => ({
    left: Math.max(page.left, rectangle.left),
    top: Math.max(page.top, rectangle.top),
    right: Math.min(page.right, rectangle.right),
    bottom: Math.min(page.bottom, rectangle.bottom),
  })).filter((rectangle) =>
    rectangle.right - rectangle.left > 0.75 && rectangle.bottom - rectangle.top > 1,
  ).sort((a, b) => a.top - b.top || a.left - b.left);

  const merged: Rect[] = [];
  for (const rectangle of clipped) {
    const last = merged.at(-1);
    if (last && Math.abs(last.top - rectangle.top) < 1.5
      && Math.abs(last.bottom - rectangle.bottom) < 1.5
      && rectangle.left <= last.right + 1) {
      last.right = Math.max(last.right, rectangle.right);
      last.left = Math.min(last.left, rectangle.left);
    } else {
      merged.push({ ...rectangle });
    }
  }
  const point = (x: number, y: number): [number, number] => {
    const values = convert(x - page.left, y - page.top);
    if (values.length < 2 || !Number.isFinite(values[0]) || !Number.isFinite(values[1])) {
      throw new Error("The selected passage has invalid PDF coordinates.");
    }
    return [values[0], values[1]];
  };
  return merged.map((rectangle) => [
    ...point(rectangle.left, rectangle.top),
    ...point(rectangle.right, rectangle.top),
    ...point(rectangle.left, rectangle.bottom),
    ...point(rectangle.right, rectangle.bottom),
  ]);
}

export function pageFromInput(value: string, count: number): number | null {
  if (!/^\d+$/.test(value.trim())) return null;
  const page = Number(value.trim());
  return Number.isSafeInteger(page) && page >= 1 && page <= count ? page - 1 : null;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "An unexpected error occurred. Try the action again or reopen the document.";
}
