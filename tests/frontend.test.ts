import test from "node:test";
import assert from "node:assert/strict";
import { selectionQuads, pageFromInput, formatBytes } from "../src/lib/geometry.ts";
import { findMatches, mapText } from "../src/lib/search.ts";

test("search escapes special characters and preserves Unicode offsets", () => {
  assert.deepEqual(findMatches("A [PDF] and a [pdf].", "[pdf]"), [
    { start: 2, end: 7 }, { start: 14, end: 19 },
  ]);
  assert.deepEqual(findMatches("ABC abc", "ABC", true), [{ start: 0, end: 3 }]);
  assert.deepEqual(findMatches("A \u{1F4C4} PDF", "PDF"), [{ start: 5, end: 8 }]);
  assert.deepEqual(findMatches("Nothing", "  "), []);
});

test("multi-line phrases match actual text-run boundaries", () => {
  const mapped = mapText([{ str: "Review the", hasEOL: true }, { str: "document carefully." }]);
  const matches = findMatches(mapped.text, "the document");
  assert.deepEqual(matches, [{ start: 7, end: 19 }]);
  assert.equal(mapped.runs[1].start, 11);
  assert.equal(findMatches("a a a a", "a", false, 2).length, 2);
});

test("selection rectangles are clipped, deduplicated, and converted to PDF coordinates", () => {
  const page = { left: 100, top: 50, right: 300, bottom: 450 };
  const quads = selectionQuads([
    { left: 90, top: 70, right: 180, bottom: 90 },
    { left: 100, top: 70, right: 180, bottom: 90 },
    { left: 181, top: 70, right: 250, bottom: 90 },
    { left: 130, top: 100, right: 130, bottom: 120 },
  ], page, (x, y) => [x / 2, 200 - y / 2]);
  assert.deepEqual(quads, [[0, 190, 75, 190, 0, 180, 75, 180]]);
});

test("rotated page coordinate conversion keeps all four corners", () => {
  const quads = selectionQuads([
    { left: 10, top: 20, right: 50, bottom: 40 },
  ], { left: 0, top: 0, right: 200, bottom: 300 }, (x, y) => [y, x]);
  assert.deepEqual(quads, [[20, 10, 20, 50, 40, 10, 40, 50]]);
});

test("page inputs reject partial numbers, invalid ranges, and non-integers", () => {
  assert.equal(pageFromInput(" 3 ", 3), 2);
  for (const value of ["0", "4", "2a", "1.5", "-1", "Infinity", ""]) {
    assert.equal(pageFromInput(value, 3), null);
  }
  assert.equal(formatBytes(1536), "1.5 KB");
});
