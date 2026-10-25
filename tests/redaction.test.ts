import test from "node:test";
import assert from "node:assert/strict";
import { redactionFromFields, validateRedactionRect } from "../src/lib/redaction.ts";

test("keyboard coordinate entry produces the same PDF rectangle as a dragged region", () => {
  assert.deepEqual(redactionFromFields(["54", "630", "350", "70"], [0, 0, 612, 792]), [54, 630, 404, 700]);
  assert.deepEqual(redactionFromFields(["-10", "20", "30", "40"], [-20, -20, 100, 200]), [-10, 20, 20, 60]);
});

test("rejects missing, malformed, non-finite, and non-positive region inputs", () => {
  for (const fields of [["", "1", "20", "30"], ["1", "2", "0", "30"], ["1", "2", "-1", "30"],
    ["1", "2", "20", "Infinity"], ["no", "2", "20", "30"], ["1", "2"]]) {
    assert.throws(() => redactionFromFields(fields, [0, 0, 612, 792]));
  }
  assert.throws(() => validateRedactionRect([0, 0, NaN, 10], [0, 0, 612, 792]));
  assert.throws(() => validateRedactionRect([1, 1, 10, 10], [0, 0, 0, 792]));
});

test("never accepts an area outside the visible page or one erased by clipping", () => {
  assert.throws(() => validateRedactionRect([-1, 0, 10, 10], [0, 0, 612, 792]));
  assert.throws(() => validateRedactionRect([1, 1, 613, 10], [0, 0, 612, 792]));
  assert.throws(() => validateRedactionRect([-0.009, 1, -0.001, 10], [0, 0, 612, 792]));
  assert.deepEqual(validateRedactionRect([-0.00001, 0, 612.00001, 792], [0, 0, 612, 792]), [0, 0, 612, 792]);
});
