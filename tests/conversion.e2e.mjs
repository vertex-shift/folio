import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, unlink, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { inflateSync } from "node:zlib";
import { PDFDict, PDFDocument, PDFName, PDFRawStream } from "pdf-lib";
import { chromium } from "playwright";
import { createServer as createViteServer } from "vite";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const artifacts = process.env.FOLIO_CONVERSION_ARTIFACTS
  ? resolve(process.env.FOLIO_CONVERSION_ARTIFACTS) : await mkdtemp(join(tmpdir(), "folio-conversion-"));
await mkdir(artifacts, { recursive: true });
const fixtures = await mkdtemp(join(artifacts, "fixtures-"));
const target = resolve(process.env.CARGO_TARGET_DIR ?? join(root, "target/conversion-build"));
const env = { ...process.env, CARGO_TARGET_DIR: target, CARGO_INCREMENTAL: "0",
  CARGO_PROFILE_DEV_DEBUG: "0", CARGO_PROFILE_TEST_DEBUG: "0", CARGO_BUILD_JOBS: "2",
  ...(process.env.FOLIO_CONVERSION_PYTHONPATH ? { PYTHONPATH: process.env.FOLIO_CONVERSION_PYTHONPATH } : {}) };
const children = [], privateFiles = [];
const evidence = { artifacts, fixtures, checks: [], visuals: [], externalRequests: [], pageErrors: [], docx: [], draftWrites: [] };
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
let browser, page, frontend;

