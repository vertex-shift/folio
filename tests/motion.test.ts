import assert from "node:assert/strict";
import test from "node:test";
import { SELECTION_MOTION, selectionTransform } from "../src/lib/motion.ts";

test("selection surfaces invert position and size without animating layout", () => {
  assert.equal(selectionTransform(
    { left: 10, top: 20, width: 100, height: 40 },
    { left: 210, top: 30, width: 200, height: 40 },
  ), "translate(-200px, -10px) scale(0.5, 1)");
  assert.ok(SELECTION_MOTION.duration > 100 && SELECTION_MOTION.duration < 300);
});

test("an interrupted selection starts from its current visible position", () => {
  assert.equal(selectionTransform(
    { left: 143, top: 7, width: 88, height: 41 },
    { left: 25, top: 7, width: 88, height: 41 },
  ), "translate(118px, 0px) scale(1, 1)");
});

test("identical, subpixel, hidden, and invalid rectangles never create a spurious animation", () => {
  const rect = { left: 10, top: 20, width: 100, height: 40 };
  assert.equal(selectionTransform(rect, rect), null);
  assert.equal(selectionTransform(rect, { ...rect, left: 10.25 }), null);
  assert.equal(selectionTransform(rect, { ...rect, width: 0 }), null);
  assert.equal(selectionTransform({ ...rect, height: -1 }, rect), null);
  assert.equal(selectionTransform(rect, { ...rect, top: Number.NaN }), null);
});

test("DOMRect-style prototype getters are validated rather than treated as enumerable fields", () => {
  class InvalidRect {
    get left() { return Number.NaN; }
    get top() { return 0; }
    get width() { return 100; }
    get height() { return 40; }
  }
  assert.equal(selectionTransform(new InvalidRect(), { left: 0, top: 0, width: 100, height: 40 }), null);
});
