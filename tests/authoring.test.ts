import test from "node:test";
import assert from "node:assert/strict";
import { destinationPage, flattenBookmarks, safeExternalUrl } from "../src/lib/navigation.ts";
import { parsePageRange, parseWritingNumber, rgbFromHex } from "../src/lib/pages.ts";

test("page ranges preserve user order and reject duplicate, out-of-range, and partial values", () => {
  assert.deepEqual(parsePageRange(" 2-4, 1, 6 ", 6), [1, 2, 3, 0, 5]);
  for (const value of ["", "0", "7", "3-1", "1,1", "1-3,2", "1x", "1.5", "1,,2", "1-", "1-999999999999"]) {
    assert.throws(() => parsePageRange(value, 6));
  }
});

test("writing settings reject invalid input instead of silently clamping", () => {
  assert.equal(parseWritingNumber("14", 6, 96, "Text size"), 14);
  for (const value of ["", "5", "97", "NaN", "1e100"]) assert.throws(() => parseWritingNumber(value, 6, 96, "Text size"));
  assert.deepEqual(rgbFromHex("#ff8000"), [1, 128 / 255, 0]);
  assert.throws(() => rgbFromHex("red"));
});

test("bookmark trees retain hierarchy and destinations", () => {
  const entries = flattenBookmarks([{ title: "Start", dest: [0, { name: "Fit" }], items: [
    { title: "Chapter", dest: "chapter", items: [] },
  ] }]);
  assert.equal(entries.length, 2);
  assert.equal(entries[1].depth, 1);
  assert.equal(entries[1].destination, "chapter");
  assert.deepEqual(flattenBookmarks(null), []);
  assert.throws(() => flattenBookmarks([{ title: 2 }]));
  assert.throws(() => flattenBookmarks({}, 0));
});

test("PDF destinations accept named, indexed, and indirect pages with explicit bounds", async () => {
  const pdf = { numPages: 3, getDestination: async () => [{ num: 4, gen: 0 }, { name: "Fit" }],
    getPageIndex: async () => 1 };
  assert.equal(await destinationPage(pdf, "chapter"), 1);
  assert.equal(await destinationPage(pdf, [0, { name: "Fit" }]), 0);
  await assert.rejects(destinationPage(pdf, [3, { name: "Fit" }]));
  await assert.rejects(destinationPage(pdf, [{ num: -1, gen: 0 }]));
  await assert.rejects(destinationPage(pdf, null));
});

test("external PDF links allow only credential-free web or email addresses", () => {
  assert.equal(safeExternalUrl("https://example.org/document"), "https://example.org/document");
  assert.equal(safeExternalUrl("mailto:reader@example.org"), "mailto:reader@example.org");
  for (const value of ["javascript:alert(1)", "file:///etc/passwd", "data:text/html,test",
    "https://user:secret@example.org", "https://example.org/\nvalue", "not a URL"]) {
    assert.throws(() => safeExternalUrl(value));
  }
});
