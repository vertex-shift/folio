import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { degrees, PDFDocument, PDFName, PDFNumber, StandardFonts } from "pdf-lib";
import { build } from "vite";
import { englishModel, prepareOcr } from "../scripts/prepare-ocr.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const temporary = await mkdtemp(join(tmpdir(), "folio-ocr-"));
const artifacts = process.env.FOLIO_OCR_ARTIFACTS ? resolve(process.env.FOLIO_OCR_ARTIFACTS) : temporary;
await mkdir(artifacts, { recursive: true });
const children = [];
let browser;
const failures = [];
const evidence = { externalRequests: [], offline: [], pages: [] };
const env = { ...process.env, CARGO_INCREMENTAL: "0", CARGO_PROFILE_DEV_DEBUG: "0",
  CARGO_PROFILE_TEST_DEBUG: "0", CARGO_BUILD_JOBS: "2" };

function start(command, args, extra = {}) {
  const child = spawn(command, args, { cwd: root, env: { ...env, ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  child.output = "";
  for (const stream of [child.stdout, child.stderr]) stream.on("data", (data) => { child.output = (child.output + data).slice(-16000); });
  child.on("error", (error) => { child.startError = error; });
  children.push(child);
  return child;
}

async function command(command, args) {
  const child = start(command, args);
  await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code) => code === 0 ? resolve() : reject(new Error(`${command} failed (${code}): ${child.output}`)));
  });
}

async function freePort() {
  const socket = createServer();
  await new Promise((resolve, reject) => { socket.once("error", reject); socket.listen(0, "127.0.0.1", resolve); });
  const port = socket.address().port;
  await new Promise((resolve) => socket.close(resolve));
  return port;
}

