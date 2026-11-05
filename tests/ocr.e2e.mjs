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
import { preparePdfWorker } from "../scripts/prepare-pdf-worker.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const temporary = await mkdtemp(join(tmpdir(), "folio-ocr-"));
const artifacts = process.env.FOLIO_OCR_ARTIFACTS ? resolve(process.env.FOLIO_OCR_ARTIFACTS) : temporary;
await mkdir(artifacts, { recursive: true });
const children = [];
let browser;
const failures = [];
const evidence = { externalRequests: [], offline: [], pages: [] };
const languageCases = [
  { language: "eng", font: "NotoSans-Regular.ttf", direction: "ltr", lines: ["Local documents stay on this computer.", "Offline reading and review 48271."] },
  { language: "fra", font: "NotoSans-Regular.ttf", direction: "ltr", lines: ["Les documents restent sur cet ordinateur.", "Lecture locale et révision 48271."] },
  { language: "spa", font: "NotoSans-Regular.ttf", direction: "ltr", lines: ["Los documentos quedan en este equipo.", "Lectura local y revisión 48271."] },
  { language: "deu", font: "NotoSans-Regular.ttf", direction: "ltr", lines: ["Dokumente bleiben auf diesem Computer.", "Lokales Lesen und Prüfung 48271."] },
  { language: "ita", font: "NotoSans-Regular.ttf", direction: "ltr", lines: ["I documenti restano su questo computer.", "Lettura locale e revisione 48271."] },
  { language: "por", font: "NotoSans-Regular.ttf", direction: "ltr", lines: ["Os documentos ficam neste computador.", "Leitura local e revisão 48271."] },
  { language: "nld", font: "NotoSans-Regular.ttf", direction: "ltr", lines: ["Documenten blijven op deze computer.", "Lokaal lezen en controleren 48271."] },
  { language: "ell", font: "NotoSans-Regular.ttf", direction: "ltr", lines: ["Τα έγγραφα μένουν σε αυτόν τον υπολογιστή.", "Τοπική ανάγνωση και έλεγχος 48271."] },
  { language: "rus", font: "NotoSans-Regular.ttf", direction: "ltr", lines: ["Документы остаются на этом компьютере.", "Местное чтение и проверка 48271."] },
  { language: "ara", font: "FolioOcrArabic-Regular.ttf", direction: "rtl", lines: ["تبقى المستندات على هذا الكمبيوتر.", "قراءة محلية ومراجعة 48271."] },
  { language: "hin", font: "FolioOcrDevanagari-Regular.ttf", direction: "ltr", lines: ["यह दस्तावेज़ कंप्यूटर पर रहता है।", "स्थानीय पढ़ना और समीक्षा 48271।"] },
  { language: "chi_sim", font: "FolioOcrCjkSC-Regular.ttf", direction: "ltr", lines: ["文件保留在本机。", "离线阅读和检查文件 48271。"] },
  { language: "chi_tra", font: "FolioOcrCjkSC-Regular.ttf", direction: "ltr", lines: ["文件保留在本機。", "離線閱讀和檢查文件 48271。"] },
  { language: "jpn", font: "FolioOcrCjkJP-Regular.ttf", direction: "ltr", lines: ["文書はこのコンピューターに保存されます。", "文書を読んで確認します 48271。"] },
  { language: "kor", font: "FolioOcrCjkKR-Regular.ttf", direction: "ltr", lines: ["문서는 이 컴퓨터에 보관됩니다.", "문서를 읽고 확인합니다 48271."] },
  { language: "mixed", font: "NotoSans-Regular.ttf", direction: "mixed", languages: ["eng","ara","hin"],
    rowFonts: ["NotoSans-Regular.ttf","FolioOcrArabic-Regular.ttf","FolioOcrDevanagari-Regular.ttf"],
    rowDirections: ["ltr","rtl","ltr"],
    lines: ["Local review stays offline.", "هذا كتاب جديد في بيت جميل", "यह किताब एक सुंदर कहानी है"] },
];
const selectedLanguageCases = process.env.FOLIO_OCR_LANGUAGES
  ? languageCases.filter((item) => process.env.FOLIO_OCR_LANGUAGES.split(",").includes(item.language))
  : languageCases;
