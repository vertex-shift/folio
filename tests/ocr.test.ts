import test, { type TestContext } from "node:test";
import assert from "node:assert/strict";
import { awaitOcrStep, recognitionWords } from "../src/lib/ocr.ts";
import { LocalOcrWorker } from "../src/lib/ocrWorker.ts";
import type { OcrPagePlan } from "../src/lib/ocrTypes.ts";
import { parsePageRange } from "../src/lib/pages.ts";

const page: OcrPagePlan = { page: 2, sourceBounds: [-20, 30, 320, 230], rotation: 90,
  userUnit: 2, pixelWidth: 1000, pixelHeight: 1500, skipped: false, existingText: false };

test("OCR range limits are checked before expanding large source page ranges", () => {
  assert.throws(() => parsePageRange("1-1000000", 1_000_000, 50), /no more than 50/);
  assert.throws(() => parsePageRange("1-30, 40-70", 100, 50), /no more than 50/);
  assert.equal(parsePageRange("1-50", 100, 50).length, 50);
  assert.throws(() => parsePageRange("1", 100, NaN));
  assert.throws(() => parsePageRange("1", 100, 0));
});

function output() {
  return { text: "FOLIO OFFLINE", blocks: [{ paragraphs: [{ is_ltr: true, lines: [{
    baseline: { x0: 50, y0: 100, x1: 500, y1: 109, has_baseline: true },
    words: [{ text: "FOLIO", bbox: { x0: 50, y0: 70, x1: 180, y1: 100 } },
      { text: "OFFLINE", bbox: { x0: 220, y0: 75, x1: 500, y1: 109 } }],
  }] }] }] };
}

test("OCR preserves actual word boxes and interpolates each word's line baseline", () => {
  const words = recognitionWords(output(), page);
  assert.equal(words.length, 2);
  assert.deepEqual(words[0], { text: "FOLIO", bounds: [50, 70, 180, 100],
    baseline: [50, 100, 180, 102.6], line: 0 });
  assert.deepEqual(words[1].baseline, [220, 103.4, 500, 109]);
});

test("blank recognition is a genuine no-text outcome, not fabricated or geometry-free text", () => {
  assert.deepEqual(recognitionWords({ text: "", blocks: null }, page), []);
  assert.deepEqual(recognitionWords({ text: "\n", blocks: [] }, page), []);
  assert.throws(() => recognitionWords({ text: "Text without geometry", blocks: [] }, page));
  assert.throws(() => recognitionWords({ text: "Text without geometry", blocks: null }, page));
});

test("invalid coordinates and unsupported direction fail the entire page", () => {
  for (const invalid of [null, {}, { text: "test", blocks: {} }]) {
    assert.throws(() => recognitionWords(invalid, page));
  }
  const outside = output();
  outside.blocks[0].paragraphs[0].lines[0].words[1].bbox.x1 = 2000;
  assert.throws(() => recognitionWords(outside, page));
  const vertical = output();
  vertical.blocks[0].paragraphs[0].lines[0].baseline.y1 = 800;
  assert.throws(() => recognitionWords(vertical, page));
  const missing = output();
  missing.blocks[0].paragraphs[0].lines[0].baseline.has_baseline = false;
  assert.throws(() => recognitionWords(missing, page));
  const rtl = output();
  rtl.blocks[0].paragraphs[0].is_ltr = false;
  assert.throws(() => recognitionWords(rtl, page));
});

test("cancel interrupts pending renderer/encoding/staging steps and handles their late result safely", async () => {
  const controller = new AbortController();
  let finish: (value: number) => void = () => {};
  const operation = new Promise<number>((resolve) => { finish = resolve; });
  const step = awaitOcrStep(operation, controller.signal);
  controller.abort();
  await assert.rejects(step);
  finish(1);
  const alreadyCanceled = new AbortController();
  alreadyCanceled.abort();
  await assert.rejects(awaitOcrStep(Promise.resolve(2), alreadyCanceled.signal));
});

interface Packet {
  jobId: string;
  action: string;
  payload: Record<string, unknown>;
}

class FakeWorker {
  static instances: FakeWorker[] = [];
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: ((event: { preventDefault(): void }) => void) | null = null;
  onmessageerror: (() => void) | null = null;
  packets: Packet[] = [];
  terminated = false;
  automatic = false;
  url: string;
  constructor(url: string) { this.url = url; FakeWorker.instances.push(this); }
  postMessage(packet: Packet) {
    this.packets.push(packet);
    if (this.automatic) queueMicrotask(() => this.reply(packet, packet.action === "recognize" ? output() : undefined));
  }
  reply(packet: Packet, data: unknown) {
    this.onmessage?.({ data: { jobId: packet.jobId, status: "resolve", data } });
  }
  terminate() { this.terminated = true; }
}

function fakeBrowser(context: TestContext) {
  for (const [key, value] of [["window", { location: { href: "http://127.0.0.1:54321/" } }], ["Worker", FakeWorker]] as const) {
    const original = Object.getOwnPropertyDescriptor(globalThis, key);
    Object.defineProperty(globalThis, key, { value, configurable: true });
    context.after(() => original ? Object.defineProperty(globalThis, key, original) : Reflect.deleteProperty(globalThis, key));
  }
  FakeWorker.instances = [];
}

test("cancel during engine initialization terminates the raw worker and rejects pending work", async (context) => {
  fakeBrowser(context);
  const controller = new AbortController();
  const worker = new LocalOcrWorker(controller.signal, () => {});
  const loading = worker.initialize(150);
  assert.match(FakeWorker.instances[0].url, /^http:\/\/127\.0\.0\.1:54321\/vendor\/ocr\/worker\.min\.js$/);
  controller.abort();
  await assert.rejects(loading);
  assert.equal(FakeWorker.instances[0].terminated, true);
  assert.equal(FakeWorker.instances[0].packets.length, 1);
});

test("every engine/model path is explicitly local and no language cache or autorotation is used", async (context) => {
  fakeBrowser(context);
  const worker = new LocalOcrWorker(new AbortController().signal, () => {});
  const raw = FakeWorker.instances[0];
  raw.automatic = true;
  await worker.initialize(300);
  assert.deepEqual(raw.packets[0].payload, {
    options: { corePath: "http://127.0.0.1:54321/vendor/ocr/", lstmOnly: true, logging: false },
  });
  assert.deepEqual(raw.packets[1].payload, { langs: "eng", options: {
    langPath: "http://127.0.0.1:54321/vendor/ocr/", gzip: false, cacheMethod: "none", lstmOnly: true,
  } });
  const result = await worker.recognize(new ArrayBuffer(8));
  assert.equal(recognitionWords(result, page).length, 2);
  assert.deepEqual(raw.packets[4].payload.options, { rotateAuto: false, rotateRadians: 0 });
  worker.close();
  assert.equal(raw.terminated, true);
});

test("worker errors are visible, terminate resources and never include recognized content", async (context) => {
  fakeBrowser(context);
  const worker = new LocalOcrWorker(new AbortController().signal, () => {});
  const pending = worker.initialize(150);
  const raw = FakeWorker.instances[0];
  raw.onmessage?.({ data: { jobId: raw.packets[0].jobId, status: "reject", data: "private recognized content" } });
  await assert.rejects(pending, (error: Error) => !error.message.includes("private recognized content"));
  assert.equal(raw.terminated, true);
});