function start(executable, args, extra = {}) {
  const child = spawn(executable, args, { cwd: root, env: { ...env, ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  child.output = "";
  for (const stream of [child.stdout, child.stderr]) stream.on("data", (bytes) => { child.output = (child.output + bytes).slice(-24000); });
  child.on("error", (error) => { child.startError = error; });
  children.push(child);
  return child;
}
async function command(executable, args) {
  const child = start(executable, args);
  await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code) => code === 0 ? resolve() : reject(new Error(`${executable} (${code}): ${child.output}`)));
  });
  return child.output.trim();
}
async function port(preferred) {
  const server = createServer();
  const listen = (number) => new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(number, "127.0.0.1", () => { server.removeListener("error", reject); resolve(); });
  });
  try { await listen(preferred); }
  catch (error) { if (error.code !== "EADDRINUSE") throw error; await listen(0); }
  const selected = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return selected;
}
async function ready(url, child) {
  for (let index = 0; index < 900; index++) {
    if (child.startError || child.exitCode !== null) throw new Error(child.startError?.message ?? child.output);
    try { if ((await fetch(url, { signal: AbortSignal.timeout(2000) })).ok) return; }
    catch (error) { if (index === 899) throw error; }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Preview not ready: ${child.output}`);
}
function record(label) {
  evidence.checks.push(label);
  console.log(`ok: ${label}`);
}
function accountResources() {
  window.__conversionAudit = { pdfWorkers: 0, urls: new Set() };
  const Worker = window.Worker;
  window.Worker = class extends Worker {
    constructor(url, options) {
      super(url, options);
      this.pdf = String(url).includes("pdf.worker");
      this.stopped = false;
      if (this.pdf) window.__conversionAudit.pdfWorkers++;
    }
    terminate() {
      if (!this.stopped && this.pdf) window.__conversionAudit.pdfWorkers--;
      this.stopped = true;
      return super.terminate();
    }
  };
  const create = URL.createObjectURL.bind(URL), revoke = URL.revokeObjectURL.bind(URL);
  URL.createObjectURL = (...args) => { const value = create(...args); window.__conversionAudit.urls.add(value); return value; };
  URL.revokeObjectURL = (value) => { window.__conversionAudit.urls.delete(value); return revoke(value); };
}
function exifSix(jpeg) {
  const exif = Buffer.alloc(32);
  exif.write("Exif\0\0", 0, "binary");
  exif.write("II", 6); exif.writeUInt16LE(42, 8); exif.writeUInt32LE(8, 10);
  exif.writeUInt16LE(1, 14); exif.writeUInt16LE(0x112, 16); exif.writeUInt16LE(3, 18);
  exif.writeUInt32LE(1, 20); exif.writeUInt16LE(6, 24);
  return Buffer.concat([jpeg.subarray(0, 2), Buffer.from([0xff, 0xe1, 0, 34]), exif, jpeg.subarray(2)]);
}
function imageObject(document, pageIndex) {
  const resources = document.getPage(pageIndex).node.Resources();
  const images = resources.lookup(PDFName.of("XObject"), PDFDict);
  const names = images.keys();
  assert.equal(names.length, 1);
  return images.lookup(names[0], PDFRawStream);
}
function plainStream(stream) {
  const filter = stream.dict.get(PDFName.of("Filter"));
  return filter?.toString() === "/FlateDecode" ? inflateSync(stream.getContents()) : stream.getContents();
}

try {
  if (process.env.FOLIO_CONVERSION_SKIP_BUILD !== "1") {
    await command("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--quiet"]);
  }
  const frontendPort = await port(1434), backendPort = await port(19162);
  const ports = { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) };
  evidence.ports = ports;
  const backend = start(join(target, "debug/folio-preview"), [], ports);
  await ready(`http://127.0.0.1:${backendPort}/health`, backend);
  const origin = `http://127.0.0.1:${frontendPort}`;
  frontend = await createViteServer({
    root, logLevel: "error", optimizeDeps: { include: ["pdf-lib"] },
    server: { host: "127.0.0.1", port: frontendPort, strictPort: true,
      proxy: { "/__folio": { target: `http://127.0.0.1:${backendPort}`, rewrite: (path) => path.replace(/^\/__folio/, "") } } },
  });
  await frontend.listen();
  async function api(route, value = null, { success = true, headers = {}, bytes } = {}) {
    const response = await fetch(`http://127.0.0.1:${backendPort}${route}`, {
      headers: { "X-Folio-Preview": "1", Origin: origin,
        ...(value !== null ? { "Content-Type": "application/json" } : {}), ...headers },
      ...(bytes ? { method: "POST", body: bytes } : value !== null ? { method: "POST", body: JSON.stringify(value) } : {}),
    });
    if (success && !response.ok) throw new Error(`${route}: ${await response.text()}`);
    if (!success) assert.equal(response.ok, false, `${route} should refuse`);
    return response;
  }
  const snapshot = async () => (await (await api("/bootstrap")).json());
  const active = async () => { const state = await snapshot(); return state.documents.find((document) => document.id === state.activeId); };
  const bytes = async (info) => Buffer.from(await (await api("/bytes", null, { headers: {
    "X-Document-Id": String(info.id), "X-Revision": String(info.revision),
  } })).arrayBuffer());
  browser = await chromium.launch({ headless: true,
    ...(process.env.FOLIO_CONVERSION_BROWSER ? { executablePath: process.env.FOLIO_CONVERSION_BROWSER } : {}) });
  evidence.browser = browser.version();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce", colorScheme: "light", acceptDownloads: true });
  await context.addInitScript(accountResources);
  await context.route("**/*", (route) => {
    const url = new URL(route.request().url());
    if (url.origin === origin || !["http:", "https:"].includes(url.protocol)) return route.continue();
    evidence.externalRequests.push(url.href);
    return route.abort();
  });
  page = await context.newPage();
  page.setDefaultTimeout(20_000);
  page.on("pageerror", (error) => evidence.pageErrors.push(error.message));
  page.on("request", (request) => {
    if (request.url().endsWith("/__folio/draft")) evidence.draftWrites.push(request.postDataJSON());
  });
  await page.goto(origin);
  const workspace = () => page.locator(".document-workspace:not([hidden])");
  async function idle() {
    await page.waitForFunction(() => {
      const create = document.querySelector(".create-files-button");
      const workspace = document.querySelector(".document-workspace:not([hidden])");
      return create && !create.disabled && workspace && !workspace.querySelector(".document-busy")
        && !workspace.querySelector(".continuous-loading") && !workspace.querySelector(".paper-loading")
        && workspace.querySelector("canvas");
    });
  }
  async function tools() {
    const panel = workspace().getByRole("button", { name: "Export Word text (.docx)", exact: true });
    if (!await panel.isVisible()) await workspace().getByRole("button", { name: "All tools", exact: true }).click();
    await panel.waitFor();
  }
  async function openUi(path, password) {
    await idle();
    await page.getByLabel("Choose PDF files", { exact: true }).setInputFiles(path);
    if (password !== undefined) {
      await page.getByLabel("PDF password", { exact: true }).fill(password);
      await page.getByRole("button", { name: "Unlock PDF", exact: true }).click();
    }
    await page.getByRole("tab", { name: basename(path), exact: true }).waitFor();
    await idle();
    return active();
  }
  const inspectText = (value) => page.evaluate(async (bytes) =>
    (await import("/tests/conversion.browser.ts")).inspectConversionText(bytes), Array.from(value));
  async function validateDocx(path, expected) {
    const expectation = path.replace(/\.docx$/, "-expected.json");
    await writeFile(expectation, JSON.stringify(expected, null, 2));
    const result = JSON.parse(await command(process.env.FOLIO_CONVERSION_PYTHON ?? "python3", ["tests/conversion.docx.py", path, expectation]));
    evidence.docx.push(result);
  }
  async function docxDialog() {
    await tools();
    await workspace().getByRole("button", { name: "Export Word text (.docx)", exact: true }).click();
    return page.getByRole("dialog", { name: "Export Word document content", exact: true });
  }
  async function prepareDocx(range, acknowledge = false) {
    const dialog = await docxDialog();
    if (range) {
      await dialog.getByLabel("Page range", { exact: true }).check();
      await dialog.getByLabel("Page numbers", { exact: true }).fill(range);
    }
    if (acknowledge) await dialog.getByRole("checkbox", { name: /I understand and want to export unencrypted document content/ }).check();
    await dialog.getByRole("button", { name: "Prepare DOCX", exact: true }).click();
    await dialog.getByRole("heading", { name: "Editable document text is ready", exact: true }).waitFor();
    await page.waitForFunction(() => [...document.querySelectorAll(".output-dialog button")].some((button) => button.textContent.includes("Save DOCX") && !button.disabled));
    assert.equal(await dialog.evaluate((element) => element.contains(document.activeElement)), true, "DOCX ready lost keyboard focus");
    return dialog;
  }
  async function saveDocx(dialog, name) {
    const download = page.waitForEvent("download");
    await dialog.getByRole("button", { name: "Save DOCX copy", exact: true }).click();
    const path = join(artifacts, name);
    await (await download).saveAs(path);
    await dialog.waitFor({ state: "hidden" });
    await idle();
    return path;
  }
  async function newCreation(kind, paths) {
    await idle();
    await page.locator(".create-files-button").click();
    const dialog = page.getByRole("dialog", { name: "Create PDF from files", exact: true });
    if (kind === "text") await dialog.getByLabel("UTF-8 text", { exact: true }).check();
    await dialog.getByLabel("Choose creation files", { exact: true }).setInputFiles(paths);
    await dialog.getByRole("button", { name: "Prepare PDF", exact: true }).waitFor({ state: "visible" });
    await page.waitForFunction(() => !document.querySelector(".conversion-dialog .dialog-actions .primary")?.disabled);
    return dialog;
  }
  async function prepareCreation(dialog) {
    await dialog.getByRole("button", { name: "Prepare PDF", exact: true }).click();
    await dialog.getByText("New PDF ready", { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => document.querySelector(".conversion-dialog").contains(document.activeElement)), true, "Creation ready lost keyboard focus");
  }
  async function commitCreation(dialog) {
    await dialog.getByRole("button", { name: "Open new PDF", exact: true }).click();
    await dialog.waitFor({ state: "hidden" });
    await idle();
    return active();
  }
  function gateRoute(pattern) {
    let enter, release, complete, fail;
    const entered = new Promise((resolve) => { enter = resolve; });
    const released = new Promise((resolve) => { release = resolve; });
    const finished = new Promise((resolve, reject) => { complete = resolve; fail = reject; });
    const handler = async (route) => {
      enter();
      try {
        const canceled = await released;
        if (!canceled) await route.continue();
        complete();
      } catch (error) { fail(error); }
    };
    return { pattern, handler, entered, release, finished };
  }

  await idle();
  await page.keyboard.press("Control+Shift+N");
  const keyboardDialog = page.getByRole("dialog", { name: "Create PDF from files", exact: true });
  await keyboardDialog.waitFor();
  await page.keyboard.press("Escape");
  await keyboardDialog.waitFor({ state: "hidden" });
  await idle();
  record("Keyboard Create from files and Escape alternatives preserve the existing workspace");
  const generated = await page.evaluate(async () => (await import("/tests/conversion.browser.ts")).syntheticConversionInputs());
  const inputPaths = {};
  const generatedFiles = {
    "Original-alpha.png": Buffer.from(generated.alpha, "base64"),
    "Oriented-replacement.jpg": exifSix(Buffer.from(generated.jpeg, "base64")),
    "Editable-text.pdf": Buffer.from(generated.pdf),
    "Blank-scan.pdf": Buffer.from(generated.blank),
    "Unicode-pages.txt": Buffer.from("\ufeffLocal Unicode marker 83412\r\nZoë café Ελληνικά Привет <w:t>&\"quotes\"\r\nTab\tseparated\r\n"
      + Array.from({ length: 85 }, (_, index) => `Paragraph ${String(index).padStart(3, "0")}: original local text for wrapped, readable pagination. ${"Longer text stays searchable and selectable. ".repeat(4)}\r\n`).join("")
      + "Last Unicode marker 72045\r\n"),
    "Second-text.txt": Buffer.from("SECOND FILE 87492\nFile order must be preserved."),
    "Invalid-utf8.txt": Buffer.from([0xc3, 0x28]),
    "Unsupported-script.txt": Buffer.from("Explicit unsupported script: مرحبا"),
    "Forbidden-xml-control.txt": Buffer.from("Explicit unsupported control \u0001 is not removed."),
  };
  for (const [name, generatedBytes] of Object.entries(generatedFiles)) {
    const value = process.env.FOLIO_CONVERSION_ACCEPTANCE_FIXTURES && name !== "Blank-scan.pdf"
      ? await readFile(join(process.env.FOLIO_CONVERSION_ACCEPTANCE_FIXTURES, name)) : generatedBytes;
    const path = join(fixtures, name);
    await writeFile(path, value);
    inputPaths[name] = path;
  }
  evidence.inputSha256 = Object.fromEntries(await Promise.all(Object.entries(inputPaths).map(async ([name, path]) => [name, digest(await readFile(path))])));
  const baseline = await snapshot();
  const baselineBytes = digest(await bytes(await active()));
  evidence.extractionChecks = await page.evaluate(async ({ source, blank }) =>
    (await import("/tests/conversion.browser.ts")).checkDocxExtractionCancellation(source, blank),
  { source: Array.from(await readFile(inputPaths["Editable-text.pdf"])), blank: generated.blank });
  record("Real PDF.js DOCX extraction cancels before staging, refuses stale sources and empty scans");

  let dialog = await newCreation("images", [inputPaths["Original-alpha.png"], inputPaths["Oriented-replacement.jpg"]]);
  assert.equal(await page.evaluate(() => { const event = new Event("beforeunload", { cancelable: true }); window.dispatchEvent(event); return event.defaultPrevented; }), true);
  await dialog.getByRole("button", { name: "Move Oriented-replacement.jpg up", exact: true }).click();
  await dialog.getByLabel("Page size", { exact: true }).selectOption("a4");
  await dialog.getByLabel("Orientation", { exact: true }).selectOption("landscape");
  await dialog.getByLabel("Margin (points)", { exact: true }).fill("48");
  await prepareCreation(dialog);
  assert.equal((await snapshot()).documents.length, baseline.documents.length, "Prepared PDF prematurely created a tab");
  const images = await commitCreation(dialog);
  assert.ok(images.dirty && !images.canUndo && !images.canRedo);
  const imageBytes = await bytes(images);
  await writeFile(join(artifacts, "created-images.pdf"), imageBytes);
  const imagePdf = await PDFDocument.load(imageBytes);
  assert.equal(imagePdf.getPageCount(), 2);
  for (const page of imagePdf.getPages()) {
    assert.ok(Math.abs(page.getWidth() - 841.89) < 0.02 && Math.abs(page.getHeight() - 595.28) < 0.02);
    assert.equal(page.node.has(PDFName.of("Annots")), false, "Creation produced removable image-stamp overlays");
  }
  const jpegImage = imageObject(imagePdf, 0);
  assert.equal(jpegImage.dict.lookup(PDFName.of("Width")).asNumber(), 72);
  assert.equal(jpegImage.dict.lookup(PDFName.of("Height")).asNumber(), 96);
  const alphaImage = imageObject(imagePdf, 1);
  assert.equal(alphaImage.dict.get(PDFName.of("ColorSpace")).toString(), "/DeviceRGB");
  const mask = alphaImage.dict.lookup(PDFName.of("SMask"), PDFRawStream);
  const alphaPixels = plainStream(mask);
  assert.ok(alphaPixels.includes(0) && alphaPixels.includes(255) && alphaPixels.some((value) => value > 0 && value < 255));
  assert.equal(plainStream(alphaImage).length, alphaPixels.length * 3);
  evidence.imagePixels = await page.evaluate(async (input) =>
    (await import("/tests/conversion.browser.ts")).inspectConversionImages(input), {
    pdf: Array.from(imageBytes),
    images: [Array.from(await readFile(inputPaths["Oriented-replacement.jpg"])), Array.from(await readFile(inputPaths["Original-alpha.png"]))],
    margin: 48,
  });
  assert.equal(digest(await bytes(baseline.documents[0])), baselineBytes);
  record("Independent PDF.js/pdf-lib: ordered real image pages, A4 landscape/margins, EXIF-6 72x96, RGB/alpha pixels and no stamp overlays");

  const imagesState = JSON.stringify(images);
  dialog = await newCreation("text", [inputPaths["Unicode-pages.txt"], inputPaths["Second-text.txt"]]);
  await dialog.getByRole("button", { name: "Move Second-text.txt up", exact: true }).click();
  await dialog.getByLabel("Text size (points)", { exact: true }).fill("12");
  await prepareCreation(dialog);
  const text = await commitCreation(dialog);
  const textBytes = await bytes(text);
  await writeFile(join(artifacts, "created-unicode-text.pdf"), textBytes);
  const textPdf = await PDFDocument.load(textBytes);
  const textInspection = await inspectText(textBytes);
  assert.ok(textPdf.getPageCount() >= 4);
  assert.ok(textInspection.pages[0].includes("87492"));
  const combined = textInspection.pages.join("\n");
  const normalizeInput = (value) => value.replace(/^\ufeff/u, "").replace(/\r\n?/g, "\n").replace(/\t/g, "    ");
  const expectedText = normalizeInput(await readFile(inputPaths["Second-text.txt"], "utf8"))
    + normalizeInput(await readFile(inputPaths["Unicode-pages.txt"], "utf8"));
  assert.equal(combined.replace(/\s/gu, ""), expectedText.replace(/\s/gu, ""), "Paginated PDF changed original non-whitespace Unicode text");
  assert.match(combined, /[\u0370-\u03ff]/u);
  assert.match(combined, /[\u0400-\u04ff]/u);
  assert.ok(combined.includes("<w:t>") && combined.includes("&"));
  assert.ok(!combined.includes("\ufeff") && !combined.includes("\r"));
  assert.ok(textInspection.bounds.every((page) => page.outOfBounds === 0));
  assert.ok(textInspection.bounds.every((page) => page.minLeft === null || page.minLeft >= 35.9 && page.maxRight <= page.width - 35.9), "Text escaped the selected 36-point margins");
  for (const page of textPdf.getPages()) {
    const fonts = page.node.Resources().lookup(PDFName.of("Font"), PDFDict);
    const font = fonts.lookup(fonts.keys()[0], PDFDict);
    assert.equal(font.get(PDFName.of("Subtype")).toString(), "/Type0");
    assert.ok(font.lookup(PDFName.of("ToUnicode"), PDFRawStream).getContents().length > 0);
    const descendant = font.lookup(PDFName.of("DescendantFonts")).lookup(0, PDFDict);
    const descriptor = descendant.lookup(PDFName.of("FontDescriptor"), PDFDict);
    assert.ok(descriptor.lookup(PDFName.of("FontFile2"), PDFRawStream).getContents().length > 0);
  }
  assert.equal(JSON.stringify((await snapshot()).documents.find((document) => document.id === images.id)), imagesState);
  evidence.textPdf = { pages: textInspection.pages.length, pageBounds: textInspection.bounds, sha256: digest(textBytes) };
  record("Independent PDF.js/pdf-lib: strict BOM/CRLF UTF-8, Latin/Greek/Cyrillic, file order, wrapped paginated real page text and unchanged source/history");

  const beforeWorkers = await page.evaluate(() => window.__conversionAudit.pdfWorkers);
  const unicodeDialog = await prepareDocx();
  const unicodeDocx = await saveDocx(unicodeDialog, "unicode-content.docx");
  await validateDocx(unicodeDocx, { pages: textInspection.pages, requiredFragments: ["<w:t>", "&",
    combined.match(/[\u0370-\u03ff]+/u)[0], combined.match(/[\u0400-\u04ff]+/u)[0]], forbiddenFragments: ["\ufeff"] });
  assert.equal(await page.evaluate(() => window.__conversionAudit.pdfWorkers), beforeWorkers);
  assert.equal(digest(await bytes(text)), digest(textBytes));
  record("Coupled UTF-8 → searchable PDF → real editable DOCX reopens with exact Unicode page text in independent python-docx");

  for (const [filename, expected] of [["Invalid-utf8.txt", "strict UTF-8"], ["Unsupported-script.txt", "unsupported"], ["Forbidden-xml-control.txt", "unsupported"]]) {
    const before = await snapshot();
    dialog = await newCreation("text", [inputPaths["Second-text.txt"], inputPaths[filename]]);
    await dialog.getByRole("button", { name: "Prepare PDF", exact: true }).click();
    await dialog.getByRole("alert").waitFor();
    assert.ok((await dialog.innerText()).includes(expected));
    assert.equal((await snapshot()).documents.length, before.documents.length);
    assert.equal(await dialog.getByRole("button", { name: "Open new PDF", exact: true }).count(), 0);
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await idle();
  }
  record("Invalid UTF-8, unsupported Arabic and forbidden control fail whole mixed batches without partial tabs");

  dialog = await newCreation("images", [inputPaths["Original-alpha.png"]]);
  const creationGate = gateRoute("**/__folio/creation/prepare");
  await page.route(creationGate.pattern, creationGate.handler);
  await dialog.getByRole("button", { name: "Prepare PDF", exact: true }).click();
  await creationGate.entered;
  await dialog.getByRole("button", { name: "Cancel preparation", exact: true }).click();
  await dialog.getByText("Creation canceled. No PDF tab was opened; existing documents and drafts are unchanged.", { exact: true }).waitFor();
  creationGate.release(true);
  await creationGate.finished;
  await page.unroute(creationGate.pattern, creationGate.handler);
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await idle();
  record("Real creation cancellation interrupts the blocked request, cleans the staged batch and opens no document");

  let editable = await openUi(inputPaths["Editable-text.pdf"]);
  await workspace().getByRole("button", { name: "Rotate current page clockwise", exact: true }).click();
  await idle();
  editable = await active();
  assert.ok(editable.dirty && editable.canUndo);
  const working = await bytes(editable), workingHash = digest(working);
  const editableText = await inspectText(working);
  const orderedTexts = [2, 0, 1].map((index) => editableText.pages[index]);
  dialog = await prepareDocx("3, 1-2");
  const ordered = await saveDocx(dialog, "ordered-document-content.docx");
  await validateDocx(ordered, { pages: orderedTexts, requiredFragments: ["72045", "83412", "29407", "<w:t>", "&"], forbiddenFragments: ["49211"] });
  assert.equal(digest(await bytes(editable)), workingHash);
  assert.deepEqual((await snapshot()).documents.find((document) => document.id === editable.id), editable);
  evidence.source = { id: editable.id, revision: editable.revision, sha256: workingHash, canUndo: editable.canUndo, dirty: editable.dirty };
  record("Actual DOCX selected order 3,1,2 excludes page 4 and preserves unsaved working bytes, revision and undo history");

  for (const route of ["render-bytes", "page"]) {
    const cancelGate = gateRoute(`**/__folio/output/${route}`);
    await page.route(cancelGate.pattern, cancelGate.handler);
    const workers = await page.evaluate(() => window.__conversionAudit.pdfWorkers);
    dialog = await docxDialog();
    await dialog.getByRole("button", { name: "Prepare DOCX", exact: true }).click();
    await cancelGate.entered;
    assert.equal(await page.getByRole("button", { name: `Close ${editable.name}`, exact: true }).isDisabled(), true);
    await dialog.getByRole("button", { name: "Cancel preparation", exact: true }).click();
    await dialog.getByRole("button", { name: "Prepare DOCX", exact: true }).waitFor();
    cancelGate.release(true);
    await cancelGate.finished;
    await page.unroute(cancelGate.pattern, cancelGate.handler);
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await idle();
    assert.equal(digest(await bytes(editable)), workingHash);
    assert.equal(await page.evaluate(() => window.__conversionAudit.pdfWorkers), workers);
  }
  record("DOCX cancellation interrupts blocked snapshot/staging requests, cleans workers and guards in-flight closing without source changes");

  for (const invalidation of ["draft", "undo"]) {
    const gate = gateRoute("**/__folio/output/page");
    await page.route(gate.pattern, gate.handler);
    dialog = await docxDialog();
    await dialog.getByRole("button", { name: "Prepare DOCX", exact: true }).click();
    await gate.entered;
    if (invalidation === "draft") {
      await api("/draft", { documentId: editable.id, pending: true });
      await api("/draft", { documentId: editable.id, pending: false });
    } else {
      const changed = await (await api("/edit", { documentId: editable.id, revision: editable.revision, edit: { type: "rotate", page: 0, degrees: 90 } })).json();
      await api("/undo", { documentId: editable.id, revision: changed.revision });
    }
    gate.release();
    await gate.finished;
    await dialog.getByRole("alert").waitFor();
    assert.match(await dialog.innerText(), /canceled|closed|changed|pending/i);
    assert.equal(await dialog.getByRole("button", { name: "Save DOCX copy", exact: true }).count(), 0);
    await page.unroute(gate.pattern, gate.handler);
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await idle();
    assert.equal(digest(await bytes(editable)), workingHash);
  }
  record("Draft→clear and edit→undo cannot resurrect an in-flight DOCX job with the old revision");

  await workspace().getByRole("button", { name: "Show document properties", exact: true }).click();
  await workspace().getByLabel("Title", { exact: true }).fill("Creation must preserve this metadata draft");
  await page.waitForFunction((id) => document.querySelector(`#document-tab-${id} .unsaved-dot`) !== null, editable.id);
  dialog = await newCreation("images", [inputPaths["Original-alpha.png"]]);
  await prepareCreation(dialog);
  await commitCreation(dialog);
  await api("/bytes", null, { success: false, headers: { "X-Document-Id": String(editable.id), "X-Revision": String(editable.revision) } });
  assert.equal(evidence.draftWrites.filter((write) => write.documentId === editable.id).at(-1)?.pending, true);
  await page.getByRole("tab", { name: editable.name, exact: true }).click();
  const originalWorkspace = page.locator(`#document-panel-${editable.id}`);
  await originalWorkspace.waitFor({ state: "visible" });
  await originalWorkspace.getByLabel("Title", { exact: true }).waitFor();
  assert.equal(await originalWorkspace.getByLabel("Title", { exact: true }).inputValue(), "Creation must preserve this metadata draft");
  const draftCleared = page.waitForResponse((response) => response.url().endsWith("/__folio/draft")
    && response.request().postDataJSON().documentId === editable.id && response.request().postDataJSON().pending === false);
  await originalWorkspace.getByRole("button", { name: "Discard draft", exact: true }).click();
  assert.equal((await draftCleared).ok(), true);
  await idle();
  assert.equal(digest(await bytes(editable)), workingHash);
  record("New-file settings/uploads/preparation/new tab preserve an inactive original metadata draft and its save guard");

  await tools();
  await workspace().getByRole("button", { name: "Page authoring", exact: true }).click();
  await workspace().getByLabel("Choose image to insert", { exact: true }).setInputFiles(inputPaths["Original-alpha.png"]);
  await workspace().getByRole("button", { name: "Insert image", exact: true }).waitFor();
  await idle();
  await tools();
  await workspace().getByRole("button", { name: "Export Word text (.docx)", exact: true }).click();
  assert.equal(await page.getByRole("dialog", { name: "Export Word document content", exact: true }).count(), 0);
  await api("/output/begin", { documentId: editable.id, revision: editable.revision, kind: "docx", pages: [0],
    dpi: null, quality: null, allowUnencrypted: true }, { success: false });
  await workspace().getByRole("button", { name: "Page authoring", exact: true }).click();
  await workspace().getByRole("button", { name: "Discard authoring draft", exact: true }).click();
  await workspace().getByText("Authoring draft discarded. The PDF was not changed.", { exact: true }).first().waitFor();
  await idle();
  assert.equal(digest(await bytes(editable)), workingHash);
  record("Staged source images remain unapplied and block DOCX in UI and engine without clearing the draft");

  await page.locator(`#document-tab-${images.id}`).click();
  await page.locator(`#document-panel-${images.id}`).waitFor({ state: "visible" });
  for (const source of ["actual image-only PDF", "blank PDF"]) {
    const blank = source === "blank PDF" ? await openUi(inputPaths["Blank-scan.pdf"]) : images;
    dialog = await docxDialog();
    await dialog.getByRole("button", { name: "Prepare DOCX", exact: true }).click();
    await dialog.getByRole("alert").waitFor();
    assert.match(await dialog.innerText(), /No editable text/);
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await idle();
    assert.equal((await snapshot()).documents.find((document) => document.id === blank.id).dirty, blank.dirty);
  }
  record("Actual raster-only and blank PDFs report No editable text with no invented Word content");

  const protectedRefs = {};
  for (const copy of [true, false]) {
    const protectedBytes = Buffer.from(await (await api("/protection/export", { documentId: editable.id, revision: editable.revision,
      authorizationPassword: null, protection: { mode: "protect", openingPassword: "reader", ownerPassword: "owner",
        permissions: { print: "none", copy, edit: false, annotate: false, fillForms: false, assemble: false } } })).arrayBuffer());
    const path = join(fixtures, `Copy-${copy}.pdf`);
    await writeFile(path, protectedBytes);
    const protectedInfo = await openUi(path, "reader");
    protectedRefs[copy] = protectedInfo;
    assert.ok(protectedInfo.passwordProtected && protectedInfo.readOnlyReason);
    assert.equal(protectedInfo.permissions.ownerAuthenticated, false);
    if (copy) {
      dialog = await docxDialog();
      assert.equal(await dialog.getByRole("button", { name: "Prepare DOCX", exact: true }).isDisabled(), true);
      await dialog.getByRole("checkbox", { name: /I understand and want to export unencrypted document content/ }).check();
      await dialog.getByRole("button", { name: "Prepare DOCX", exact: true }).click();
      await dialog.getByRole("heading", { name: "Editable document text is ready", exact: true }).waitFor();
      assert.equal(await dialog.getByRole("checkbox", { name: /I understand and want to export unencrypted document content/ }).isDisabled(), true);
      const output = await saveDocx(dialog, "protected-authorized-content.docx");
      await validateDocx(output, { pages: editableText.pages });
      assert.equal(digest(await bytes(protectedInfo)), digest(protectedBytes));
    } else {
      await tools();
      await workspace().getByRole("button", { name: "Export Word text (.docx)", exact: true }).click();
      assert.equal(await page.getByRole("dialog", { name: "Export Word document content", exact: true }).count(), 0);
      await api("/output/begin", { documentId: protectedInfo.id, revision: protectedInfo.revision, kind: "docx",
        pages: [0], dpi: null, quality: null, allowUnencrypted: true }, { success: false });
      const ownerPath = join(fixtures, "Copy-false-owner.pdf");
      await writeFile(ownerPath, protectedBytes);
      const owner = await openUi(ownerPath, "owner");
      assert.equal(owner.permissions.ownerAuthenticated, true);
      const ownerDialog = await prepareDocx("3, 1-2", true);
      const output = await saveDocx(ownerDialog, "owner-authorized-content.docx");
      await validateDocx(output, { pages: orderedTexts });
    }
  }
  record("Real AES-256 copy-only/restricted user and authenticated owner sources enforce rights and explicit unencrypted DOCX acknowledgment");

  const key = join(fixtures, "runtime-signer.key"), certificate = join(fixtures, "runtime-signer.crt"), identity = join(fixtures, "runtime-signer.p12");
  privateFiles.push(key, certificate, identity);
  await command("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-sha256", "-nodes", "-keyout", key, "-out", certificate,
    "-days", "2", "-subj", "/CN=Folio synthetic conversion signer", "-addext", "basicConstraints=critical,CA:false", "-addext", "keyUsage=digitalSignature"]);
  await command("openssl", ["pkcs12", "-export", "-inkey", key, "-in", certificate, "-out", identity, "-passout", "pass:SyntheticConversionOnly"]);
  const signed = Buffer.from(await (await api("/signatures/export", { documentId: editable.id, revision: editable.revision,
    identity: Array.from(await readFile(identity)), password: "SyntheticConversionOnly", fieldId: null })).arrayBuffer());
  const signedPath = join(fixtures, "Real-signed-source.pdf");
  await writeFile(signedPath, signed);
  const signedInfo = await openUi(signedPath);
  assert.ok(signedInfo.readOnlyReason && signedInfo.copyAllowed);
  dialog = await prepareDocx("3, 1-2");
  const signedOutput = await saveDocx(dialog, "signed-source-content.docx");
  await validateDocx(signedOutput, { pages: orderedTexts });
  assert.equal(digest(await bytes(signedInfo)), digest(signed));
  const signedPdf = await PDFDocument.load(signed, { updateMetadata: false });
  let signatures = 0;
  for (const [, object] of signedPdf.context.enumerateIndirectObjects()) {
    if (!(object instanceof PDFDict) || object.get(PDFName.of("Type"))?.toString() !== "/Sig") continue;
    const [start, first, second, length] = object.lookup(PDFName.of("ByteRange")).asArray().map((value) => value.asNumber());
    assert.equal(start, 0);
    const data = join(fixtures, "signed-range.data"), cms = join(fixtures, "signature.cms"), verified = join(fixtures, "signature-verified.data");
    await writeFile(data, Buffer.concat([signed.subarray(0, first), signed.subarray(second, second + length)]));
    await writeFile(cms, object.lookup(PDFName.of("Contents")).asBytes());
    await command("openssl", ["cms", "-verify", "-binary", "-inform", "DER", "-noverify", "-in", cms, "-content", data, "-out", verified]);
    assert.equal(digest(await readFile(data)), digest(await readFile(verified)));
    signatures++;
  }
  assert.equal(signatures, 1);
  evidence.signed = { signatures, unchangedSha256: digest(signed), independentCmsVerification: true };
  record("A genuine locally signed/read-only PDF exports permitted DOCX without rewriting signed bytes; independent OpenSSL CMS verification still passes");

  const noHeader = await fetch(`http://127.0.0.1:${backendPort}/creation/begin`, { method: "POST", body: JSON.stringify({ kind: "text" }) });
  assert.equal(noHeader.ok, false);
  const alien = await fetch(`http://127.0.0.1:${backendPort}/creation/begin`, {
    method: "POST", headers: { "X-Folio-Preview": "1", Origin: "https://unrelated.invalid", "Content-Type": "application/json" }, body: JSON.stringify({ kind: "text" }),
  });
  assert.equal(alien.ok, false);
  await api("/creation/begin", { kind: "text", path: "/not-a-picked-file" }, { success: false });
  record("Preview enforces loopback-origin/custom-header rules and accepts bytes/names, not arbitrary creation paths");

  if (process.env.FOLIO_CONVERSION_SKIP_VISUAL !== "1") {
    for (const surface of ["creation", "docx"]) {
      if (surface === "creation") {
        dialog = await newCreation("text", [inputPaths["Unicode-pages.txt"], inputPaths["Second-text.txt"]]);
        await prepareCreation(dialog);
      } else {
        dialog = await prepareDocx("3, 1-2");
      }
      for (let index = 0; index < 8; index++) {
        await page.keyboard.press(index === 7 ? "Shift+Tab" : "Tab");
        assert.equal(await dialog.evaluate((element) => element.contains(document.activeElement)), true, `${surface} focus escaped its dialog`);
      }
      for (const theme of ["system", "light", "dark", "sepia", "contrast"]) {
        await page.evaluate((value) => {
          localStorage.setItem("folio.appearance", value);
          window.dispatchEvent(new StorageEvent("storage", { key: "folio.appearance", newValue: value }));
        }, theme);
        if (theme === "system") {
          await page.emulateMedia({ colorScheme: "dark" });
          await page.waitForFunction(() => document.documentElement.dataset.theme === "dark");
          await page.emulateMedia({ colorScheme: "light" });
          await page.waitForFunction(() => document.documentElement.dataset.theme === "light");
        }
        for (const [size, width, height] of [["desktop", 1440, 1000], ["compact", 760, 820]]) {
          await page.setViewportSize({ width, height });
          const geometry = await dialog.evaluate((element) => {
            const footer = element.querySelector(".dialog-actions").getBoundingClientRect();
            const body = element.querySelector(".dialog-body");
            const feedback = element.querySelector(".creation-ready, .output-result").getBoundingClientRect();
            const bodyBounds = body.getBoundingClientRect();
            const rgb = (value) => value.match(/[\d.]+/g).slice(0, 3).map(Number).map((channel) => {
              const normal = channel / 255; return normal <= 0.04045 ? normal / 12.92 : ((normal + 0.055) / 1.055) ** 2.4;
            });
            const luminance = (value) => { const color = rgb(value); return color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722; };
            const contrast = (a, b) => { const first = luminance(a), second = luminance(b); return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05); };
            const background = getComputedStyle(element).backgroundColor;
            const primary = element.querySelector(".dialog-actions .primary");
            return { horizontalOverflow: document.documentElement.scrollWidth > innerWidth,
              footerVisible: footer.top >= 0 && footer.bottom <= innerHeight, dialogMotion: getComputedStyle(element).animationName,
              bodyScrollable: getComputedStyle(body).overflowY === "auto", focusInside: element.contains(document.activeElement),
              readySummaryVisible: feedback.top >= bodyBounds.top && feedback.bottom <= bodyBounds.bottom,
              bodyContrast: contrast(getComputedStyle(body).color, background),
              mutedContrast: contrast(getComputedStyle(element.querySelector(".muted")).color, background),
              primaryContrast: contrast(getComputedStyle(primary).color, getComputedStyle(primary).backgroundColor) };
          });
          assert.equal(geometry.horizontalOverflow, false, `${surface}/${theme}/${size} overflows`);
          assert.equal(geometry.footerVisible, true, `${surface}/${theme}/${size} hides actions`);
          assert.equal(geometry.bodyScrollable, true);
          assert.equal(geometry.dialogMotion, "none", "Reduced Motion did not disable dialog travel");
          assert.ok(geometry.bodyContrast >= 4.5 && geometry.mutedContrast >= 4.5 && geometry.primaryContrast >= 4.5,
            `${surface}/${theme}/${size} text contrast failed: ${JSON.stringify(geometry)}`);
          assert.equal(geometry.focusInside, true);
          assert.equal(geometry.readySummaryVisible, true, `${surface}/${theme}/${size} hides its ready summary`);
          const path = join(artifacts, `${surface}-${theme}-${size}.png`);
          await page.screenshot({ path });
          evidence.visuals.push({ surface, theme, size, path, ...geometry });
        }
      }
      await page.setViewportSize({ width: 390, height: 844 });
      const footer = await dialog.locator(".dialog-actions").boundingBox();
      assert.ok(footer.y >= 0 && footer.y + footer.height <= 844);
      assert.equal(await dialog.evaluate((element) => {
        const feedback = element.querySelector(".creation-ready, .output-result").getBoundingClientRect();
        const body = element.querySelector(".dialog-body").getBoundingClientRect();
        return feedback.top >= body.top && feedback.bottom <= body.bottom;
      }), true, `${surface}/narrow hides its ready summary`);
      const path = join(artifacts, `${surface}-narrow.png`);
      await page.screenshot({ path });
      evidence.visuals.push({ surface, size: "narrow", path });
      await page.setViewportSize({ width: 1440, height: 1000 });
      await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
      await idle();
    }
    record("Batched desktop/compact/narrow review: five themes, fixed visible footers, scrollable dialogs and Reduce Motion");
  }
  for (const [name, path] of Object.entries(inputPaths)) assert.equal(digest(await readFile(path)), evidence.inputSha256[name], `${name} was modified`);
  await page.waitForFunction(() => window.__conversionAudit.urls.size === 0, null, { timeout: 35_000 });
  evidence.resources = await page.evaluate(() => ({ retainedObjectUrls: window.__conversionAudit.urls.size, activeReaderWorkers: window.__conversionAudit.pdfWorkers }));
  evidence.resources.openSyntheticTabs = (await snapshot()).documents.length;
  assert.equal(evidence.resources.activeReaderWorkers, evidence.resources.openSyntheticTabs, "An isolated conversion renderer worker was retained");
  assert.deepEqual(evidence.externalRequests, []);
  assert.deepEqual(evidence.pageErrors, []);
  record("All source files unchanged; no external document requests, leaked object URLs or browser exceptions");
  evidence.success = true;
} catch (error) {
  evidence.success = false;
  evidence.error = error.stack ?? String(error);
  if (page) evidence.lastVisibleUi = await page.locator("body").innerText().catch(() => "Browser page unavailable");
  throw error;
} finally {
  await writeFile(join(artifacts, "conversion-evidence.json"), JSON.stringify(evidence, null, 2));
  await browser?.close();
  await frontend?.close();
  for (const child of children.reverse()) if (child.exitCode === null) child.kill("SIGTERM");
  await Promise.all(children.map((child) => child.exitCode !== null ? Promise.resolve() : new Promise((resolve) => child.once("exit", resolve))));
  for (const path of privateFiles) {
    try { await unlink(path); }
    catch (error) { if (error.code !== "ENOENT") throw error; }
  }
  console.log(`Evidence: ${join(artifacts, "conversion-evidence.json")}`);
}
