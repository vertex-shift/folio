import assert from "node:assert/strict";
import test from "node:test";
import { archivePages, assertTechnicalCurrent, validateArchiveGeometry } from "../src/features/preflight/model.ts";
import type { ArchivePage } from "../src/features/preflight/model.ts";

const page: ArchivePage = {
  sourcePage: 4, pixelWidth: 834, pixelHeight: 1417,
  geometry: { mediaBox: [-20, -10, 400, 250], bounds: [10, 20, 350, 220], originalBounds: [10, 20, 350, 220],
    rotation: 90, userUnit: 2, widthPoints: 400, heightPoints: 680, cropModified: false },
};
const actual = { rotate: 90, userUnit: 2, view: [10, 20, 350, 220], width: 400, height: 680,
  rasterWidth: 200 * (150 / 72) * 2, rasterHeight: 340 * (150 / 72) * 2 };

test("archival selection never silently truncates, reorders or duplicates source pages", () => {
  assert.deepEqual(archivePages(true, "", 3), [0, 1, 2]);
  assert.deepEqual(archivePages(false, "2-4, 6", 8), [1, 2, 3, 5]);
  assert.equal(archivePages(true, "", 100).length, 100);
  assert.throws(() => archivePages(true, "", 101), /100/);
  for (const count of [0, 1.2, Infinity, -1]) assert.throws(() => archivePages(true, "", count));
  for (const range of ["", "1,1", "3,1", "1-101", "102", "2-1"]) assert.throws(() => archivePages(false, range, 101));
  assert.throws(() => archivePages(false, "1".repeat(4097), 1));
});

test("archive source checks guard identity, revision and every aggregate draft domain", () => {
  const source = { documentId: 5, revision: 7 };
  assert.doesNotThrow(() => assertTechnicalCurrent(source, { id: 5, revision: 7 }, false));
  for (const current of [null, { id: 6, revision: 7 }, { id: 5, revision: 8 }]) {
    assert.throws(() => assertTechnicalCurrent(source, current, false), /source changed/);
  }
  assert.throws(() => assertTechnicalCurrent(source, { id: 5, revision: 7 }, true), /live-gesture/);
});

test("archival renderer checks CropBox, right-angle rotation, UserUnit and physical pixels against the engine", () => {
  assert.doesNotThrow(() => validateArchiveGeometry(page, actual, 150));
  for (const changed of [{ ...actual, rotate: 0 }, { ...actual, userUnit: 1 },
    { ...actual, view: [0, 0, 612, 792] }, { ...actual, width: 200 }, { ...actual, height: NaN }]) {
    assert.throws(() => validateArchiveGeometry(page, changed, 150), /Source page 5/);
  }
  assert.throws(() => validateArchiveGeometry(page, actual, 300));
  assert.throws(() => validateArchiveGeometry({ ...page, pixelWidth: 833 }, actual, 150));
  assert.throws(() => validateArchiveGeometry(page, actual, 96));
  const rounded: ArchivePage = { ...page, pixelWidth: 750, pixelHeight: 150,
    geometry: { ...page.geometry, widthPoints: 360, heightPoints: 72 } };
  assert.throws(() => validateArchiveGeometry(rounded, { ...actual, width: 360.000001, height: 72, rasterWidth: 360.000001 * (150 / 72), rasterHeight: 72 * (150 / 72) }, 150),
    /inconsistent or unsupported/, "Near-integer source geometry must not lose its last pixel column");
  const noisy: ArchivePage = { sourcePage: 0, pixelWidth: Math.ceil(240 * (150 / 72)), pixelHeight: Math.ceil(144 * (150 / 72)),
    geometry: { ...page.geometry, rotation: 0, userUnit: 1, bounds: [10, 20, 250, 164], widthPoints: 240, heightPoints: 144 } };
  assert.doesNotThrow(() => validateArchiveGeometry(noisy, { rotate: 0, userUnit: 1, view: [10, 20, 250, 164], width: 240, height: 144,
    rasterWidth: 240 * (150 / 72), rasterHeight: 144 * (150 / 72) }, 150));
});
