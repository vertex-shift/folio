import assert from "node:assert/strict";
import test from "node:test";
import { assertOutputCurrent, outputPages, printPageRule, rasterGeometry } from "../src/lib/output.ts";

test("output ranges include non-visible pages and preserve the selected order", () => {
  assert.deepEqual(outputPages(false, "24, 2-3, 1", 24, "print"), [23, 1, 2, 0]);
  assert.deepEqual(outputPages(true, "", 24, "png"), Array.from({ length: 24 }, (_, index) => index));
  assert.deepEqual(outputPages(false, "500", 500, "text"), [499]);
  assert.throws(() => outputPages(false, "1, 1", 24, "print"), /more than once/);
  assert.throws(() => outputPages(false, "25", 24, "print"));
  assert.throws(() => outputPages(false, "", 24, "text"));
});

test("range expansion is bounded before allocating large output selections", () => {
  assert.equal(outputPages(true, "", 200, "print").length, 200);
  assert.throws(() => outputPages(true, "", 201, "print"), /200/);
  assert.equal(outputPages(true, "", 500, "png").length, 500);
  assert.throws(() => outputPages(true, "", 501, "text"), /500/);
  assert.throws(() => outputPages(false, "1-999999999", 999999999, "text"), /500/);
  assert.throws(() => outputPages(false, "1".repeat(4097), 24, "print"), /too long/);
  for (const count of [0, -1, NaN, Infinity, 1.5]) assert.throws(() => outputPages(true, "", count, "png"));
});

test("raster geometry uses physical PDF.js viewport points, not raw MediaBox coordinates", () => {
  assert.deepEqual(rasterGeometry({ width: 400, height: 680 }, 150, 2),
    { widthPoints: 400, heightPoints: 680, pixelWidth: 834, pixelHeight: 1417 });
  assert.deepEqual(rasterGeometry({ width: 600, height: 450 }, 96, 3),
    { widthPoints: 600, heightPoints: 450, pixelWidth: 800, pixelHeight: 600 });
  assert.deepEqual(rasterGeometry({ width: 612, height: 792 }, 300, 1),
    { widthPoints: 612, heightPoints: 792, pixelWidth: 2550, pixelHeight: 3300 });
});

test("pixel budgets are inclusive at the exact threshold and never silently downscale", () => {
  assert.equal(rasterGeometry({ width: 2880, height: 1920 }, 150, 7).pixelWidth, 6000);
  assert.throws(() => rasterGeometry({ width: 2880.01, height: 1920 }, 150, 7), /Page 7 exceeds 24 megapixels/);
  assert.equal(rasterGeometry({ width: 12288, height: 1 }, 96, 1).pixelWidth, 16384);
  assert.throws(() => rasterGeometry({ width: 12288.01, height: 1 }, 96, 1), /16,384/);
  for (const base of [{ width: 0, height: 1 }, { width: 1, height: -1 }, { width: Infinity, height: 1 }, { width: 1, height: NaN }]) {
    assert.throws(() => rasterGeometry(base, 150, 1), /unsupported output geometry/);
  }
});

test("prepared print rules carry exact physical size and stable names without untrusted strings", () => {
  assert.equal(printPageRule(rasterGeometry({ width: 400, height: 680 }, 150, 2), 1),
    "@page folio-output-1 { size: 400.000000pt 680.000000pt; margin: 0; }");
  assert.throws(() => printPageRule({ widthPoints: NaN, heightPoints: 10, pixelWidth: 1, pixelHeight: 1 }, 0));
  assert.throws(() => printPageRule({ widthPoints: 10, heightPoints: 10, pixelWidth: 1, pixelHeight: 1 }, -1));
});

test("stale document identities, revisions and unapplied drafts block output", () => {
  const plan = { documentId: 3, revision: 12 };
  assert.doesNotThrow(() => assertOutputCurrent(plan, { id: 3, revision: 12 }, false));
  for (const info of [null, { id: 4, revision: 12 }, { id: 3, revision: 13 }]) {
    assert.throws(() => assertOutputCurrent(plan, info, false), /document changed/);
  }
  assert.throws(() => assertOutputCurrent(plan, { id: 3, revision: 12 }, true), /redaction marks/);
});