async function ready(url, child) {
  const until = Date.now() + 120_000;
  while (Date.now() < until) {
    if (child.startError || child.exitCode !== null) throw new Error(`Test server did not start: ${child.startError ?? child.output}`);
    try { if ((await fetch(url, { signal: AbortSignal.timeout(2000) })).ok) return; } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Test server did not become ready: ${child.output}`);
}

const csp = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; connect-src 'self'; worker-src 'self' blob:; object-src 'none'; base-uri 'none'; form-action 'none'";
const workerAccounting = () => {
  const NativeWorker = window.Worker;
  window.__ocrWorkers = { active: 0, created: 0, terminated: 0 };
  window.Worker = class extends NativeWorker {
    constructor(url, options) {
      super(url, options);
      this.ocr = String(url).includes("/vendor/ocr/worker.min.js");
      this.stopped = false;
      if (this.ocr) { window.__ocrWorkers.active++; window.__ocrWorkers.created++; }
    }
    terminate() {
      if (this.ocr && !this.stopped) {
        window.__ocrWorkers.active--; window.__ocrWorkers.terminated++; this.stopped = true;
      }
      return super.terminate();
    }
  };
  window.__ocrCanvases = [];
  const create = document.createElement.bind(document);
  document.createElement = (...args) => {
    const element = create(...args);
    if (args[0] === "canvas" && new Error().stack?.includes("/src/lib/ocr.ts")) window.__ocrCanvases.push(element);
    return element;
  };
};

try {
  prepareOcr(root);
  const corruptRoot = join(temporary, "asset-corruption-check");
  await mkdir(join(corruptRoot, "src-tauri/resources"), { recursive: true });
  await cp(join(root, "src-tauri/resources/ocr"), join(corruptRoot, "src-tauri/resources/ocr"), { recursive: true });
  for (const [name, files] of [["tesseract.js", ["package.json", "LICENSE.md", "dist/worker.min.js"]],
    ["tesseract.js-core", ["package.json", "LICENSE"]]]) {
    for (const file of files) {
      const destination = join(corruptRoot, "node_modules", name, file);
      await mkdir(dirname(destination), { recursive: true });
      await cp(join(root, "node_modules", name, file), destination);
    }
  }
  await writeFile(join(corruptRoot, "node_modules/tesseract.js/dist/worker.min.js"), "altered worker");
  assert.throws(() => prepareOcr(corruptRoot), /worker\.min\.js failed SHA-256/);
  await writeFile(join(corruptRoot, "src-tauri/resources/ocr/eng.traineddata"), "altered model");
  assert.throws(() => prepareOcr(corruptRoot), /English OCR model failed SHA-256/);
  evidence.corruptedAssetsRejected = true;
  await command("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--quiet"]);
  const backendPort = await freePort(), frontendPort = await freePort();
  const ports = { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) };
  const backend = start(join(root, "src-tauri/target/debug/folio-preview") + (process.platform === "win32" ? ".exe" : ""), [], ports);
  await ready(`http://127.0.0.1:${backendPort}/health`, backend);
  const frontend = start(process.execPath, ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--strictPort"], ports);
  const origin = `http://127.0.0.1:${frontendPort}`;
  await ready(origin, frontend);
  const api = async (route, value, headers = {}) => {
    const response = await fetch(`http://127.0.0.1:${backendPort}${route}`, {
      headers: { "X-Folio-Preview": "1", ...(value ? { "Content-Type": "application/json" } : {}), ...headers },
      ...(value ? { method: "POST", body: JSON.stringify(value) } : {}),
    });
    assert.ok(response.ok, `Engine request ${route}: ${await response.clone().text()}`);
    return response;
  };
  const bytes = async (info) => new Uint8Array(await (await api("/bytes", null, {
    "X-Document-Id": String(info.id), "X-Revision": String(info.revision),
  })).arrayBuffer());

  browser = await chromium.launch({ headless: true,
    executablePath: process.env.FOLIO_OCR_BROWSER ?? chromium.executablePath() });
  evidence.browserVersion = browser.version();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce" });
  await context.addInitScript(workerAccounting);
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url());
    return route.abort();
  });
  const page = await context.newPage();
  page.on("pageerror", (error) => failures.push(error.message));
  const staged = [];
  let capturedPlan;
  page.on("request", (request) => {
    if (request.url().endsWith("/__folio/ocr/page")) staged.push(request.postDataJSON().result);
  });
  page.on("response", async (response) => {
    if (response.url().endsWith("/__folio/ocr/begin") && response.ok()) capturedPlan = await response.json();
  });
  await page.goto(origin);
  await page.getByRole("button", { name: "Recognize scanned text", exact: true }).waitFor();
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some((button) => button.textContent === "Recognize scanned text" && !button.disabled));
  const scans = await page.evaluate(async () => {
    const images = [];
    for (const rotation of [0, 90, 180, 270]) {
      const canvas = document.createElement("canvas");
      canvas.width = (rotation % 180 ? 200 : 340) * 4;
      canvas.height = (rotation % 180 ? 340 : 200) * 4;
      const context = canvas.getContext("2d");
      context.fillStyle = "white"; context.fillRect(0, 0, canvas.width, canvas.height);
      context.fillStyle = "black"; context.font = "bold 64px sans-serif";
      context.fillText("FOLIO OFFLINE", 64, 128);
      context.font = "48px sans-serif"; context.fillText("Searchable paper 2468", 64, 230);
      context.fillStyle = "#1971b8"; context.fillRect(64, canvas.height - 160, 80, 80);
      const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
      images.push(Array.from(new Uint8Array(await blob.arrayBuffer())));
      canvas.width = canvas.height = 0;
    }
    return images;
  });
  const pdf = await PDFDocument.create();
  pdf.setTitle("Original synthetic OCR fixture");
  pdf.setAuthor("Folio tests");
  for (const [index, rotation] of [0, 90, 180, 270].entries()) {
    const target = pdf.addPage([400, 240]);
    target.setMediaBox(-40, 10, 400, 240); target.setCropBox(-20, 30, 340, 200);
    target.setRotation(degrees(rotation));
    target.node.set(PDFName.of("UserUnit"), PDFNumber.of(index % 2 ? 1.5 : 2));
    const image = await pdf.embedPng(new Uint8Array(scans[index]));
    const matrix = rotation === 0 ? [340, 0, 0, 200, -20, 30]
      : rotation === 90 ? [0, 200, -340, 0, 320, 30]
      : rotation === 180 ? [-340, 0, 0, -200, 320, 230] : [0, -200, 340, 0, -20, 230];
    target.node.set(PDFName.of("Resources"), pdf.context.obj({ XObject: { Scan: image.ref } }));
    const stream = pdf.context.flateStream(`q ${matrix.join(" ")} cm /Scan Do Q\n0.5 0 0 0.5 11 9 cm\n`);
    target.node.set(PDFName.of("Contents"), pdf.context.register(stream));
  }
  pdf.addPage([340, 200]);
  const textPage = pdf.addPage([340, 200]);
  textPage.drawText("ALREADY SEARCHABLE", { x: 30, y: 110, size: 18, font: await pdf.embedFont(StandardFonts.Helvetica) });
  const source = await pdf.save({ useObjectStreams: false });
  const sourcePath = join(artifacts, "synthetic-scan.pdf");
  await writeFile(sourcePath, source);
  const fileInput = page.getByLabel("Choose PDF files");
  await fileInput.setInputFiles(sourcePath);
  await page.waitForFunction(() => [...document.querySelectorAll(".document-tab.selected")].some((tab) => tab.textContent.includes("synthetic-scan.pdf")));
  await page.getByRole("button", { name: "Recognize scanned text", exact: true }).click();
  await page.locator('.document-workspace:not([hidden])').getByLabel("Pages to recognize").fill("1-6");
  await page.locator('.document-workspace:not([hidden])').getByLabel("Scan resolution").selectOption("150");
  await page.getByRole("button", { name: "Recognize text", exact: true }).click();
  await page.getByRole("heading", { name: "Searchable text added" }).waitFor({ timeout: 180_000 });
  const summary = await page.locator(".ocr-outcome").innerText();
  assert.match(summary, /Skipped existing text on pages 6/);
  assert.match(summary, /No text recognized on pages 5/);
  assert.equal(staged.length, 5);
  assert.deepEqual(staged[4].words, []);
  for (let index = 0; index < 4; index++) assert.ok(staged[index].words.some((word) => word.text === "OFFLINE"), `Page ${index + 1} recognition`);
  assert.equal(await page.evaluate(() => window.__ocrWorkers.active), 0);
  assert.ok(await page.evaluate(() => window.__ocrCanvases.length >= 5 && window.__ocrCanvases.every((canvas) => canvas.width === 0 && canvas.height === 0)));
  assert.equal(await page.getByRole("tab").count(), 2);
  await page.locator('.document-workspace:not([hidden]) .textLayer').filter({ hasText: "OFFLINE" }).first().waitFor({ timeout: 30_000 });
  await page.screenshot({ path: join(artifacts, "ocr-light-desktop.png") });
  await page.evaluate(() => { document.documentElement.dataset.theme = "dark"; });
  await page.setViewportSize({ width: 950, height: 800 });
  await page.getByRole("button", { name: "All tools", exact: true }).click();
  await page.getByRole("button", { name: "Recognize scanned text", exact: true }).click();
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  await page.screenshot({ path: join(artifacts, "ocr-dark-compact.png") });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.evaluate(() => { document.documentElement.dataset.theme = "light"; });

  let snapshot = await (await api("/bootstrap")).json();
  const recognized = snapshot.documents.find((info) => info.name === "synthetic-scan.pdf");
  const recognizedBytes = await bytes(recognized);
  const downloadEvent = page.waitForEvent("download");
  await page.getByRole("button", { name: "Download PDF", exact: true }).click();
  const savedPath = join(artifacts, "synthetic-recognized.pdf");
  await (await downloadEvent).saveAs(savedPath);
  assert.deepEqual(new Uint8Array(await readFile(savedPath)), recognizedBytes);
  await fileInput.setInputFiles(savedPath);
  await page.waitForFunction(() => [...document.querySelectorAll(".document-tab.selected")].some((tab) => tab.textContent.includes("synthetic-recognized.pdf")));
  await page.locator('.document-workspace:not([hidden]) .textLayer').filter({ hasText: "OFFLINE" }).first().waitFor({ timeout: 30_000 });
  evidence.pages = await page.evaluate(async ({ original, saved, results, plan }) => {
    const { loadPdf } = await import("/src/lib/pdf.ts");
    const { TextLayer } = await import("/node_modules/pdfjs-dist/legacy/build/pdf.mjs");
    const { mapText, findMatches } = await import("/src/lib/search.ts");
    const before = await loadPdf(new Uint8Array(original)).promise;
    const after = await loadPdf(new Uint8Array(saved)).promise;
    const evidence = [];
    try {
      for (let index = 0; index < before.numPages; index++) {
        const sourcePage = await before.getPage(index + 1), outputPage = await after.getPage(index + 1);
        const viewport = outputPage.getViewport({ scale: 1 });
        const canvases = [];
        for (const page of [sourcePage, outputPage]) {
          const canvas = document.createElement("canvas");
          canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
          await page.render({ canvas, viewport }).promise;
          canvases.push(canvas);
        }
        const left = canvases[0].getContext("2d").getImageData(0, 0, canvases[0].width, canvases[0].height).data;
        const right = canvases[1].getContext("2d").getImageData(0, 0, canvases[1].width, canvases[1].height).data;
        let changed = 0;
        for (let offset = 0; offset < left.length; offset++) if (left[offset] !== right[offset]) changed++;
        for (const canvas of canvases) canvas.width = canvas.height = 0;
        const text = await outputPage.getTextContent();
        const row = { page: index + 1, rotation: outputPage.rotate, userUnit: outputPage.userUnit, changedChannels: changed,
          searchable: findMatches(mapText(text.items.filter((item) => "str" in item)).text, "FOLIO OFFLINE").length };
        if (index < 4) {
          const wrapper = document.createElement("div");
          wrapper.style.cssText = `position:fixed;left:0;top:0;width:${viewport.width}px;height:${viewport.height}px;`;
          const container = document.createElement("div");
          container.className = "textLayer";
          container.style.setProperty("--total-scale-factor", String(outputPage.userUnit));
          wrapper.append(container); document.body.append(wrapper);
          const layer = new TextLayer({ textContentSource: text, container, viewport });
          container.style.width = `${viewport.rotation % 180 ? viewport.height : viewport.width}px`;
          container.style.height = `${viewport.rotation % 180 ? viewport.width : viewport.height}px`;
          await layer.render();
          const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
          let node;
          while ((node = walker.nextNode()) && !node.textContent.includes("OFFLINE")) {}
          if (!node) throw new Error(`Reopened page ${index + 1} has no selectable OFFLINE text`);
          const start = node.textContent.indexOf("OFFLINE"), range = document.createRange();
          range.setStart(node, start); range.setEnd(node, start + 7);
          const selection = window.getSelection(); selection.removeAllRanges(); selection.addRange(range);
          row.selection = selection.toString();
          const box = range.getBoundingClientRect(), origin = wrapper.getBoundingClientRect();
          const expected = results[index].words.find((word) => word.text === "OFFLINE").bounds.map((value) => value * 72 / plan.dpi);
          row.alignment = { actual: [box.left - origin.left, box.top - origin.top, box.right - origin.left, box.bottom - origin.top], expected };
          const item = text.items.find((item) => "str" in item && item.str.includes("OFFLINE"));
          const baseline = viewport.convertToViewportPoint(item.transform[4], item.transform[5]);
          const expectedBaseline = results[index].words.find((word) => word.text === "OFFLINE").baseline;
          const direction = viewport.convertToViewportPoint(item.transform[4] + item.transform[0], item.transform[5] + item.transform[1]);
          const atWord = baseline[1] + (expectedBaseline[0] * 72 / plan.dpi - baseline[0]) * (direction[1] - baseline[1]) / (direction[0] - baseline[0]);
          row.baselineError = Math.abs(atWord - expectedBaseline[1] * 72 / plan.dpi);
          selection.removeAllRanges(); layer.cancel(); wrapper.remove();
        }
        evidence.push(row);
      }
    } finally { await before.loadingTask.destroy(); await after.loadingTask.destroy(); }
    return evidence;
  }, { original: Array.from(source), saved: Array.from(recognizedBytes), results: staged, plan: capturedPlan });
  await writeFile(join(artifacts, "ocr-alignment.json"), JSON.stringify(evidence.pages, null, 2) + "\n");
  for (const row of evidence.pages) {
    assert.equal(row.changedChannels, 0, `Page ${row.page} pixels changed`);
    if (row.page <= 4) {
      assert.ok(row.searchable > 0, `Reopened page ${row.page} searchable phrase`);
      assert.equal(row.selection, "OFFLINE");
      const { actual, expected } = row.alignment;
      assert.ok(row.baselineError < 0.05, `Page ${row.page} saved baseline alignment: ${row.baselineError}`);
      assert.ok(Math.abs(actual[0] - expected[0]) < 2 && Math.abs(actual[2] - expected[2]) < 3, `Page ${row.page} horizontal alignment: ${JSON.stringify(row.alignment)}`);
      // Browser text selections include font ascender/descender metrics, not just the
      // scan's tight ink box. Require strong overlap as well as exact saved baselines.
      const height = expected[3] - expected[1];
      const overlap = Math.max(0, Math.min(actual[3], expected[3]) - Math.max(actual[1], expected[1]));
      assert.ok(Math.abs(actual[1] - expected[1]) < height * 0.25 && overlap >= height * 0.75, `Page ${row.page} vertical selection alignment: ${JSON.stringify(row.alignment)}`);
      assert.ok(actual[3] >= expected[3] - 2 && actual[3] <= expected[3] + (expected[3] - expected[1]) * 0.5, `Page ${row.page} baseline/height alignment: ${JSON.stringify(row.alignment)}`);
    }
  }

  await page.getByRole("button", { name: "Find in document (Command+F)" }).click();
  await page.getByRole("textbox", { name: "Search PDF text" }).fill("FOLIO OFFLINE");
  await page.waitForFunction(() => document.querySelector(".document-workspace:not([hidden]) .search-summary")?.textContent.includes("4 matches"));
  await page.getByRole("button", { name: "All tools", exact: true }).click();
  await page.getByRole("button", { name: "Recognize scanned text", exact: true }).click();
  const activeWorkspace = page.locator('.document-workspace:not([hidden])');
  await activeWorkspace.getByLabel("Pages to recognize").fill("6");
  await activeWorkspace.getByLabel("Scan resolution").selectOption("150");
  await activeWorkspace.getByLabel("Pages with existing text").selectOption("force");
  snapshot = await (await api("/bootstrap")).json();
  const forceSource = snapshot.documents.find((info) => info.id === snapshot.activeId);
  const beforeForce = await bytes(forceSource);
  await page.getByRole("button", { name: "Recognize text", exact: true }).click();
  await page.getByRole("heading", { name: "Searchable text added" }).waitFor({ timeout: 180_000 });
  snapshot = await (await api("/bootstrap")).json();
  const forceInfo = snapshot.documents.find((info) => info.id === snapshot.activeId);
  const afterForce = await bytes(forceInfo);
  evidence.force = await page.evaluate(async ({ beforeBytes, afterBytes }) => {
    const { loadPdf } = await import("/src/lib/pdf.ts");
    const before = await loadPdf(new Uint8Array(beforeBytes)).promise;
    const after = await loadPdf(new Uint8Array(afterBytes)).promise;
    try {
      const canvases = [];
      for (const pdf of [before, after]) {
        const page = await pdf.getPage(6), viewport = page.getViewport({ scale: 1 });
        const canvas = document.createElement("canvas");
        canvas.width = viewport.width; canvas.height = viewport.height;
        await page.render({ canvas, viewport }).promise;
        canvases.push(canvas);
      }
      const pixels = canvases.map((canvas) => canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data);
      let changes = 0;
      for (let index = 0; index < pixels[0].length; index++) if (pixels[0][index] !== pixels[1][index]) changes++;
      for (const canvas of canvases) canvas.width = canvas.height = 0;
      const text = (await (await after.getPage(6)).getTextContent()).items.filter((item) => "str" in item).map((item) => item.str).join(" ");
      return { changedChannels: changes, phraseCount: (text.match(/ALREADY\s+SEARCHABLE/g) ?? []).length };
    } finally { await before.loadingTask.destroy(); await after.loadingTask.destroy(); }
  }, { beforeBytes: Array.from(beforeForce), afterBytes: Array.from(afterForce) });
  assert.equal(evidence.force.changedChannels, 0, "Force OCR changed original visible source text");
  assert.equal(evidence.force.phraseCount, 2, "Force policy must preserve the original, not replace text silently");
  const undone = page.waitForResponse((response) => response.url().endsWith("/__folio/undo"));
  await page.getByRole("button", { name: "Undo (Command+Z)", exact: true }).click();
  assert.deepEqual(await bytes(await (await undone).json()), beforeForce);
  const redone = page.waitForResponse((response) => response.url().endsWith("/__folio/redo"));
  await page.getByRole("button", { name: "Redo (Command+Shift+Z)", exact: true }).click();
  assert.deepEqual(await bytes(await (await redone).json()), afterForce);
  await activeWorkspace.getByLabel("Pages to recognize").fill("1-4");
  snapshot = await (await api("/bootstrap")).json();
  const cancelSource = snapshot.documents.find((info) => info.id === snapshot.activeId);
  const cancelBytes = await bytes(cancelSource);
  await page.getByRole("button", { name: "Recognize text", exact: true }).click();
  await page.waitForFunction(() => window.__ocrWorkers.active === 1);
  await page.getByRole("button", { name: "Cancel OCR", exact: true }).click();
  await page.getByText("OCR canceled. The working PDF is unchanged.", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.__ocrWorkers.active), 0);
  assert.deepEqual(await bytes(cancelSource), cancelBytes);
  evidence.workerLifecycle = await page.evaluate(() => window.__ocrWorkers);
  await context.close();

  await build({ root, logLevel: "error" });
  const assetManifest = JSON.parse(await readFile(join(root, "dist/vendor/ocr/manifest.json"), "utf8"));
  assert.equal(assetManifest.model.sha256, englishModel.sha256);
  for (const asset of assetManifest.files) {
    const bundled = await readFile(join(root, "dist/vendor/ocr", asset.name));
    assert.equal(createHash("sha256").update(bundled).digest("hex"), asset.sha256, `Built OCR asset ${asset.name}`);
  }
  evidence.builtAssetsVerified = assetManifest.files.length;
  await build({ root, configFile: false, publicDir: false, logLevel: "error", build: {
    target: "safari16", outDir: join(temporary, "offline"), emptyOutDir: false,
    lib: { entry: join(root, "src/lib/ocrWorker.ts"), formats: ["es"], fileName: () => "ocr-harness.js" },
  } });
  for (const nonSimd of [false, true]) {
    const offline = await browser.newContext({ offline: true });
    await offline.addInitScript(workerAccounting);
    const requested = [];
    await offline.route("**/*", async (route) => {
      const url = new URL(route.request().url());
      assert.equal(url.origin, "http://folio-ocr.invalid", `Unexpected offline OCR network request ${url.href}`);
      requested.push(url.pathname);
      let body, contentType;
      if (url.pathname === "/") {
        body = "<!doctype html><html><head><title>Offline OCR verification</title></head><body>Offline OCR</body></html>";
        contentType = "text/html";
      } else if (url.pathname === "/ocr-harness.js") {
        body = await readFile(join(temporary, "offline/ocr-harness.js")); contentType = "application/javascript";
      } else {
        assert.ok(url.pathname.startsWith("/vendor/ocr/") && !url.pathname.includes(".."));
        body = await readFile(join(root, "dist", url.pathname));
        if (nonSimd && url.pathname.endsWith("/worker.min.js")) body = Buffer.concat([Buffer.from("WebAssembly.validate = () => false;\n"), body]);
        contentType = extname(url.pathname) === ".js" ? "application/javascript" : extname(url.pathname) === ".wasm" ? "application/wasm" : "application/octet-stream";
      }
      await route.fulfill({ status: 200, contentType, headers: { "Content-Security-Policy": csp }, body });
    });
    const tab = await offline.newPage();
    tab.on("pageerror", (error) => failures.push(error.message));
    await tab.goto("http://folio-ocr.invalid/");
    const result = await tab.evaluate(async (png) => {
      const { LocalOcrWorker } = await import("/ocr-harness.js");
      const worker = new LocalOcrWorker(new AbortController().signal, () => {});
      try {
        await worker.initialize(150);
        const output = await worker.recognize(new Uint8Array(png).buffer);
        return { online: navigator.onLine, recognized: output.text.includes("FOLIO OFFLINE") };
      } finally { worker.close(); }
    }, scans[0]);
    assert.equal(result.online, false);
    assert.equal(result.recognized, true);
    assert.equal(await tab.evaluate(() => window.__ocrWorkers.active), 0);
    assert.ok(requested.includes("/vendor/ocr/eng.traineddata"));
    assert.ok(requested.some((path) => path.includes(nonSimd ? "tesseract-core-lstm.wasm.js" : "tesseract-core-simd-lstm.wasm.js")));
    evidence.offline.push({ nonSimd, networkOffline: true, recognizedKnownPhrase: true, requested });
    await offline.close();
  }
  assert.deepEqual(evidence.externalRequests, []);
  assert.deepEqual(failures, []);
  evidence.sourceSha256 = createHash("sha256").update(source).digest("hex");
  evidence.outputSha256 = createHash("sha256").update(recognizedBytes).digest("hex");
  await writeFile(join(artifacts, "ocr-evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
  console.log("Offline OCR passed: real English recognition; unchanged pixels on 6 reopened pages; search/selection alignment at 0/90/180/270 with crop offsets and UserUnit; skip/no-text; cancel/cleanup; production worker with native CSP and networking disabled (SIMD and non-SIMD).");
  if (process.env.FOLIO_OCR_ARTIFACTS) console.log(`Synthetic fixtures and evidence: ${artifacts}`);
} finally {
  await browser?.close();
  for (const child of children.reverse()) {
    if (child.exitCode === null && child.pid) {
      child.kill("SIGTERM");
      await Promise.race([new Promise((resolve) => child.once("exit", resolve)), new Promise((resolve) => setTimeout(resolve, 3000))]);
      if (child.exitCode === null) child.kill("SIGKILL");
    }
  }
  await rm(temporary, { recursive: true, force: true });
}
