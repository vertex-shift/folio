import test, { type TestContext } from "node:test";
import assert from "node:assert/strict";
import { awaitOcrStep, horizontalOcrBands, ocrResolutionIssue, recognitionWords, shiftOcrWords } from "../src/lib/ocr.ts";
import { LocalOcrWorker } from "../src/lib/ocrWorker.ts";
import { verifiedOcrAsset } from "../src/lib/ocrWorker.ts";
import { OCR_LANGUAGES, selectedOcrLanguages } from "../src/lib/ocrLanguages.ts";
import { readFileSync } from "node:fs";
import type { OcrPagePlan } from "../src/lib/ocrTypes.ts";
import { parsePageRange } from "../src/lib/pages.ts";
import type { DocumentInfo } from "../src/lib/types.ts";
import { OCR_RESOLUTIONS } from "../src/lib/ocrTypes.ts";

const page: OcrPagePlan = { page: 2, sourceBounds: [-20, 30, 320, 230], rotation: 90,
  userUnit: 2, pixelWidth: 1000, pixelHeight: 1500, skipped: false, existingText: false };

function pageGeometry(width: number, height: number, userUnit = 1): DocumentInfo["pages"][number] {
  const bounds: [number, number, number, number] = [-32, 64, width - 32, height + 64];
  return { key: "synthetic-scan", rotation: 0, geometry: {
    mediaBox: bounds, bounds, originalBounds: bounds, rotation: 0, userUnit,
    widthPoints: width * userUnit, heightPoints: height * userUnit, cropModified: false,
  } };
}

test("oversized scan pages recommend an explicit safe resolution instead of silently resampling", () => {
  const pages = [pageGeometry(600, 800), pageGeometry(2400, 3600)];
  assert.match(ocrResolutionIssue(pages, [0, 1], 300) ?? "", /Page 2.*300 DPI.*Choose 96 DPI/);
  assert.match(ocrResolutionIssue(pages, [1], 150) ?? "", /Choose 96 DPI/);
  for (const dpi of [72, 96] as const) assert.equal(ocrResolutionIssue(pages, [0, 1], dpi), null);
  assert.equal(ocrResolutionIssue(pages, [0], 300), null);
  const narrow = [pageGeometry(20_000, 10)];
  assert.match(ocrResolutionIssue(narrow, [0], 72) ?? "", /Even 72 DPI exceeds/);
  assert.throws(() => ocrResolutionIssue(pages, [2], 96), /invalid OCR geometry/);
  assert.throws(() => ocrResolutionIssue([pageGeometry(600, 800, NaN)], [0], 96), /invalid OCR geometry/);
});

test("OCR budget recommendations include pixel ceilings, CropBox rotation and UserUnit", () => {
  const exact = pageGeometry(4000, 6000);
  assert.equal(ocrResolutionIssue([exact], [0], 72), null);
  assert.match(ocrResolutionIssue([pageGeometry(4000.001, 6000)], [0], 72) ?? "", /exceeds/);
  const fractional = pageGeometry(144, 288, 1.25);
  fractional.geometry.rotation = 270;
  for (const dpi of OCR_RESOLUTIONS) assert.equal(ocrResolutionIssue([fractional], [0], dpi), null);
  assert.match(ocrResolutionIssue([pageGeometry(2000, 3000, 2)], [0], 96) ?? "", /Choose 72 DPI/);
});

test("OCR range limits are checked before expanding large source page ranges", () => {
  assert.throws(() => parsePageRange("1-1000000", 1_000_000, 50), /no more than 50/);
  assert.throws(() => parsePageRange("1-30, 40-70", 100, 50), /no more than 50/);
  assert.equal(parsePageRange("1-50", 100, 50).length, 50);
  assert.throws(() => parsePageRange("1", 100, NaN));
  assert.throws(() => parsePageRange("1", 100, 0));
});

function output() {
  return { text: "FOLIO OFFLINE", blocks: [{ paragraphs: [{ is_ltr: true, lines: [{
    text: "FOLIO OFFLINE", bbox: { x0: 50, y0: 70, x1: 500, y1: 110 },
    baseline: { x0: 50, y0: 100, x1: 500, y1: 109, has_baseline: true },
    words: [{ text: "FOLIO", bbox: { x0: 50, y0: 70, x1: 180, y1: 100 } },
      { text: "OFFLINE", bbox: { x0: 220, y0: 75, x1: 500, y1: 109 } }],
  }] }] }] };
}

test("OCR preserves actual word boxes and interpolates each word's line baseline", () => {
  const words = recognitionWords(output(), page);
  assert.equal(words.length, 2);
  assert.deepEqual(words[0], { text: "FOLIO", bounds: [50, 70, 180, 100],
    baseline: [50, 100, 180, 102.6], line: 0, lineBounds: [50,70,500,110], direction: "ltr", separator: " " });
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
  const invalidDirection = output();
  Reflect.deleteProperty(invalidDirection.blocks[0].paragraphs[0], "is_ltr");
  assert.throws(() => recognitionWords(invalidDirection, page));
});

