import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { PDFDocument, PDFName, StandardFonts } from "pdf-lib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const artifacts = process.env.FOLIO_IMAGE_EVIDENCE_DIR
  ? resolve(process.env.FOLIO_IMAGE_EVIDENCE_DIR) : await mkdtemp(join(tmpdir(), "folio-image-editing-"));
const target = resolve(process.env.CARGO_TARGET_DIR ?? join(root, "target/image-edit-build"));
await mkdir(artifacts, { recursive: true });
const env = { ...process.env, CARGO_TARGET_DIR: target, CARGO_INCREMENTAL: "0",
  CARGO_PROFILE_DEV_DEBUG: "0", CARGO_PROFILE_TEST_DEBUG: "0", CARGO_BUILD_JOBS: "2",
  FOLIO_IMAGE_EVIDENCE_DIR: artifacts };
const children = [];
const evidence = { artifacts, checks: [], visuals: [], externalRequests: [], consoleErrors: [], draftWrites: [] };
let browser, page;
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
function near(actual, expected, label) {
  assert.ok(Math.abs(actual - expected) <= 0.03, `${label}: ${actual} differs from ${expected} by more than 0.03 physical pt`);
}

function start(command, args, extra = {}) {
  const child = spawn(command, args, { cwd: root, env: { ...env, ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  child.output = "";
  for (const stream of [child.stdout, child.stderr]) stream.on("data", (data) => {
    child.output = (child.output + data).slice(-16000);
  });
  child.on("error", (error) => { child.startError = error; });
  children.push(child);
  return child;
}

async function command(command, args) {
  const child = start(command, args);
  await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code) => code === 0 ? resolve() : reject(new Error(`${command} (${code}): ${child.output}`)));
  });
}

async function freePort(preferred) {
  const socket = createServer();
  try {
    await new Promise((resolve, reject) => { socket.once("error", reject); socket.listen(preferred, "127.0.0.1", resolve); });
  } catch (error) {
    socket.close();
    if (error.code === "EADDRINUSE") return freePort(0);
    throw error;
  }
  const port = socket.address().port;
  await new Promise((resolve) => socket.close(resolve));
  return port;
}

