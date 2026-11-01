import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const workerSha256 = "bb422f60804477de4a0b09091d8347a916a387755ed1cbad24ccaec1d09c78bf";
const prefix = "FolioOCRLogicalV1_";

export function preparePdfWorker(root) {
  const source = readFileSync(join(root, "node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs"), "utf8");
  if (createHash("sha256").update(source).digest("hex") !== workerSha256) {
    throw new Error("The PDF.js 6.4.299 OCR extraction adapter requires the pinned worker. Restore the npm lockfile instead of patching an unknown renderer.");
  }
  const anchors = [
    ["          fontFamily: font.fallbackName,", `          fontFamily: font.fallbackName,\n          folioOcrLogical: /^${prefix}(?:latin|arabic|devanagari|cjk_sc|cjk_jp|cjk_kr)_(?:ltr|rtl)$/.test(font.name),`],
    ["        if (saveLastChar(glyphUnicode)) {", `        if (!/^${prefix}(?:latin|arabic|devanagari|cjk_sc|cjk_jp|cjk_kr)_(?:ltr|rtl)$/.test(font.name)\n          && saveLastChar(glyphUnicode)) {`],
    ["        if (!category.isZeroWidthDiacritic && !compareWithLastPosition(scaledDim)) {", `        if (!/^${prefix}(?:latin|arabic|devanagari|cjk_sc|cjk_jp|cjk_kr)_(?:ltr|rtl)$/.test(font.name)\n          && !category.isZeroWidthDiacritic && !compareWithLastPosition(scaledDim)) {`],
    ["          category,\n          originalCharCode\n        } = glyph;", `          originalCharCode\n        } = glyph;\n        const category = /^${prefix}(?:latin|arabic|devanagari|cjk_sc|cjk_jp|cjk_kr)_(?:ltr|rtl)$/.test(font.name)\n          ? { isWhitespace: false, isZeroWidthDiacritic: false, isInvisibleFormatMark: false }\n          : glyph.category;`],
    ["      textContentItem.fontName = loadedName;", `      textContentItem.folioOcrDirection = /^${prefix}(?:latin|arabic|devanagari|cjk_sc|cjk_jp|cjk_kr)_(ltr|rtl)$/.exec(font.name)?.[1];\n      textContentItem.fontName = loadedName;`],
    ['      const bidiResult = bidi(text, -1, textChunk.vertical);', '      const bidiResult = textChunk.folioOcrDirection\n        ? { str: textChunk.str.join(""), dir: textChunk.folioOcrDirection }\n        : bidi(text, -1, textChunk.vertical);'],
  ];
  let adapted = source;
  for (const [before, after] of anchors) {
    if (adapted.split(before).length !== 2) throw new Error("The pinned PDF.js OCR extraction adapter anchor is ambiguous.");
    adapted = adapted.replace(before, after);
  }
  const output = join(root, "public/vendor/pdfjs/folio-worker.mjs");
  if (!existsSync(output) || readFileSync(output, "utf8") !== adapted) writeFileSync(output, adapted);
}