test("above-baseline Japanese punctuation retains its real line box rather than manufacturing a word baseline", () => {
  const result = { text: "ー\n", blocks: [{ blocktype:1, paragraphs:[{is_ltr:1,lines:[{
    text:"ー\n",bbox:{x0:50,y0:70,x1:500,y1:110},
    baseline:{x0:50,y0:105,x1:500,y1:105},words:[{text:"ー",bbox:{x0:100,y0:75,x1:180,y1:80}}],
  }]}]}] };
  const [word] = recognitionWords(result,page);
  assert.deepEqual(word.baseline,[100,105,180,105]);
  assert.deepEqual(word.lineBounds,[50,70,500,110]);
  result.blocks[0].blocktype=7;
  assert.throws(()=>recognitionWords(result,page),/Vertical/);
});

test("single-column bands preserve actual DPI and positions, merge nearby marks and reject dark/excessive inputs", async () => {
  const width=40,height=100,pixels=new Uint8ClampedArray(width*height*4).fill(255);
  const row=(y:number)=>{for(let x=10;x<30;x++)pixels.fill(0,(y*width+x)*4,(y*width+x)*4+3);};
  for(const y of [10,11,12,17,18,19,50,51,52])row(y);
  const signal=new AbortController().signal;
  const bands=await horizontalOcrBands(pixels,width,height,300,signal);
  assert.equal(bands.length,2);
  assert.ok(bands[0].top<=10 && bands[0].top+bands[0].height>=20);
  assert.ok(bands[0].top+bands[0].height<=bands[1].top);
  assert.deepEqual(await horizontalOcrBands(new Uint8ClampedArray(width*height*4).fill(255),width,height,300,signal),[]);
  await assert.rejects(horizontalOcrBands(new Uint8ClampedArray(width*height*4),width,height,300,signal),/light-background/);
  const abort=new AbortController();
  const scan=horizontalOcrBands(pixels,width,height,300,abort.signal);
  abort.abort(new Error("Cancel band scan"));
  await assert.rejects(scan,/Cancel band scan/);
  const words=recognitionWords(output(),page),shifted=shiftOcrWords(words,40,7);
  assert.equal(shifted[0].text,words[0].text);
  assert.equal(shifted[0].separator,words[0].separator);
  assert.equal(shifted[0].bounds[1],words[0].bounds[1]+40);
  assert.equal(shifted[0].baseline[1],words[0].baseline[1]+40);
  assert.equal(shifted[0].lineBounds[1],words[0].lineBounds[1]+40);
  assert.equal(shifted[0].line,7);
});

test("all fifteen fixed languages and one-to-three distinct combinations are validated without path injection or fallback", () => {
  assert.equal(OCR_LANGUAGES.length, 15);
  for (const language of OCR_LANGUAGES) assert.deepEqual(selectedOcrLanguages([language.id]), [language.id]);
  assert.deepEqual(selectedOcrLanguages(["ara", "hin", "eng"]), ["ara", "hin", "eng"]);
  for (const invalid of [[], ["eng", "eng"], ["eng", "fra", "deu", "spa"], ["../eng"], ["jpn_vert"], ["osd"]]) {
    assert.throws(() => selectedOcrLanguages(invalid));
  }
});