async function ready(url, child) {
  const deadline = Date.now() + 90_000;
  let lastError;
  while (Date.now() < deadline) {
    if (child.startError || child.exitCode !== null) throw new Error(child.startError?.message ?? child.output);
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(2000) });
      if (response.ok) return;
      lastError = new Error(`HTTP ${response.status}`);
    } catch (error) { lastError = error; }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Image-editing helper was not responsive: ${lastError}; ${child.output}`);
}

async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null || child.startError) return;
  child.kill("SIGTERM");
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    new Promise((resolve) => setTimeout(resolve, 5000)),
  ]);
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
}

try {
  if (process.env.FOLIO_IMAGE_SKIP_BUILD !== "1") {
    await command("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--test", "image_edit", "--quiet"]);
    await command("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--quiet"]);
  }
  const frontendPort = await freePort(1433), backendPort = await freePort(19161);
  const ports = { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) };
  evidence.ports = { frontendPort, backendPort };
  const engine = start(join(target, `debug/folio-preview${process.platform === "win32" ? ".exe" : ""}`), [], ports);
  await ready(`http://127.0.0.1:${backendPort}/health`, engine);
  const ui = start(process.execPath, ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--strictPort"], ports);
  const origin = `http://127.0.0.1:${frontendPort}`;
  await ready(origin, ui);
  async function request(route, options = {}, failure) {
    const response = await fetch(`http://127.0.0.1:${backendPort}${route}`, {
      ...options, headers: { "X-Folio-Preview": "1", ...options.headers },
    });
    if (failure) {
      assert.ok(!response.ok, `${route} must reject`);
      assert.match((await response.json()).error, failure, route);
    } else assert.ok(response.ok, `${route}: ${await response.clone().text()}`);
    return response;
  }
  const api = (route, value, failure) => request(route, value === undefined ? {} : {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(value),
  }, failure);
  const snapshot = async () => (await (await api("/bootstrap")).json());
  const activeInfo = async () => { const state = await snapshot(); return state.documents.find((item) => item.id === state.activeId); };
  const ref = (info) => ({ documentId: info.id, revision: info.revision });
  const documentBytes = async (info, render = false, failure) => {
    const response = await request(render ? "/render-bytes" : "/bytes", {
      headers: { "X-Document-Id": String(info.id), "X-Revision": String(info.revision) },
    }, failure);
    return failure ? null : new Uint8Array(await response.arrayBuffer());
  };
  const inspect = async (info, page = 0) => (await (await api("/images", { ...ref(info), page })).json());
  const source = new Uint8Array(await readFile(join(artifacts, "source.pdf")));
  const contract = JSON.parse(await readFile(join(artifacts, "fixture-contract.json"), "utf8"));
  evidence.sourceSha256 = digest(source);
  await api("/bootstrap");
  const sourceOpen = await (await request("/open", {
    method: "POST", headers: { "Content-Type": "application/pdf", "X-File-Name": encodeURIComponent("Image occurrences.pdf") },
    body: source,
  })).json();
  assert.equal(sourceOpen.status, "opened");
  const peer = await PDFDocument.create();
  peer.addPage([600, 770]).drawText("Original synthetic comparison peer", { x: 50, y: 700 });
  const peerBytes = await peer.save();
  await writeFile(join(artifacts, "peer.pdf"), peerBytes);

  browser = await chromium.launch({ headless: true,
    ...(process.env.FOLIO_IMAGE_BROWSER ? { executablePath: process.env.FOLIO_IMAGE_BROWSER } : {}) });
  evidence.browserVersion = browser.version();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce", acceptDownloads: true });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url());
    return route.abort();
  });
  await context.addInitScript(() => {
    window.__imageUrls = new Set();
    const create = URL.createObjectURL.bind(URL), revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = (...args) => {
      const url = create(...args);
      if (new Error().stack?.includes("useImageEditing")) window.__imageUrls.add(url);
      return url;
    };
    URL.revokeObjectURL = (url) => { window.__imageUrls.delete(url); return revoke(url); };
  });
  page = await context.newPage();
  page.on("pageerror", (error) => evidence.consoleErrors.push(error.message));
  page.on("request", (request) => {
    if (request.url().endsWith("/__folio/draft")) evidence.draftWrites.push(request.postDataJSON());
  });
  let downloads = 0;
  page.on("download", () => { downloads++; });
  const openFile = async (file) => {
    await page.waitForFunction(() => document.querySelector('button[aria-label="Open PDFs in new tabs"]')?.disabled === false
      && document.querySelector('.document-workspace:not([hidden]) canvas[aria-label^="Rendered PDF page"]')
      && !document.querySelector(".document-busy, .paper-loading"));
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await page.getByLabel("Choose PDF files", { exact: true }).setInputFiles(file);
  };
  await page.goto(origin);
  await page.getByRole("tab", { name: "Image occurrences.pdf", exact: true }).waitFor();
  const workspace = () => page.locator(".document-workspace:not([hidden])");
  const panel = () => workspace().getByRole("complementary", { name: "Edit existing images", exact: true });
  const field = (name) => panel().getByRole("spinbutton", { name, exact: true });
  const draftAck = (id, pending) => page.waitForResponse((response) => response.url().endsWith("/__folio/draft")
    && response.ok() && response.request().postDataJSON().documentId === id && response.request().postDataJSON().pending === pending);
  const settled = async () => {
    await workspace().getByRole("img", { name: "Rendered PDF page 1", exact: true }).waitFor();
    await workspace().locator(".document-busy").waitFor({ state: "hidden" });
    await workspace().locator(".paper-loading").first().waitFor({ state: "hidden" });
  };
  const chooseReplacement = async (filename) => {
    const [chooser] = await Promise.all([
      page.waitForEvent("filechooser"),
      panel().getByRole("button", { name: /Replace with PNG or JPEG|Choose another replacement/ }).click(),
    ]);
    await chooser.setFiles(join(artifacts, filename));
    await panel().locator(".image-replacement").waitFor();
    await workspace().locator(".document-busy").waitFor({ state: "hidden" });
  };
  await settled();
  const opened = await activeInfo();
  await workspace().getByRole("button", { name: "Edit existing images", exact: true }).click();
  await panel().locator(".image-occurrence").nth(3).waitFor();
  assert.equal(await panel().locator(".image-occurrence").count(), 4);
  assert.equal((await inspect(opened)).images.length, 4, "Stamp AP is not a page image target");
  await workspace().getByRole("button", { name: "Select source image 2, Img, page 1", exact: true }).click();
  assert.equal(await field("X (pt)").inputValue(), String(contract.images[1].placement[0]));
  assert.equal(await field("Y (pt)").inputValue(), String(contract.images[1].placement[1]));
  await chooseReplacement("replacement.png");
  const imageUrl = await panel().locator(".image-replacement img").getAttribute("src");
  if (process.env.FOLIO_IMAGE_VISUALS === "0") {
    await page.setViewportSize({ width: 800, height: 900 });
    await workspace().getByRole("button", { name: "Review image", exact: true }).click();
    await panel().getByRole("button", { name: "Select on page", exact: true }).click();
    await panel().waitFor({ state: "hidden" });
    assert.equal(await workspace().locator(".notification.toast").count(), 0, "Selection feedback must not cover compact page controls");
    await page.screenshot({ path: join(artifacts, "compact-on-page-functional-regression.png") });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await workspace().getByRole("button", { name: "Review image", exact: true }).click();
    assert.equal(await panel().locator(".image-replacement img").getAttribute("src"), imageUrl);
    evidence.checks.push("Compact on-page functional regression: transient replacement feedback clears, page controls remain uncovered, draft/asset survive responsive panel changes");
  }
  assert.equal(await field("Width (pt)").inputValue(), "80");
  assert.equal(await field("Height (pt)").inputValue(), "60");
  await documentBytes(opened, false, /Apply or discard pending/);
  assert.equal(digest(await documentBytes(opened, true)), digest(source));

  await workspace().getByRole("button", { name: "Show document properties", exact: true }).click();
  await workspace().getByRole("button", { name: "Review image", exact: true }).click();
  assert.equal(await panel().locator(".image-replacement img").getAttribute("src"), imageUrl);
  await openFile(join(artifacts, "peer.pdf"));
  await page.getByRole("tab", { name: "peer.pdf", exact: true }).waitFor();
  const peerInfo = await activeInfo();
  await page.getByRole("tab", { name: "Image occurrences.pdf", exact: true }).click();
  await settled();
  assert.equal(await panel().locator(".image-replacement img").getAttribute("src"), imageUrl);
  await workspace().getByRole("textbox", { name: "Page number", exact: true }).fill("2");
  await workspace().getByRole("textbox", { name: "Page number", exact: true }).press("Enter");
  await panel().getByRole("heading", { name: "Images on page 2", exact: true }).waitFor();
  await panel().getByRole("button", { name: "Go to selected image", exact: true }).click();
  await panel().getByRole("heading", { name: "Images on page 1", exact: true }).waitFor();
  assert.equal(await panel().locator(".image-replacement img").getAttribute("src"), imageUrl);
  evidence.checks.push("Actual on-page source selection; replacement footprint default; replacement bytes survive panel/tab switches and page virtualization");

  await workspace().locator(".page-scroll").focus();
  await page.keyboard.press("Meta+s");
  await panel().getByRole("alert").filter({ hasText: /image draft before saving/ }).waitFor();
  assert.equal(downloads, 0);
  await page.keyboard.press("Meta+z");
  await panel().getByRole("alert").filter({ hasText: /image draft before undoing/ }).waitFor();
  await page.getByRole("button", { name: "Close Image occurrences.pdf", exact: true }).click();
  await panel().getByRole("alert").filter({ hasText: /before closing this PDF tab/ }).waitFor();
  assert.ok(await page.getByRole("tab", { name: "Image occurrences.pdf", exact: true }).count());
  await api("/extract", { ...ref(opened), pages: [0] }, /Apply or discard pending/);
  await api("/output/begin", { ...ref(opened), kind: "png", pages: [0], dpi: 96, quality: null, allowUnencrypted: false }, /Apply or discard pending/);
  await api("/ocr/begin", { ...ref(opened), options: { pages: [0], dpi: 150, language: "eng", existingText: "force" } }, /Apply or discard pending/);
  await api("/signatures/export", { ...ref(opened), identity: [], password: "", fieldId: null }, /Apply or discard pending/);
  await api("/comparison/begin", { before: ref(opened), after: ref(peerInfo), settings: { dpi: 96, tolerance: 0 } }, /Apply or discard pending/);
  assert.equal((await activeInfo()).revision, opened.revision);
  evidence.checks.push("Pending replacement blocks source Save/history/close, raw PDF output/extraction/print pipeline, OCR, signing and comparison without exporting older content");

  await field("X (pt)").fill("170");
  await field("Y (pt)").fill("570");
  await field("Width (pt)").fill("70");
  await field("Height (pt)").fill("40");
  await panel().getByRole("button", { name: "Apply image changes", exact: true }).click();
  await settled();
  const replacedInfo = await activeInfo();
  assert.equal(replacedInfo.revision, opened.revision + 1);
  const replacedImages = (await inspect(replacedInfo)).images;
  assert.deepEqual(replacedImages[1].placement, [170, 570, 70, 40]);
  assert.deepEqual(replacedImages.filter((_, at) => at !== 1).map((item) => item.placement),
    contract.images.filter((_, at) => at !== 1).map((item) => item.placement));
  const uiReplaced = await documentBytes(replacedInfo);
  await writeFile(join(artifacts, "ui-replaced.pdf"), uiReplaced);
  await page.waitForFunction(() => window.__imageUrls.size === 0);

  await panel().locator(".image-occurrence").nth(0).click();
  const targetImage = workspace().getByRole("button", { name: "Select source image 1, Img, page 1", exact: true });
  let box = await targetImage.boundingBox();
  let scale = box.width / 80;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 10 * scale, box.y + box.height / 2 - 10 * scale);
  await page.waitForFunction(() => document.querySelector(".document-workspace:not([hidden]) .statusbar")?.textContent.includes("Uncommitted writing or area"));
  await documentBytes(replacedInfo, false, /Apply or discard pending/);
  await page.keyboard.press("Meta+p");
  await page.mouse.up();
  near(Number(await field("X (pt)").inputValue()), 70, "Drag X");
  near(Number(await field("Y (pt)").inputValue()), 570, "Drag Y");
  await workspace().getByRole("button", { name: "Review image", exact: true }).waitFor();
  await workspace().locator(".paper-loading").first().waitFor({ state: "hidden" });
  scale = (await targetImage.boundingBox()).width / 80;
  const handle = workspace().getByRole("button", { name: "Resize selected source image", exact: true });
  box = await handle.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 - 10 * scale, box.y + box.height / 2 - 10 * scale);
  await page.mouse.up();
  near(Number(await field("Width (pt)").inputValue()), 70, "Resize width");
  near(Number(await field("Height (pt)").inputValue()), 50, "Resize height");
  await targetImage.focus();
  await page.keyboard.press("ArrowRight");
  near(Number(await field("X (pt)").inputValue()), 71, "Keyboard X");
  await page.keyboard.press("ArrowLeft");
  near(Number(await field("X (pt)").inputValue()), 70, "Keyboard reverse X");
  await field("X (pt)").fill("70");
  await field("Y (pt)").fill("570");
  await field("Width (pt)").fill("70");
  await field("Height (pt)").fill("50");
  await panel().getByRole("button", { name: "Apply image changes", exact: true }).click();
  await settled();
  const movedInfo = await activeInfo(), uiMoved = await documentBytes(movedInfo);
  assert.deepEqual((await inspect(movedInfo)).images[0].placement, [70, 570, 70, 50]);
  await writeFile(join(artifacts, "ui-moved.pdf"), uiMoved);
  await workspace().getByRole("button", { name: "Undo (Command+Z)", exact: true }).click();
  await settled();
  assert.equal(digest(await documentBytes(await activeInfo())), digest(uiReplaced));
  await workspace().getByRole("button", { name: "Redo (Command+Shift+Z)", exact: true }).click();
  await settled();
  assert.equal(digest(await documentBytes(await activeInfo())), digest(uiMoved));
  evidence.checks.push("Live move and corner-resize gestures protect outputs before pointer-up; keyboard movement and numeric alternatives; one actual PDF mutation; byte-exact undo/redo");

  await panel().locator(".image-occurrence").nth(1).click();
  await panel().getByRole("button", { name: "Mark selected occurrence for removal", exact: true }).click();
  await panel().getByRole("button", { name: "Remove image occurrence", exact: true }).click();
  await settled();
  const removedInfo = await activeInfo();
  assert.equal((await inspect(removedInfo)).images.length, 3);
  assert.equal((await inspect(removedInfo, 1)).images.length, 4);
  const [download] = await Promise.all([
    page.waitForEvent("download"), workspace().getByRole("button", { name: "Download PDF", exact: true }).click(),
  ]);
  const downloaded = join(artifacts, "ui-saved-reopened.pdf");
  await download.saveAs(downloaded);
  const saved = new Uint8Array(await readFile(downloaded));
  assert.equal(digest(saved), digest(await documentBytes(removedInfo)));
  assert.equal((await PDFDocument.load(saved)).getPageCount(), 2);
  await openFile(downloaded);
  await page.getByRole("tab", { name: "ui-saved-reopened.pdf", exact: true }).waitFor();
  await settled();
  await workspace().getByRole("button", { name: "Edit existing images", exact: true }).click();
  await panel().locator(".image-occurrence").nth(2).waitFor();
  assert.equal(await panel().locator(".image-occurrence").count(), 3);
  evidence.checks.push("Actual source removal affects one invocation; ordinary save/download and UI reopen retain three page-1 images and all four shared page-2 images");

  const reopenedInfo = await activeInfo(), reopenedBytes = await documentBytes(reopenedInfo);
  await panel().locator(".image-occurrence").nth(0).click();
  await chooseReplacement("oriented.jpg");
  await panel().locator(".image-replacement").filter({ hasText: /2 × 3 px · unapplied/ }).waitFor();
  await field("X (pt)").fill("0");
  await field("Y (pt)").fill("0");
  await panel().getByRole("button", { name: "Apply image changes", exact: true }).click();
  await panel().getByRole("alert").filter({ hasText: /Form BBox/ }).waitFor();
  assert.equal((await activeInfo()).revision, reopenedInfo.revision);
  assert.equal(digest(await documentBytes(reopenedInfo, true)), digest(reopenedBytes));
  assert.ok(await panel().locator(".image-replacement img").getAttribute("src"));
  await field("Width (pt)").fill("");
  assert.equal(await panel().getByRole("button", { name: "Apply image changes", exact: true }).isDisabled(), true);
  await panel().getByRole("button", { name: "Discard image draft", exact: true }).click();
  await workspace().locator(".document-busy").waitFor({ state: "hidden" });
  await page.waitForFunction(() => window.__imageUrls.size === 0);
  assert.equal(digest(await documentBytes(reopenedInfo)), digest(reopenedBytes));
  await panel().locator(".image-occurrence").nth(0).click();
  const geometryDraftAck = draftAck(reopenedInfo.id, true);
  await field("X (pt)").fill("75");
  await geometryDraftAck;
  await documentBytes(reopenedInfo, false, /Apply or discard pending/);
  await panel().getByRole("button", { name: "Discard image draft", exact: true }).click();
  await workspace().locator(".document-busy").waitFor({ state: "hidden" });
  assert.equal(digest(await documentBytes(reopenedInfo)), digest(reopenedBytes));
  evidence.checks.push("EXIF-oriented JPEG stage; atomic Form-clip failure preserves replacement and coordinates; invalid numeric drafts and geometry-only drafts guard outputs; explicit discard releases assets/URLs without changing bytes");

  const unsupported = await PDFDocument.create(), unsupportedPage = unsupported.addPage([600, 770]);
  const raster = await unsupported.embedPng(await readFile(join(artifacts, "replacement.png")));
  const resources = unsupported.context.obj({ XObject: { I: raster.ref }, ExtGState: { A: { ca: 0.5 } } });
  unsupportedPage.node.set(PDFName.of("Resources"), resources);
  unsupportedPage.node.set(PDFName.of("Contents"), unsupported.context.register(unsupported.context.flateStream(
    "q 0 0 160 300 re W n 80 0 0 60 80 140 cm /I Do Q q 80 10 0 60 220 140 cm /I Do Q q /A gs 80 0 0 60 360 140 cm /I Do Q",
  )));
  const empty = unsupported.addPage([600, 770]), font = await unsupported.embedFont(StandardFonts.Helvetica);
  empty.drawText("No source raster images on this page", { font, x: 40, y: 700 });
  const unsupportedBytes = await unsupported.save();
  await writeFile(join(artifacts, "unsupported-and-empty.pdf"), unsupportedBytes);
  await openFile(join(artifacts, "unsupported-and-empty.pdf"));
  await page.getByRole("tab", { name: "unsupported-and-empty.pdf", exact: true }).waitFor();
  await settled();
  await workspace().getByRole("button", { name: "Edit existing images", exact: true }).click();
  await panel().locator(".image-occurrence").nth(2).waitFor();
  assert.equal((await inspect(await activeInfo())).images.filter((image) => image.editable).length, 0);
  await panel().locator(".image-occurrence").nth(0).click();
  assert.equal(await panel().getByRole("button", { name: "Replace with PNG or JPEG", exact: true }).isDisabled(), true);
  await workspace().getByRole("textbox", { name: "Page number", exact: true }).fill("2");
  await workspace().getByRole("textbox", { name: "Page number", exact: true }).press("Enter");
  await panel().getByText(/No Image-XObject occurrences/).waitFor();
  evidence.checks.push("Real engine read-only support reasons for clipped, skewed and translucent source images; meaningful empty-page UI");

  const renderInput = {};
  for (const name of ["source", "replaced", "removed", "moved", "rotations-source", "rotations-moved", "ui-replaced", "ui-moved", "ui-saved-reopened"]) {
    renderInput[name] = Array.from(await readFile(join(artifacts, `${name}.pdf`)));
  }
  const renderEvidence = await page.evaluate(async ({ files, contract }) => {
    const pdfjs = await import("/node_modules/pdfjs-dist/legacy/build/pdf.mjs");
    pdfjs.GlobalWorkerOptions.workerSrc = "/node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs";
    const output = { comparisons: [], rotations: [], pngs: {}, texts: {} };
    const options = { standardFontDataUrl: `${location.origin}/vendor/pdfjs/standard_fonts/`,
      cMapUrl: `${location.origin}/vendor/pdfjs/cmaps/`, cMapPacked: true,
      wasmUrl: `${location.origin}/vendor/pdfjs/wasm/`, useSystemFonts: false, enableXfa: false };
    const render = async (bytes, name) => {
      const task = pdfjs.getDocument({ ...options, data: new Uint8Array(bytes) });
      const pdf = await task.promise, pages = [];
      try {
        for (let index = 0; index < pdf.numPages; index++) {
          const page = await pdf.getPage(index + 1), viewport = page.getViewport({ scale: 1 });
          const canvas = document.createElement("canvas");
          canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
          await page.render({ canvas, viewport, background: "#ffffff" }).promise;
          const data = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
          const text = (await page.getTextContent()).items.filter((item) => "str" in item).map((item) => item.str).join(" ");
          pages.push({ width: canvas.width, height: canvas.height, data, text, rotation: page.rotate, unit: page.userUnit });
          if (index === 0 || name === "rotations-moved") output.pngs[`${name}-page-${index + 1}.png`] = canvas.toDataURL("image/png").split(",")[1];
          canvas.width = canvas.height = 0;
        }
        output.texts[name] = pages.map((page) => page.text);
        return pages;
      } finally { await task.destroy(); }
    };
    const original = await render(files.source, "source");
    const regions = {
      replaced: [contract.images[1].placement], removed: [contract.images[1].placement],
      moved: [contract.images[0].placement, contract.movedPlacement],
      "ui-replaced": [contract.images[1].placement, [170, 570, 70, 40]],
      "ui-moved": [contract.images[0].placement, [70, 570, 70, 50], contract.images[1].placement, [170, 570, 70, 40]],
      "ui-saved-reopened": [contract.images[0].placement, [70, 570, 70, 50], contract.images[1].placement],
    };
    for (const [name, allowed] of Object.entries(regions)) {
      const after = await render(files[name], name);
      for (let page = 0; page < original.length; page++) {
        const before = original[page], current = after[page];
        if (before.width !== current.width || before.height !== current.height) throw new Error(`Page dimensions changed: ${name}`);
        let changed = 0, outside = 0;
        for (let y = 0; y < before.height; y++) for (let x = 0; x < before.width; x++) {
          const offset = (y * before.width + x) * 4;
          if (![0, 1, 2, 3].some((channel) => before.data[offset + channel] !== current.data[offset + channel])) continue;
          changed++;
          if (page !== 0 || !allowed.some(([left, top, width, height]) => x >= left - 1 && x <= left + width + 1 && y >= top - 1 && y <= top + height + 1)) outside++;
        }
        output.comparisons.push({ name, page: page + 1, changed, outside });
      }
    }
    const rotations = await render(files["rotations-moved"], "rotations-moved");
    for (let page = 0; page < rotations.length; page++) {
      const rendered = rotations[page];
      let left = Infinity, top = Infinity, right = -Infinity, bottom = -Infinity, pixels = 0;
      for (let y = 0; y < rendered.height; y++) for (let x = 0; x < rendered.width; x++) {
        const offset = (y * rendered.width + x) * 4;
        if (Math.min(rendered.data[offset], rendered.data[offset + 1], rendered.data[offset + 2]) >= 240) continue;
        left = Math.min(left, x); top = Math.min(top, y); right = Math.max(right, x + 1); bottom = Math.max(bottom, y + 1); pixels++;
      }
      output.rotations.push({ page: page + 1, rotation: rendered.rotation, userUnit: rendered.unit,
        width: rendered.width, height: rendered.height, bounds: [left, top, right - left, bottom - top], pixels });
    }
    return output;
  }, { files: renderInput, contract });
  for (const result of renderEvidence.comparisons) {
    assert.equal(result.outside, 0, `Unselected pixels changed: ${JSON.stringify(result)}`);
    assert.equal(result.page === 1 ? result.changed > 0 : result.changed === 0, true, JSON.stringify(result));
  }
  for (const result of renderEvidence.rotations) {
    for (const [index, expected] of [40, 60, 125, 75].entries()) {
      assert.ok(Math.abs(result.bounds[index] - expected) <= 1, JSON.stringify(result));
    }
    assert.ok(result.pixels > 8000);
  }
  for (const [name, texts] of Object.entries(renderEvidence.texts).filter(([name]) => !name.startsWith("rotations"))) {
    assert.deepEqual(texts, renderEvidence.texts.source, `Source text changed: ${name}`);
  }
  for (const [name, png] of Object.entries(renderEvidence.pngs)) await writeFile(join(artifacts, name), Buffer.from(png, "base64"));
  delete renderEvidence.pngs;
  evidence.independentPdfJs = renderEvidence;
  evidence.checks.push("Independent PDF.js reopen/pixels: replacement/removal/movement only affect selected regions; exact RGBA equality on shared page 2 and all nonselected page-1/Stamp pixels; unchanged source text; all four rotations and UserUnit 0.75/1/1.5/2 physical placement");

  if (process.env.FOLIO_IMAGE_VISUALS !== "0") {
    await page.getByRole("tab", { name: "Image occurrences.pdf", exact: true }).click();
    await settled();
    await workspace().getByRole("button", { name: "All tools", exact: true }).click();
    await workspace().getByRole("button", { name: "Edit existing images", exact: true }).click();
    await panel().locator(".image-occurrence").nth(2).waitFor();
    await panel().locator(".image-occurrence").nth(0).click();
    await chooseReplacement("replacement.png");
    for (const [size, viewport] of [["desktop", { width: 1440, height: 1000 }], ["compact", { width: 800, height: 900 }]]) {
      await page.setViewportSize(viewport);
      await workspace().getByRole("button", { name: "Review image", exact: true }).click();
      for (const [label, resolved] of [["Light", "light"], ["Dark", "dark"], ["Sepia", "sepia"], ["High Contrast", "contrast"], ["System", "light"]]) {
        await page.getByRole("button", { name: "Appearance", exact: true }).click();
        await page.getByRole("radio", { name: label, exact: true }).check();
        await page.keyboard.press("Escape");
        await page.waitForFunction((theme) => document.documentElement.dataset.theme === theme, resolved);
        await field("X (pt)").focus();
        const metrics = await panel().evaluate((panel) => {
          const apply = panel.querySelector(".image-edit-commit .button"), input = panel.querySelector("input");
          const bounds = apply.getBoundingClientRect(), host = panel.getBoundingClientRect();
          const pairs = [...panel.querySelectorAll(".image-edit-intro p, .image-edit-settings > p, .setting-field input, .image-occurrence strong")].map((element) => {
            const style = getComputedStyle(element);
            let parent = element, background = style.backgroundColor;
            while ((background === "rgba(0, 0, 0, 0)" || background === "transparent") && parent.parentElement) {
              parent = parent.parentElement; background = getComputedStyle(parent).backgroundColor;
            }
            return { text: element.textContent.slice(0, 80), color: style.color, background };
          });
          return { footerVisible: bounds.top >= host.top && bounds.bottom <= Math.min(host.bottom, innerHeight) + 1,
            panelWidth: host.width, focusWidth: getComputedStyle(input).outlineWidth, pairs,
            reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches, horizontalOverflow: panel.scrollWidth > panel.clientWidth + 1 };
        });
        assert.equal(metrics.footerVisible, true, `${size}/${label} apply footer`);
        assert.equal(metrics.horizontalOverflow, false, `${size}/${label} panel overflow`);
        assert.equal(metrics.focusWidth, "2px");
        evidence.visuals.push({ size, theme: label, ...metrics });
        await page.screenshot({ path: join(artifacts, `${size}-${resolved}-${label.toLowerCase().replaceAll(" ", "-")}.png`) });
      }
    }
    await panel().getByRole("button", { name: "Select on page", exact: true }).click();
    await panel().waitFor({ state: "hidden" });
    await page.screenshot({ path: join(artifacts, "compact-on-page.png") });
    await workspace().getByRole("button", { name: "Review image", exact: true }).click();
    await panel().getByRole("button", { name: "Discard image draft", exact: true }).click();
    await page.waitForFunction(() => window.__imageUrls.size === 0);
    evidence.checks.push("One batched desktop/compact visual round across all five appearance settings, visible focused numeric fields and fixed Apply/Discard footer; compact Select on page/Review image workflow; reduced-motion enabled");
  }
  assert.deepEqual(evidence.externalRequests, []);
  assert.deepEqual(evidence.consoleErrors, []);
  evidence.passed = true;
  console.log(JSON.stringify({ passed: true, checks: evidence.checks, artifacts, ports: evidence.ports }, null, 2));
} catch (error) {
  evidence.passed = false;
  evidence.failure = error.stack ?? String(error);
  if (page) {
    await page.screenshot({ path: join(artifacts, "functional-failure.png") }).catch((error) => { evidence.failureScreenshotError = String(error); });
  }
  throw error;
} finally {
  if (browser) await browser.close();
  for (const child of children.toReversed()) await stop(child);
  evidence.helperProcesses = children.map((child) => ({ pid: child.pid, exitCode: child.exitCode, signalCode: child.signalCode }));
  await writeFile(join(artifacts, process.env.FOLIO_IMAGE_VISUALS === "0"
    ? "image-editing-functional-evidence.json" : "image-editing-evidence.json"), JSON.stringify(evidence, null, 2));
}
