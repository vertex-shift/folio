import assert from "node:assert/strict";
import test from "node:test";
import {
  creationSettings, moveCreationInput, validateCreationFiles, validateDocxText, waitForCreation,
} from "../src/features/conversion/model.ts";
import { outputPages } from "../src/lib/output.ts";

test("creation settings explicitly select physical page size, orientation, margins and readable font bounds", () => {
  assert.deepEqual(creationSettings("letter", false, "36", "12"),
    { width: 612, height: 792, margin: 36, fontSize: 12 });
  assert.deepEqual(creationSettings("a4", true, "144", "36"),
    { width: 841.89, height: 595.28, margin: 144, fontSize: 36 });
  for (const margin of ["", "NaN", "Infinity", "-0.1", "144.01"]) {
    assert.throws(() => creationSettings("letter", false, margin, "12"), /Margin/);
  }
  for (const font of ["", "NaN", "7.99", "36.01"]) {
    assert.throws(() => creationSettings("letter", false, "0", font), /Text size/);
  }
});

test("creation batches enforce inclusive exact file/count/aggregate limits before upload", () => {
  const mib = 1024 * 1024;
  assert.doesNotThrow(() => validateCreationFiles("images", Array.from({ length: 4 }, () => ({ name: "Image.png", size: 16 * mib }))));
  assert.throws(() => validateCreationFiles("images", [{ name: "Image.jpg", size: 16 * mib + 1 }]), /16 MiB/);
  assert.throws(() => validateCreationFiles("images", [...Array.from({ length: 4 }, () => ({ name: "Image.png", size: 16 * mib })), { name: "Extra.png", size: 1 }]), /64 MiB/);
  assert.doesNotThrow(() => validateCreationFiles("text", Array.from({ length: 4 }, () => ({ name: "Text.txt", size: mib }))));
  assert.throws(() => validateCreationFiles("text", [{ name: "Text.txt", size: mib + 1 }]), /1 MiB/);
  assert.throws(() => validateCreationFiles("text", Array.from({ length: 5 }, () => ({ name: "Text.txt", size: mib }))), /4 MiB/);
  assert.doesNotThrow(() => validateCreationFiles("text", Array.from({ length: 50 }, () => ({ name: "Text.txt", size: 1 }))));
  assert.throws(() => validateCreationFiles("text", Array.from({ length: 51 }, () => ({ name: "Text.txt", size: 1 }))), /50/);
  assert.throws(() => validateCreationFiles("images", []), /1 to 50/);
  for (const size of [0, -1, 0.5, NaN, Infinity]) assert.throws(() => validateCreationFiles("text", [{ name: "Text.txt", size }]));
  assert.throws(() => validateCreationFiles("images", [{ name: "Photo.gif", size: 1 }]), /still PNG or JPEG/);
  for (const name of ["", "../Text.txt", "bad\\Text.txt", "bad\0Text.txt", "a".repeat(513)]) {
    assert.throws(() => validateCreationFiles("text", [{ name, size: 1 }]), /filename/);
  }
});

test("keyboard reorder operations preserve every original file exactly once without mutating the previous order", () => {
  const original = [0, 1, 2];
  const changed = moveCreationInput(original, 2, -1);
  assert.deepEqual(original, [0, 1, 2]);
  assert.deepEqual(changed, [0, 2, 1]);
  assert.deepEqual(moveCreationInput(changed, 0, 1), [2, 0, 1]);
  assert.throws(() => moveCreationInput(original, 0, -1));
  assert.throws(() => moveCreationInput(original, 2, 1));
  assert.throws(() => moveCreationInput(original, 1.5, 1));
});

test("DOCX accepts real extracted Unicode/XML literals and refuses unsupported XML controls or malformed surrogates", () => {
  assert.doesNotThrow(() => validateDocxText("Zoë Ελληνικά Привет 漢字 😀 <w:t>&\"'\t\r\n"));
  for (const text of ["bad\0text", "\u0001", "\u000b", "\u001f", "\ufffe", "\uffff", "\ud800", "\udfff"]) {
    assert.throws(() => validateDocxText(text), /No text was silently removed/);
  }
});

test("DOCX selection retains requested order with an inclusive 200-page limit", () => {
  assert.deepEqual(outputPages(false, "3, 1-2", 4, "docx"), [2, 0, 1]);
  assert.equal(outputPages(true, "", 200, "docx").length, 200);
  assert.throws(() => outputPages(true, "", 201, "docx"), /200/);
  assert.throws(() => outputPages(false, "1-201", 1000, "docx"), /200/);
  assert.throws(() => outputPages(false, "3, 1-3", 4, "docx"), /more than once/);
});

test("creation waits for the complete result, and abort releases polling without committing anything", async () => {
  let calls = 0;
  const ready = await waitForCreation(7, async (jobId) => {
    calls++;
    return { jobId, phase: calls === 2 ? "ready" : "preparing", pageCount: calls === 2 ? 3 : null, byteLength: calls === 2 ? 1024 : null };
  }, new AbortController().signal);
  assert.equal(calls, 2);
  assert.equal(ready.pageCount, 3);
  const abort = new AbortController();
  let canceledCalls = 0;
  const waiting = waitForCreation(8, async (jobId) => {
    canceledCalls++;
    abort.abort();
    return { jobId, phase: "preparing", pageCount: null, byteLength: null };
  }, abort.signal);
  await assert.rejects(waiting, (error: unknown) => error instanceof DOMException && error.name === "AbortError");
  assert.equal(canceledCalls, 1);
});