test("logical RTL/Indic text and separators preserve every combining mark and joiner without normalization", () => {
  const result = { text: "هذا كِتاب\nक्‍षि e\u0301\n", blocks: [{ paragraphs: [{
    is_ltr: 0, lines: [{ text: "هذا كِتاب\n", bbox: { x0:50,y0:70,x1:500,y1:102 },
      baseline: { x0: 50, y0: 100, x1: 500, y1: 100 },
      words: [{ text: "هذا", bbox: { x0: 300, y0: 70, x1: 500, y1: 100 } },
        { text: "كِتاب", bbox: { x0: 50, y0: 70, x1: 200, y1: 100 } }] }],
  }, { is_ltr: 1, lines: [{ text: "क्‍षि e\u0301\n", bbox: { x0:50,y0:160,x1:500,y1:203 },
    baseline: { x0: 50, y0: 200, x1: 500, y1: 200 },
    words: [{ text: "क्‍षि", bbox: { x0: 50, y0: 160, x1: 200, y1: 200 } },
      { text: "e\u0301", bbox: { x0: 300, y0: 160, x1: 400, y1: 200 } }] }],
  }] }] };
  const words = recognitionWords(result, page);
  assert.equal(words.map((word) => word.text + word.separator).join(""), result.text);
  assert.equal(words[0].direction, "rtl");
  assert.equal(words[0].bounds[0], 300, "logical RTL order must not be sorted geometrically");
  assert.equal(words[2].text, "क्‍षि");
  assert.equal(words[3].text, "e\u0301");
  assert.throws(() => recognitionWords({ ...result, text: result.text + "lost" }, page));
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
  const fetchAsset = async (url: string | URL | Request) => {
    const name = new URL(String(url)).pathname.split("/").at(-1)!;
    if (!/^[a-z_]+\.traineddata$/.test(name)) throw new Error(`Unexpected asset ${name}`);
    return new Response(readFileSync(new URL(`../src-tauri/resources/ocr/${name}`, import.meta.url)));
  };
  for (const [key, value] of [["window", { location: { href: "http://127.0.0.1:54321/" } }], ["Worker", FakeWorker], ["fetch", fetchAsset]] as const) {
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
  assert.equal(FakeWorker.instances[0].packets.length, 0);
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
  assert.deepEqual(raw.packets[1].payload.options, {
    langPath: "http://127.0.0.1:54321/vendor/ocr/", gzip: false, cacheMethod: "none", lstmOnly: true,
  });
  const langs = raw.packets[1].payload.langs as { code: string; data: Uint8Array }[];
  assert.equal(langs[0].code, "eng");
  assert.equal(langs[0].data.byteLength, 4_113_088);
  assert.equal(raw.packets[2].payload.langs, "eng");
  assert.match(String(raw.packets[2].payload.config), /tessedit_load_sublangs\ntextord_tabfind_vertical_text 0/);
  const result = await worker.recognize(new ArrayBuffer(8));
  assert.equal(recognitionWords(result, page).length, 2);
  assert.deepEqual(raw.packets[4].payload.options, { rotateAuto: false, rotateRadians: 0 });
  worker.close();
  assert.equal(raw.terminated, true);
});

test("all explicit OCR resolutions reach the worker unchanged, including oversized-scan settings", async (context) => {
  fakeBrowser(context);
  for (const dpi of OCR_RESOLUTIONS) {
    const worker = new LocalOcrWorker(new AbortController().signal, () => {});
    const raw = FakeWorker.instances.at(-1)!;
    raw.automatic = true;
    await worker.initialize(dpi);
    const params = raw.packets.find((packet) => packet.action === "setParameters")?.payload.params;
    assert.ok(params && typeof params === "object");
    assert.equal(Reflect.get(params, "user_defined_dpi"), String(dpi));
    worker.close();
    assert.equal(raw.terminated, true);
  }
  const invalid = new LocalOcrWorker(new AbortController().signal, () => {});
  await assert.rejects(invalid.initialize(75), /Choose 72, 96, 150 or 300 DPI/);
  assert.equal(FakeWorker.instances.at(-1)!.packets.length, 0);
});

test("worker errors are visible, terminate resources and never include recognized content", async (context) => {
  fakeBrowser(context);
  const worker = new LocalOcrWorker(new AbortController().signal, () => {});
  const pending = worker.initialize(150);
  const raw = FakeWorker.instances[0];
  while (!raw.packets.length) await new Promise((resolve) => setTimeout(resolve, 0));
  raw.onmessage?.({ data: { jobId: raw.packets[0].jobId, status: "reject", data: "private recognized content" } });
  await assert.rejects(pending, (error: Error) => !error.message.includes("private recognized content"));
  assert.equal(raw.terminated, true);
});

test("model hash, missing and unknown-size failures never reach engine initialization", async (context) => {
  fakeBrowser(context);
  context.mock.method(globalThis, "fetch", async () => new Response(new Uint8Array([1, 2, 3, 4])));
  await assert.rejects(verifiedOcrAsset("eng.traineddata", 4, "0".repeat(64), new AbortController().signal), /SHA-256/);
  await assert.rejects(verifiedOcrAsset("eng.traineddata", 3, "0".repeat(64), new AbortController().signal), /exceeds/);
  const worker = new LocalOcrWorker(new AbortController().signal, () => {});
  await assert.rejects(worker.initialize(150), /wrong size/);
  assert.equal(FakeWorker.instances[0].packets.length, 0);
  assert.equal(FakeWorker.instances[0].terminated, true);
});

test("hard initialization deadline releases the worker and ignores late replies", async (context) => {
  fakeBrowser(context);
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const worker=new LocalOcrWorker(new AbortController().signal,()=>{});
  const initializing=worker.initialize(150);
  const rejected=assert.rejects(initializing,/two-minute limit/);
  await new Promise<void>((resolve)=>setImmediate(resolve));
  const raw=FakeWorker.instances[0];
  assert.equal(raw.packets.length,1);
  context.mock.timers.tick(120_000);
  await rejected;
  raw.reply(raw.packets[0],{loaded:true});
  await Promise.resolve();
  assert.equal(raw.terminated,true);
  assert.equal(raw.packets.length,1);
});

test("single-column scanning refuses faint and over-budget bands without a success-shaped empty result", async () => {
  const signal=new AbortController().signal;
  await assert.rejects(horizontalOcrBands(new Uint8ClampedArray(40*100*4).fill(230),40,100,300,signal),/faint/);
  const width=10,height=1500,pixels=new Uint8ClampedArray(width*height*4).fill(255);
  for(let y=0;y<height;y+=10)pixels.fill(0,y*width*4,y*width*4+3);
  await assert.rejects(horizontalOcrBands(pixels,width,height,150,signal),/128 horizontal/);
});