function accuracy(expected, recognized) {
  const a = Array.from(expected.normalize("NFC").replace(/\s/gu, ""));
  const b = Array.from(recognized.normalize("NFC").replace(/\s/gu, ""));
  const row = Array.from({ length: b.length + 1 }, (_, index) => index);
  for (let i = 1; i <= a.length; i++) {
    let previous = row[0]; row[0] = i;
    for (let j = 1; j <= b.length; j++) {
      const current = row[j];
      row[j] = Math.min(row[j] + 1, row[j - 1] + 1, previous + (a[i - 1] === b[j - 1] ? 0 : 1));
      previous = current;
    }
  }
  return 1 - row[b.length] / Math.max(a.length, b.length, 1);
}
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
  preparePdfWorker(root);
  const corruptRoot = join(temporary, "asset-corruption-check");
  await mkdir(join(corruptRoot, "src-tauri/resources"), { recursive: true });
  await cp(join(root, "src-tauri/resources/ocr"), join(corruptRoot, "src-tauri/resources/ocr"), { recursive: true });
  await cp(join(root, "src-tauri/resources/fonts"), join(corruptRoot, "src-tauri/resources/fonts"), { recursive: true });
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
  assert.throws(() => prepareOcr(corruptRoot), /eng\.traineddata failed SHA-256/);
  await cp(join(root,"src-tauri/resources/ocr/eng.traineddata"),join(corruptRoot,"src-tauri/resources/ocr/eng.traineddata"));
  await writeFile(join(corruptRoot,"src-tauri/resources/ocr/fonts/FolioOcrArabic-Regular.ttf"),"altered font");
  assert.throws(()=>prepareOcr(corruptRoot),/FolioOcrArabic-Regular\.ttf failed SHA-256/);
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
  let ocrBeginRequests = 0;
  const offlineFixtures=[];
  let capturedPlan;
  page.on("request", (request) => {
    if (request.url().endsWith("/__folio/ocr/page")) staged.push(request.postDataJSON().result);
    if (request.url().endsWith("/__folio/ocr/begin")) ocrBeginRequests++;
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
  if (!process.env.FOLIO_OCR_SKIP_VISUAL) {
    await page.screenshot({ path: join(artifacts, "ocr-light-desktop.png") });
    await page.evaluate(() => { document.documentElement.dataset.theme = "dark"; });
    await page.setViewportSize({ width: 950, height: 800 });
    await page.getByRole("button", { name: "All tools", exact: true }).click();
    await page.getByRole("button", { name: "Recognize scanned text", exact: true }).click();
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.screenshot({ path: join(artifacts, "ocr-dark-compact.png") });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.evaluate(() => { document.documentElement.dataset.theme = "light"; });
  }

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
    const { alignOcrTextLayer } = await import("/src/lib/ocrTextLayer.ts");
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
  const oversizedImage = await page.evaluate(async () => {
    const canvas = document.createElement("canvas");
    canvas.width = 1200; canvas.height = 1800;
    const context = canvas.getContext("2d");
    context.fillStyle = "white"; context.fillRect(0, 0, canvas.width, canvas.height);
    context.fillStyle = "black"; context.font = "bold 52px sans-serif";
    context.fillText("FOLIO OVERSIZED SCAN", 70, 180);
    context.font = "36px sans-serif";
    context.fillText("Offline paper stays on this computer.", 70, 290);
    context.fillText("Searchable document 58247.", 70, 390);
    const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
    canvas.width = canvas.height = 0;
    return Array.from(new Uint8Array(await blob.arrayBuffer()));
  });
  const oversized = await PDFDocument.create();
  const oversizedPage = oversized.addPage([2400, 3600]);
  oversizedPage.drawImage(await oversized.embedPng(new Uint8Array(oversizedImage)),
    { x: 0, y: 0, width: 2400, height: 3600 });
  const oversizedSource = await oversized.save({ useObjectStreams: false });
  evidence.oversized = [];
  for (const dpi of [96, 72]) {
    const name = `oversized-${dpi}-source.pdf`;
    await fileInput.setInputFiles({ name, mimeType: "application/pdf", buffer: Buffer.from(oversizedSource) });
    await page.waitForFunction((name) => document.querySelector(".document-tab.selected")?.textContent.includes(name), name);
    await page.getByRole("button", { name: "Recognize scanned text", exact: true }).click();
    const active = page.locator(".document-workspace:not([hidden])");
    await active.getByRole("status").filter({ hasText: "Choose 96 DPI" }).waitFor();
    assert.equal(await active.getByRole("button", { name: "Recognize text", exact: true }).isDisabled(), true);
    const previousBegins = ocrBeginRequests;
    await active.getByLabel("Pages to recognize").press("Enter");
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    assert.equal(ocrBeginRequests, previousBegins, "Keyboard submission must respect the visible resolution warning.");
    await active.getByLabel("Scan resolution").selectOption("150");
    await active.getByRole("status").filter({ hasText: "Choose 96 DPI" }).waitFor();
    assert.equal(await active.getByRole("button", { name: "Recognize text", exact: true }).isDisabled(), true);
    await active.getByLabel("Scan resolution").selectOption(String(dpi));
    const first = staged.length;
    await active.getByRole("button", { name: "Recognize text", exact: true }).click();
    await active.getByRole("heading", { name: "Searchable text added", exact: true }).waitFor({ timeout: 180_000 });
    assert.equal(staged.length, first + 1);
    const result = staged[first];
    assert.ok(result.words.some((word) => word.text === "OVERSIZED"));
    assert.equal(capturedPlan.dpi, dpi);
    assert.deepEqual([capturedPlan.pages[0].pixelWidth, capturedPlan.pages[0].pixelHeight],
      dpi === 96 ? [3200, 4800] : [2400, 3600]);
    const download = page.waitForEvent("download");
    await page.getByRole("button", { name: "Download PDF", exact: true }).click();
    const savedPath = join(artifacts, `oversized-${dpi}-recognized.pdf`);
    await (await download).saveAs(savedPath);
    const saved = new Uint8Array(await readFile(savedPath));
    await fileInput.setInputFiles(savedPath);
    await page.waitForFunction((name) => document.querySelector(".document-tab.selected")?.textContent.includes(name),
      `oversized-${dpi}-recognized.pdf`);
    await page.locator(".document-workspace:not([hidden]) .textLayer").filter({ hasText: "OVERSIZED" }).first().waitFor();
    const inspection = await page.evaluate(async ({ original, saved, words, dpi }) => {
      const { loadPdf } = await import("/src/lib/pdf.ts");
      const tasks = [loadPdf(new Uint8Array(original)), loadPdf(new Uint8Array(saved))];
      const documents = await Promise.all(tasks.map((task) => task.promise));
      try {
        const pages = await Promise.all(documents.map((pdf) => pdf.getPage(1)));
        const viewport = pages[1].getViewport({ scale: 1 });
        const canvases = [];
        for (const page of pages) {
          const canvas = document.createElement("canvas");
          canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
          await page.render({ canvas, viewport, background: "#ffffff" }).promise;
          canvases.push(canvas);
        }
        const pixels = canvases.map((canvas) => canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data);
        let changedChannels = 0;
        for (let index = 0; index < pixels[0].length; index++) if (pixels[0][index] !== pixels[1][index]) changedChannels++;
        for (const canvas of canvases) canvas.width = canvas.height = 0;
        const content = await pages[1].getTextContent();
        const items = content.items.filter((item) => "str" in item && item.str.trim());
        if (items.length !== words.length || items.some((item, index) => item.str !== words[index].text)) {
          throw new Error("Lower-resolution saved OCR did not preserve each actual recognized word.");
        }
        const baselineErrors = items.map((item, index) => {
          const actual = viewport.convertToViewportPoint(item.transform[4], item.transform[5]);
          const expected = words[index].baseline.map((value) => value * 72 / dpi);
          return Math.max(Math.abs(actual[0] - expected[0]), Math.abs(actual[1] - expected[1]));
        });
        return { changedChannels, words: items.length, view: pages[1].view,
          maxBaselineError: Math.max(...baselineErrors) };
      } finally { await Promise.all(tasks.map((task) => task.destroy())); }
    }, { original: Array.from(oversizedSource), saved: Array.from(saved), words: result.words, dpi });
    assert.equal(inspection.changedChannels, 0);
    assert.deepEqual(inspection.view, [0, 0, 2400, 3600]);
    assert.ok(inspection.maxBaselineError < 0.05);
    evidence.oversized.push({ dpi, ...inspection });
  }
  if (process.env.FOLIO_OCR_MULTILINGUAL || process.argv.includes("--multilingual")) {
    const fixtures = await page.evaluate(async (cases) => {
      const loaded = new Set();
      const images = [];
      for (const item of cases) {
        for (const filename of item.rowFonts??[item.font]) if (!loaded.has(filename)) {
          const font = new FontFace(filename.replaceAll(".", "_"), `url(/vendor/ocr/fonts/${filename})`);
          document.fonts.add(await font.load()); loaded.add(filename);
        }
        const canvas = document.createElement("canvas"); canvas.width = 1500; canvas.height = item.language==="mixed"?950:750;
        const ctx = canvas.getContext("2d");
        ctx.fillStyle = "white"; ctx.fillRect(0, 0, canvas.width,canvas.height);
        ctx.fillStyle = "black";
        for (const [index, line] of item.lines.entries()) {
          const filename=item.rowFonts?.[index]??item.font,direction=item.rowDirections?.[index]??item.direction;
          ctx.direction=direction;ctx.textAlign=direction==="rtl"?"right":"left";
          ctx.font=`72px "${filename.replaceAll(".","_")}"`;
          const size=72*Math.min(1,1350/ctx.measureText(line).width);
          ctx.font=`${size}px "${filename.replaceAll(".","_")}"`;
          ctx.fillText(line,direction==="rtl"?1425:75,220+index*200);
        }
        const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
        images.push(Array.from(new Uint8Array(await blob.arrayBuffer())));
        canvas.width = canvas.height = 0;
      }
      return images;
    }, selectedLanguageCases);
    evidence.languages = [];
    for (const [index, item] of selectedLanguageCases.entries()) {
      const doc = await PDFDocument.create();
      const height=item.language==="mixed"?228:180;
      const target = doc.addPage([360,height]), image = await doc.embedPng(new Uint8Array(fixtures[index]));
      target.drawImage(image, { x: 0, y: 0, width: 360, height });
      const password=process.env.FOLIO_OCR_PROTECTED_ENGINE && item.language==="ara"?"reader":null;
      let original = process.env.FOLIO_OCR_QUAD_FIXTURE && item.language==="ara"
        ? new Uint8Array(await readFile(process.env.FOLIO_OCR_QUAD_FIXTURE))
        : process.env.FOLIO_OCR_CORPUS && item.language!=="mixed"
        ? new Uint8Array(await readFile(join(process.env.FOLIO_OCR_CORPUS, `${item.language}.pdf`)))
        : await doc.save({ useObjectStreams: false });
      if(process.env.FOLIO_OCR_PROTECTED_ENGINE && item.language==="ara") {
        const response=await fetch(`http://127.0.0.1:${backendPort}/open`,{method:"POST",
          headers:{"X-Folio-Preview":"1","Content-Type":"application/pdf","X-File-Name":"public-protected-source-fixture.pdf"},body:original});
        const outcome=await response.json();
        if(outcome.status!=="opened")throw new Error("The original synthetic protection fixture could not open.");
        const protectedCopy=await api("/protection/export",{documentId:outcome.document.id,revision:outcome.document.revision,authorizationPassword:null,
          protection:{mode:"protect",openingPassword:"reader",ownerPassword:"owner",permissions:{print:"high",copy:true,edit:true,annotate:true,fillForms:true,assemble:true}}});
        original=new Uint8Array(await protectedCopy.arrayBuffer());
      }
      const pageCount=password?1:(await PDFDocument.load(original)).getPageCount();
      const sourcePath = join(artifacts, `${item.language}-scan.pdf`);
      await writeFile(sourcePath, original);
      await writeFile(join(artifacts, `${item.language}-scan.png`), process.env.FOLIO_OCR_CORPUS && item.language!=="mixed"
        ? await readFile(join(process.env.FOLIO_OCR_CORPUS, `${item.language}.png`)) : new Uint8Array(fixtures[index]));
      offlineFixtures.push({language:item.language,languages:item.languages??[item.language],lines:item.lines,
        png:Array.from(await readFile(join(artifacts,`${item.language}-scan.png`)))});
      if(password) {
        const checked=await fetch(`http://127.0.0.1:${backendPort}/open`,{method:"POST",
          headers:{"X-Folio-Preview":"1","Content-Type":"application/pdf","X-File-Name":"protected-fixture-check.pdf"},body:original});
        const outcome=await checked.json();
        if(outcome.status!=="password_required")throw new Error(`Protected fixture did not produce a password challenge: ${JSON.stringify(outcome)}`);
        await api("/open/cancel",{requestId:outcome.requestId});
      }
      await fileInput.setInputFiles(sourcePath);
      if(password) {
        await page.getByLabel("PDF password",{exact:true}).fill(password);
        await page.getByRole("button",{name:"Unlock PDF",exact:true}).click();
      }
      await page.waitForFunction((name) => document.querySelector(".document-tab.selected")?.textContent.includes(name), `${item.language}-scan.pdf`);
      await page.getByRole("button", { name: "Recognize scanned text", exact: true }).click();
      const active = page.locator('.document-workspace:not([hidden])');
      await active.getByLabel("Pages to recognize").fill(pageCount===1?"1":`1-${pageCount}`);
      await active.getByLabel("Scan resolution").selectOption("300");
      await active.getByLabel("OCR language", { exact: true }).selectOption(item.languages?.[0]??item.language);
      await active.getByLabel("Additional language 1", { exact: true }).selectOption(item.languages?.[1]??"");
      await active.getByLabel("Additional language 2", { exact: true }).selectOption(item.languages?.[2]??"");
      await active.getByLabel("OCR text layout", { exact: true }).selectOption("single_column");
      if(item.language==="mixed") {
        await page.locator(".document-tab:not(.selected)").first().click();
        await page.locator(".document-tab").filter({hasText:"mixed-scan.pdf"}).click();
        assert.equal(await active.getByLabel("OCR language",{exact:true}).inputValue(),"eng");
        assert.equal(await active.getByLabel("Additional language 1",{exact:true}).inputValue(),"ara");
        assert.equal(await active.getByLabel("Additional language 2",{exact:true}).inputValue(),"hin");
        assert.equal(await active.getByLabel("OCR text layout",{exact:true}).inputValue(),"single_column");
      }
      const resultStart = staged.length;
      await page.getByRole("button", { name: "Recognize text", exact: true }).click();
      await page.waitForFunction(() => document.querySelector(".document-workspace:not([hidden]) .ocr-outcome h3")?.textContent === "Searchable text added"
        || Boolean(document.querySelector(".document-workspace:not([hidden]) .error-notification")), { timeout: 120_000 });
      const error = await active.locator(".error-notification").count();
      if (error) throw new Error(`${item.language}: ${await active.locator(".error-notification").innerText()}`);
      assert.equal(staged.length, resultStart + pageCount, `${item.language}: exact staged pages`);
      const recognizedPages=staged.slice(resultStart).map((result)=>result.words.map((word)=>word.text+word.separator).join(""));
      for(const recognized of recognizedPages) assert.ok(accuracy(item.lines.join("\n"), recognized) >= 0.9, `${item.language}: recognition below 90% ${JSON.stringify(recognized)}`);
      const download = page.waitForEvent("download");
      await page.getByRole("button", { name: "Download PDF", exact: true }).click();
      const savedPath = join(artifacts, `${item.language}-recognized.pdf`);
      await (await download).saveAs(savedPath);
      const saved = new Uint8Array(await readFile(savedPath));
      if(password) {
        const current=await(await api("/bootstrap")).json(),info=current.documents.find((info)=>info.id===current.activeId);
        assert.equal(info.passwordProtected,true);
        assert.deepEqual(saved,await bytes(info),"Save must use protected Session bytes, not renderer bytes");
        const render=await(await api("/render-bytes",null,{"X-Document-Id":String(info.id),"X-Revision":String(info.revision)})).arrayBuffer();
        assert.notDeepEqual(saved,new Uint8Array(render),"Protected save cannot interchange renderer data");
      }
      await fileInput.setInputFiles(savedPath);
      if(password) {
        await page.getByLabel("PDF password",{exact:true}).fill(password);
        await page.getByRole("button",{name:"Unlock PDF",exact:true}).click();
      }
      await page.waitForFunction((name) => document.querySelector(".document-tab.selected")?.textContent.includes(name), `${item.language}-recognized.pdf`);
      const inspections=[];
      for(let sourcePage=0;sourcePage<pageCount;sourcePage++) {
      const inspection = await page.evaluate(async ({ original, saved, words, plan,sourcePage,password }) => {
        const { loadPdf } = await import("/src/lib/pdf.ts");
        const { TextLayer } = await import("/node_modules/pdfjs-dist/legacy/build/pdf.mjs");
        const { mapText, findMatches } = await import("/src/lib/search.ts");
        const { alignOcrTextLayer } = await import("/src/lib/ocrTextLayer.ts");
        const beforeTask=loadPdf(new Uint8Array(original)),afterTask=loadPdf(new Uint8Array(saved));
        let originalPassword=false,savedPassword=false;
        if(password) {
          beforeTask.onPassword=(update)=>{originalPassword=true;update(password);};
          afterTask.onPassword=(update)=>{savedPassword=true;update(password);};
        }
        const before=await beforeTask.promise,after=await afterTask.promise;
        try {
          const originalPage = await before.getPage(sourcePage+1), outputPage = await after.getPage(sourcePage+1);
          const text = await outputPage.getTextContent();
          const items = text.items.filter((item) => "str" in item);
          const logical = mapText(items).text;
          const viewport = outputPage.getViewport({ scale: 1 });
          const canvases = [];
          for (const pdfPage of [originalPage, outputPage]) {
            const canvas = document.createElement("canvas"); canvas.width = viewport.width; canvas.height = viewport.height;
            await pdfPage.render({ canvas, viewport }).promise; canvases.push(canvas);
          }
          const pixels = canvases.map((canvas) => canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data);
          let changes = 0;
          for (let offset = 0; offset < pixels[0].length; offset++) if (pixels[0][offset] !== pixels[1][offset]) changes++;
          for (const canvas of canvases) canvas.width = canvas.height = 0;
          const wrapper=document.createElement("div");
          wrapper.style.cssText=`position:fixed;left:0;top:0;width:${viewport.width}px;height:${viewport.height}px;`;
          const container = document.createElement("div"); container.className = "textLayer";
          container.style.setProperty("--total-scale-factor", String(outputPage.userUnit));wrapper.append(container); document.body.append(wrapper);
          const layer = new TextLayer({ textContentSource: text, container, viewport }); await layer.render();
          container.style.width=`${viewport.rotation%180?viewport.height:viewport.width}px`;
          container.style.height=`${viewport.rotation%180?viewport.width:viewport.height}px`;
          alignOcrTextLayer(text,layer,viewport);
          const positions = [];
          const candidates=items.map((item,index)=>({item,span:layer.textDivs[index]})).filter(({item})=>/\S/u.test(item.str));
          if(candidates.length!==words.length)throw new Error("OCR word count changed during saved extraction.");
          for (const [index,word] of words.entries()) {
            const {item,span}=candidates[index];
            if (!item || !span) throw new Error(`Logical word missing after saved reopen: ${word.text}`);
            if(item.str!==word.text)throw new Error(`OCR word order changed after saved reopen: ${word.text}`);
            const range = document.createRange(); range.selectNodeContents(span);
            const selection = window.getSelection(); selection.removeAllRanges(); selection.addRange(range);
            const box = range.getBoundingClientRect(), origin = wrapper.getBoundingClientRect();
            const baseline = viewport.convertToViewportPoint(item.transform[4], item.transform[5]);
            const expected = word.bounds.map((value) => value * 72 / plan.dpi);
            positions.push({ text: word.text, direction: item.dir, selection: selection.toString(),
              expectedDirection:word.direction,
              baselineError: Math.abs(baseline[1] - word.baseline[1] * 72 / plan.dpi),
              actual: [box.left - origin.left, box.top - origin.top, box.right - origin.left, box.bottom - origin.top], expected });
            selection.removeAllRanges();
          }
          layer.cancel(); wrapper.remove();
          return { page:sourcePage+1,rotation:outputPage.rotate,userUnit:outputPage.userUnit,pixelWidth:plan.pages[sourcePage].pixelWidth,
            pixelHeight:plan.pages[sourcePage].pixelHeight,originalPassword,savedPassword,logical, changedChannels: changes, positions, queryMatches: findMatches(logical, words[0].text).length };
        } finally { await before.loadingTask.destroy(); await after.loadingTask.destroy(); }
      }, { original: Array.from(original), saved: Array.from(saved), words: staged[resultStart+sourcePage].words, plan: capturedPlan,sourcePage,password });
      assert.equal(inspection.logical, recognizedPages[sourcePage], `${item.language}: exact recognized Unicode after saved reopen`);
      assert.equal(inspection.changedChannels, 0, `${item.language}: source pixels`);
      assert.ok(inspection.queryMatches > 0, `${item.language}: logical search`);
      if(password){assert.equal(inspection.originalPassword,true);assert.equal(inspection.savedPassword,true);}
      for (const position of inspection.positions) {
        assert.equal(position.selection, position.text, `${item.language}: exact Unicode selection`);
        assert.equal(position.direction, item.direction==="mixed"?position.expectedDirection:item.direction, `${item.language}: actual direction`);
        assert.ok(position.baselineError < 0.05, `${item.language}: baseline`);
        assert.ok(Math.abs(position.actual[0] - position.expected[0]) < 2 && Math.abs(position.actual[2] - position.expected[2]) < 3,
          `${item.language}: actual word width ${JSON.stringify(position)}`);
      }
      inspections.push(inspection);
      }
      assert.equal(await page.evaluate(() => window.__ocrWorkers.active), 0);
      evidence.languages.push({ ...item, recognized:recognizedPages.join("\f"), accuracy:Math.min(...recognizedPages.map((text)=>accuracy(item.lines.join("\n"),text))),
        inspections,
        sourceSha256: createHash("sha256").update(original).digest("hex"), outputSha256: createHash("sha256").update(saved).digest("hex") });
      await writeFile(join(artifacts, "multilingual-evidence.json"), JSON.stringify(evidence.languages, null, 2) + "\n");
      console.log(`${item.language}: actual UI OCR/save/reopen, ${(evidence.languages.at(-1).accuracy * 100).toFixed(1)}% fixture character accuracy; exact Unicode/direction/search/selection; 0 changed RGBA`);
    }
  }
  if(process.env.FOLIO_OCR_VISUAL) {
    await page.getByRole("button",{name:"Recognize scanned text",exact:true}).click();
    const active=page.locator(".document-workspace:not([hidden])");
    await active.getByLabel("OCR language",{exact:true}).selectOption("eng");
    await active.getByLabel("Additional language 1",{exact:true}).selectOption("ara");
    await active.getByLabel("Additional language 2",{exact:true}).selectOption("hin");
    await active.getByLabel("OCR text layout",{exact:true}).selectOption("single_column");
    evidence.visual=[];
    for(const [theme,width,height] of [["light",1440,1000],["dark",950,800]]) {
      await page.setViewportSize({width,height});
      await page.evaluate(()=>new Promise((resolve)=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
      await page.evaluate((theme)=>{document.documentElement.dataset.theme=theme;},theme);
      if(!await page.getByRole("button",{name:"Recognize text",exact:true}).isVisible()) {
        await page.getByRole("button",{name:"All tools",exact:true}).click();
        await page.getByRole("button",{name:"Recognize scanned text",exact:true}).click();
      }
      await active.getByLabel("Scan resolution").selectOption("300");
      const warning = active.getByRole("status").filter({ hasText: "Choose 96 DPI" });
      await warning.waitFor();
      assert.equal(await active.getByRole("button", { name: "Recognize text", exact: true }).isDisabled(), true);
      const warningBounds = await warning.boundingBox();
      assert.ok(warningBounds.y >= 0 && warningBounds.y + warningBounds.height <= height);
      await page.screenshot({path:join(artifacts,`ocr-${theme}-oversized-warning.png`)});
      await active.getByLabel("Scan resolution").selectOption("96");
      await active.getByRole("button",{name:"Recognize text",exact:true}).focus();
      await page.evaluate(()=>{document.querySelector(".document-workspace:not([hidden]) .ocr-panel-content").scrollTop=0;});
      const metrics=await active.evaluate((workspace)=>{
        const panel=workspace.querySelector(".ocr-panel").getBoundingClientRect(),button=workspace.querySelector(".ocr-start").getBoundingClientRect();
        return {overflow:document.documentElement.scrollWidth>innerWidth,panel:[panel.left,panel.top,panel.right,panel.bottom],
          action:[button.left,button.top,button.right,button.bottom],focus:document.activeElement===workspace.querySelector(".ocr-start")};
      });
      assert.equal(metrics.overflow,false);
      assert.equal(metrics.focus,true);
      assert.ok(metrics.action[1]>=metrics.panel[1] && metrics.action[3]<=metrics.panel[3],"OCR action must stay visible while settings scroll");
      await page.screenshot({path:join(artifacts,`ocr-${theme}-${width>=1100?"desktop":"compact"}-final.png`)});
      evidence.visual.push({theme,width,height,...metrics});
    }
    await page.setViewportSize({width:1440,height:1000});
    await page.evaluate(()=>{document.documentElement.dataset.theme="light";});
  }
  if(process.env.FOLIO_OCR_GUARDS) {
    await fileInput.setInputFiles(sourcePath);
    await page.waitForFunction(()=>document.querySelector(".document-tab.selected")?.textContent.includes("synthetic-scan.pdf"));
    const snapshot=await(await api("/bootstrap")).json();
    const source=snapshot.documents.find((info)=>info.id===snapshot.activeId);
    const before=await bytes(source);
    evidence.guards=await page.evaluate(async ({info,png})=>{
      const {bridge}=await import("/src/lib/bridge.ts");
      const {imageEditingBridge}=await import("/src/features/imageEditing/bridge.ts");
      const {createDraftSynchronizer,hasDocumentDrafts}=await import("/src/lib/draftProtection.ts");
      const synchronize=createDraftSynchronizer(bridge.draft);
      const clear={writing:false,formFilling:false,formDesign:false,pageAuthoring:false,imageEditing:false,paragraphEditing:false,accessibility:false,redaction:false};
      const options={pages:[0],dpi:150,languages:["eng"],layout:"horizontal_print",segmentation:"auto",existingText:"skip"};
      const observed=[];
      const rejected=async(operation,label,expression)=>{
        try {await operation;} catch(error) {
          if(!(error instanceof Error)||!expression.test(error.message))throw error;
          observed.push({label,error:error.message});return;
        }
        throw new Error(`${label}: an unsupported/stale OCR action was accepted`);
      };
      for(const [label,field] of [["writing","writing"],["metadata","writing"],["live gesture","writing"],["form filling","formFilling"],
        ["form design","formDesign"],["page authoring","pageAuthoring"],["image editing","imageEditing"],["paragraph editing","paragraphEditing"],
        ["accessibility","accessibility"],["redaction marks","redaction"]]) {
        await synchronize(info.id,hasDocumentDrafts({...clear,[field]:true}));
        await rejected(bridge.beginOcr(info,options),label,/pending|Apply or discard|draft|redaction/i);
        await synchronize(info.id,hasDocumentDrafts(clear));
      }
      await synchronize(info.id,hasDocumentDrafts({...clear,paragraphEditing:true,accessibility:true}));
      await synchronize(info.id,hasDocumentDrafts({...clear,paragraphEditing:true,accessibility:false}));
      await rejected(bridge.beginOcr(info,options),"clearing accessibility retains paragraph draft",/pending|Apply or discard|draft/i);
      await synchronize(info.id,hasDocumentDrafts(clear));
      const stale=await bridge.beginOcr(info,options);
      await synchronize(info.id,hasDocumentDrafts({...clear,paragraphEditing:true}));
      await synchronize(info.id,hasDocumentDrafts(clear));
      await rejected(bridge.stageOcrPage(stale.jobId,{page:0,words:[]}),"draft clearing cannot resurrect a job",/canceled|expired|invalidated|epoch/i);
      const pngFile=new File([new Uint8Array(png)],"synthetic-draft.png",{type:"image/png"});
      const imageStale=await bridge.beginOcr(info,options);
      const asset=await bridge.stageImage(info,pngFile);
      await rejected(bridge.beginOcr(info,options),"staged page image",/pending|Apply or discard/i);
      await bridge.cancelImage(info.id,asset.assetId);
      await rejected(bridge.stageOcrPage(imageStale.jobId,{page:0,words:[]}),"image cancellation cannot resurrect a job",/canceled|expired|invalidated|epoch/i);
      const replacementStale=await bridge.beginOcr(info,options);
      const replacement=await imageEditingBridge.stage(info,pngFile);
      await rejected(bridge.beginOcr(info,options),"staged source replacement",/pending|Apply or discard/i);
      await imageEditingBridge.cancel(info.id,replacement.assetId);
      await rejected(bridge.stageOcrPage(replacementStale.jobId,{page:0,words:[]}),"replacement cancellation cannot resurrect a job",/canceled|expired|invalidated|epoch/i);
      const running=await bridge.beginOcr(info,options);
      await rejected(bridge.beginRedaction(info,150,[{page:0,rect:[20,50,80,90]}]),"OCR excludes concurrent redaction",/Cancel.*OCR/i);
      await bridge.cancelOcr(running.jobId);
      const redaction=await bridge.beginRedaction(info,150,[{page:0,rect:[20,50,80,90]}]);
      await rejected(bridge.beginOcr(info,options),"prepared redaction excludes OCR",/pending|draft|redaction/i);
      await bridge.cancelRedaction(redaction.jobId);
      return observed;
    },{info:source,png:scans[0]});
    assert.deepEqual(await bytes(source),before,"Every draft/stage/cancel guard must preserve exact source bytes");
    evidence.guardSourceUnchanged=true;
  }
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
    lib: { entry: join(root, "src/lib/ocr.ts"), formats: ["es"], fileName: () => "ocr-harness.js" },
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
    if(!nonSimd && offlineFixtures.length) {
      evidence.offlineLanguages=[];
      for(const fixture of offlineFixtures) {
        const actual=await tab.evaluate(async ({png,languages})=>{
          const {LocalOcrWorker,horizontalOcrBands,recognitionWords}=await import("/ocr-harness.js");
          const signal=new AbortController().signal;
          const bitmap=await createImageBitmap(new Blob([new Uint8Array(png)],{type:"image/png"}));
          const canvas=document.createElement("canvas");canvas.width=bitmap.width;canvas.height=bitmap.height;
          const ctx=canvas.getContext("2d",{alpha:false});ctx.drawImage(bitmap,0,0);bitmap.close();
          const worker=new LocalOcrWorker(signal,()=>{});
          try {
            await worker.initialize(300,languages,"single_column");
            const bands=await horizontalOcrBands(ctx.getImageData(0,0,canvas.width,canvas.height).data,canvas.width,canvas.height,300,signal);
            let text="";
            for(const band of bands) {
              const cropped=document.createElement("canvas");cropped.width=canvas.width;cropped.height=band.height;
              try {
                cropped.getContext("2d",{alpha:false}).drawImage(canvas,0,band.top,canvas.width,band.height,0,0,canvas.width,band.height);
                const blob=await new Promise((resolve)=>cropped.toBlob(resolve,"image/png"));
                const output=await worker.recognize(await blob.arrayBuffer());
                const words=recognitionWords(output,{page:0,pixelWidth:canvas.width,pixelHeight:band.height});
                text+=words.map((word)=>word.text+word.separator).join("");
              } finally {cropped.width=cropped.height=0;}
            }
            return {text,online:navigator.onLine,bands:bands.length};
          } finally {worker.close();canvas.width=canvas.height=0;}
        },fixture);
        assert.equal(actual.online,false);
        assert.ok(accuracy(fixture.lines.join("\n"),actual.text)>=0.9,`${fixture.language}: real network-offline recognition`);
        assert.equal(await tab.evaluate(()=>window.__ocrWorkers.active),0);
        evidence.offlineLanguages.push({language:fixture.language,languages:fixture.languages,...actual,
          accuracy:accuracy(fixture.lines.join("\n"),actual.text)});
      }
    }
    evidence.offline.push({ nonSimd, networkOffline: true, recognizedKnownPhrase: true, requested });
    await offline.close();
  }
  assert.deepEqual(evidence.externalRequests, []);
  assert.deepEqual(failures, []);
  evidence.sourceSha256 = createHash("sha256").update(source).digest("hex");
  evidence.outputSha256 = createHash("sha256").update(recognizedBytes).digest("hex");
  await writeFile(join(artifacts, "ocr-evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
  const languages=evidence.languages?.filter((item)=>item.language!=="mixed").length??0;
  const combination=evidence.languages?.some((item)=>item.language==="mixed")?" plus a real three-language scan":"";
  console.log(`Offline OCR passed: real English regression${languages?` and ${languages} selected languages${combination}`:""}; actual saved Unicode/search/selection/direction; unchanged pixels and exact four-rotation CropBox/UserUnit geometry; skip/force/no-text/cancel/cleanup; bundled production worker with networking disabled (SIMD and non-SIMD).`);
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
