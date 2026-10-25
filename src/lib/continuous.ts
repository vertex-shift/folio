import type { Zoom } from "./types.ts";

export interface PageSize { width: number; height: number }
export interface PageFrame extends PageSize { top: number; scale: number }
export interface PageLayout { pages: PageFrame[]; width: number; height: number }

export function pageScale(size: PageSize, width: number, height: number, zoom: Zoom): number {
  if (zoom.mode === "custom") return zoom.scale;
  const availableWidth = Math.max(120, width - (width < 560 ? 48 : 128));
  const availableHeight = Math.max(160, height - 108);
  return zoom.mode === "page" ? Math.min(availableWidth / size.width, availableHeight / size.height)
    : availableWidth / size.width;
}

export function continuousLayout(sizes: readonly PageSize[], width: number, height: number, zoom: Zoom): PageLayout {
  let top = 24;
  let widest = 0;
  const pages = sizes.map((size) => {
    const scale = pageScale(size, width, height, zoom);
    const frame = { top, scale, width: size.width * scale, height: size.height * scale };
    top += frame.height + 24;
    widest = Math.max(widest, frame.width);
    return frame;
  });
  return { pages, width: Math.max(width, widest + (width < 560 ? 48 : 128)),
    height: Math.max(height, top + 60, (pages.at(-1)?.top ?? 24) + height - 24) };
}

function firstEndingAfter(pages: readonly PageFrame[], position: number): number {
  let left = 0, right = pages.length;
  while (left < right) {
    const middle = (left + right) >>> 1;
    if (pages[middle].top + pages[middle].height <= position) left = middle + 1;
    else right = middle;
  }
  return left;
}

export function visiblePage(pages: readonly PageFrame[], top: number, height: number): number {
  if (!pages.length) return 0;
  let selected = Math.min(firstEndingAfter(pages, top), pages.length - 1);
  let greatest = -1;
  for (let index = selected; index < pages.length && pages[index].top < top + height; index++) {
    const page = pages[index];
    const overlap = Math.max(0, Math.min(page.top + page.height, top + height) - Math.max(page.top, top));
    if (overlap > greatest) { greatest = overlap; selected = index; }
  }
  return selected;
}

export function renderPages(pages: readonly PageFrame[], top: number, height: number): number[] {
  const result: number[] = [];
  for (let index = firstEndingAfter(pages, top - height);
    index < pages.length && pages[index].top < top + height * 2; index++) result.push(index);
  return result;
}
