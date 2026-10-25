import test from "node:test";
import assert from "node:assert/strict";
import { continuousLayout, pageScale, renderPages, visiblePage } from "../src/lib/continuous.ts";

test("continuous pages preserve each page's dimensions, rotations, and spacing", () => {
  const layout = continuousLayout([{ width: 612, height: 792 }, { width: 792, height: 612 },
    { width: 420, height: 900 }], 1000, 800, { mode: "custom", scale: 1 });
  assert.deepEqual(layout.pages.map((page) => [page.top, page.width, page.height]),
    [[24, 612, 792], [840, 792, 612], [1476, 420, 900]]);
  assert.equal(layout.height, 2460);
  assert.equal(layout.width, 1000);
});

test("fit page and width match the actual page renderer, including narrow previews", () => {
  assert.equal(pageScale({ width: 612, height: 792 }, 293, 600, { mode: "width" }), 245 / 612);
  assert.equal(pageScale({ width: 612, height: 792 }, 1000, 800, { mode: "page" }), 692 / 792);
  const layout = continuousLayout([{ width: 612, height: 792 }], 800, 600, { mode: "custom", scale: 4 });
  assert.equal(layout.width, 2576);
  assert.equal(layout.pages[0].width, 2448);
});

test("the current page follows the largest visible area rather than a page boundary", () => {
  const { pages } = continuousLayout(Array.from({ length: 3 }, () => ({ width: 612, height: 792 })),
    1000, 800, { mode: "custom", scale: 1 });
  assert.equal(visiblePage(pages, 0, 800), 0);
  assert.equal(visiblePage(pages, 500, 800), 1);
  assert.equal(visiblePage(pages, 1600, 800), 2);
  assert.equal(visiblePage(pages, 10000, 800), 2);
  assert.equal(visiblePage([], 0, 800), 0);
});

test("large files render only a viewport neighborhood, including jumps to the last page", () => {
  const { pages } = continuousLayout(Array.from({ length: 1000 }, () => ({ width: 612, height: 792 })),
    1000, 800, { mode: "custom", scale: 1 });
  assert.deepEqual(renderPages(pages, 0, 800), [0, 1]);
  const nearEnd = renderPages(pages, pages[999].top, 800);
  assert(nearEnd.includes(999));
  assert(nearEnd.length <= 4);
  const middle = renderPages(pages, pages[500].top, 800);
  assert(middle.includes(500));
  assert(middle.length <= 4);
});

test("a short final page can align with the viewport instead of selecting its predecessor", () => {
  const layout = continuousLayout([{ width: 612, height: 792 }, { width: 792, height: 200 }],
    1000, 800, { mode: "custom", scale: 0.25 });
  const destination = layout.pages[1].top - 24;
  assert(destination <= layout.height - 800);
  assert.equal(visiblePage(layout.pages, destination, 800), 1);
});
