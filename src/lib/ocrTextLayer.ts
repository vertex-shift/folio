import type { PageViewport } from "pdfjs-dist";
import type { TextContent } from "pdfjs-dist/types/src/display/api";
import type { TextLayer } from "pdfjs-dist/legacy/build/pdf.mjs";

export function alignOcrTextLayer(content: TextContent, layer: TextLayer, viewport: PageViewport): void {
  const items = content.items.filter((item) => "str" in item);
  const targets = items.map((item, index) => ({ item, div: layer.textDivs[index], style: content.styles[item.fontName] }))
    .filter(({ item, style, div }) => div && item.str.length === 1
      && item.str.trim() && style && "folioOcrLogical" in style && style.folioOcrLogical === true);
  if (!targets.length) return;
  const canvas = document.createElement("canvas");
  try {
    const context = canvas.getContext("2d");
    if (!context) throw new Error("The OCR selection layer could not measure its single-character word boxes.");
    for (const { item, div } of targets) {
      const style = getComputedStyle(div);
      context.font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
      const measured = context.measureText(item.str).width;
      const expected = item.width * viewport.scale * viewport.userUnit;
      if (!Number.isFinite(expected) || expected <= 0 || !Number.isFinite(measured) || measured <= 0) {
        throw new Error("An OCR single-character selection has invalid font geometry.");
      }
      div.style.setProperty("--scale-x", String(expected / measured));
    }
  } finally { canvas.width = 0; canvas.height = 0; }
}
