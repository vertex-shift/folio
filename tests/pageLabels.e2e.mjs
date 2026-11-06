import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { PDFDocument, PDFName, StandardFonts } from "pdf-lib";
import { chromium } from "playwright";
import { createServer as createViteServer } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));
const output = process.env.FOLIO_PAGE_LABELS_EVIDENCE
  ? resolve(process.env.FOLIO_PAGE_LABELS_EVIDENCE) : await mkdtemp(join(tmpdir(), "folio-page-labels-"));
await mkdir(output, { recursive: true });
const target = resolve(process.env.CARGO_TARGET_DIR ?? join(root, "src-tauri/target"));
const children = [];
const evidence = { checks: [], screenshots: [], errors: [], externalRequests: [], draftWrites: [] };
let engine, browser, ui, page;
function start(command, args, extra = {}) {
  const child = spawn(command, args, { cwd: root, env: { ...process.env, ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  child.output = "";
  for (const stream of [child.stdout, child.stderr]) stream.on("data", (value) => { child.output = (child.output + value).slice(-12000); });
  children.push(child);
  return child;
}
async function complete(child) {
  await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code) => code === 0 ? resolve() : reject(new Error(child.output)));
  });
}
async function freePort() {
  const server = createServer();
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
try {
  if (process.env.FOLIO_PAGE_LABELS_SKIP_BUILD !== "1") {
    await complete(start("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml",
      "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--locked"], { CARGO_TARGET_DIR: target }));
  }
  const frontendPort = await freePort(), backendPort = await freePort();
  engine = start(join(target, `debug/folio-preview${process.platform === "win32" ? ".exe" : ""}`), [],
    { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("The owned label-test engine did not start.")), 15000);
    engine.once("error", reject);
    engine.stdout.on("data", () => { if (engine.output.includes("ready at")) { clearTimeout(timer); resolve(); } });
  });
  const origin = `http://127.0.0.1:${frontendPort}`;
  const api = async (path, value, failure) => {
    const response = await fetch(`http://127.0.0.1:${backendPort}${path}`, {
      headers: { "X-Folio-Preview": "1", ...(value === undefined ? {} : { "Content-Type": "application/json" }) },
      ...(value === undefined ? {} : { method: "POST", body: JSON.stringify(value) }),
    });
    if (failure) {
      assert.equal(response.ok, false, path);
      assert.match((await response.json()).error, failure);
    } else assert.equal(response.ok, true, `${path}: ${await response.clone().text()}`);
    return response;
  };
  const snapshot = async () => (await api("/bootstrap")).json();
  const current = async () => { const state = await snapshot(); return state.documents.find((doc) => doc.id === state.activeId); };
  const bytes = async (info) => {
    const response = await fetch(`http://127.0.0.1:${backendPort}/bytes`, {
      headers: { "X-Folio-Preview": "1", "X-Document-Id": String(info.id), "X-Revision": String(info.revision) },
    });
    assert.equal(response.ok, true);
    return Buffer.from(await response.arrayBuffer());
  };
  ui = await createViteServer({ root, cacheDir: join(output, "vite-cache"), logLevel: "error", server: {
    host: "127.0.0.1", port: frontendPort, strictPort: true,
    proxy: { "/__folio": { target: `http://127.0.0.1:${backendPort}`, rewrite: (path) => path.replace(/^\/__folio/, "") } },
  } });
  await ui.listen();
  browser = await chromium.launch({ headless: true,
    ...(process.env.FOLIO_PAGE_LABELS_BROWSER ? { executablePath: process.env.FOLIO_PAGE_LABELS_BROWSER } : {}) });
  const context = await browser.newContext({ reducedMotion: "reduce", viewport: { width: 1440, height: 1000 }, acceptDownloads: true });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url()); return route.abort();
  });
  page = await context.newPage();
  page.setDefaultTimeout(15000);
  page.on("pageerror", (error) => evidence.errors.push(error.message));
  page.on("request", (request) => {
    if (request.url().endsWith("/__folio/draft")) evidence.draftWrites.push(request.postDataJSON());
  });
  await page.goto(origin);
  await page.waitForFunction(() => document.querySelector('[aria-label="Open PDFs in new tabs"]:not(:disabled)'));
  const pdf = await PDFDocument.create(), font = await pdf.embedFont(StandardFonts.Helvetica);
  for (const title of ["Front one", "Front two", "Body one", "Body two"]) {
    pdf.addPage([420, 600]).drawText(title, { x: 40, y: 500, size: 20, font });
  }
  const source = Buffer.from(await pdf.save({ useObjectStreams: false }));
  const workspace = () => page.locator(".document-workspace:not([hidden])");
  const upload = async (name, data, password = null) => {
    await page.waitForFunction(() => {
      const active = document.querySelector(".document-workspace:not([hidden])");
      return document.querySelector('[aria-label="Open PDFs in new tabs"]:not(:disabled)')
        && active?.querySelector('canvas[aria-label^="Rendered PDF page "]')
        && !active.querySelector(".document-busy, .paper-loading");
    });
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await page.getByLabel("Choose PDF files", { exact: true }).setInputFiles({ name, mimeType: "application/pdf", buffer: Buffer.from(data) });
    if (password) {
      await page.getByLabel("PDF password", { exact: true }).fill(password);
      await page.getByRole("button", { name: "Unlock PDF", exact: true }).click();
    }
    await page.waitForFunction((name) => document.querySelector(".document-tab.selected")?.textContent.includes(name), name);
    await page.waitForFunction(() => {
      const active = document.querySelector(".document-workspace:not([hidden])");
      return active?.querySelector('canvas[aria-label="Rendered PDF page 1"]')
        && !active.querySelector(".document-busy, .paper-loading");
    });
  };
  await upload("Label source.pdf", source);
  const initial = await current();
  const openEditor = async () => {
    if (!await workspace().getByRole("button", { name: "PDF page labels", exact: true }).isVisible()) {
      await workspace().getByRole("button", { name: "All tools", exact: true }).click();
    }
    await workspace().getByRole("button", { name: "PDF page labels", exact: true }).click();
    await page.getByRole("dialog", { name: "PDF page labels", exact: true }).waitFor();
  };
  const editor = () => page.getByRole("dialog", { name: "PDF page labels", exact: true });
  const applyLabels = async () => {
    const id = (await current()).id;
    const cleared = page.waitForResponse((response) => response.url().endsWith("/__folio/draft")
      && response.request().postDataJSON().documentId === id && response.request().postDataJSON().pending === false);
    await editor().getByRole("button", { name: "Apply labels", exact: true }).click();
    await page.waitForFunction(() => !document.querySelector('[aria-modal="true"]'));
    await cleared;
  };
  await openEditor();
  const draftStarted = page.waitForResponse((response) => response.url().endsWith("/__folio/draft") && response.request().postDataJSON().pending === true);
  await editor().getByLabel("Section 1 style", { exact: true }).selectOption("roman_lower");
  await draftStarted;
  await api("/ocr/begin", { documentId: initial.id, revision: initial.revision,
    options: { pages: [0], dpi: 150, languages: ["eng"], layout: "horizontal_print", segmentation: "auto", existingText: "skip" } }, /pending|draft|Apply or discard/i);
  const draftCleared = page.waitForResponse((response) => response.url().endsWith("/__folio/draft") && response.request().postDataJSON().pending === false);
  await editor().getByRole("button", { name: "Cancel", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('[aria-modal="true"]'));
  await draftCleared;
  assert.equal(hash(await bytes(await current())), hash(source));
  evidence.checks.push("Canceling a page-label draft preserves exact source bytes and clears aggregate draft protection.");
  await openEditor();
  await editor().getByLabel("Section 1 style", { exact: true }).selectOption("roman_lower");
  await editor().getByRole("button", { name: "Add numbering section", exact: true }).click();
  await editor().getByLabel("Section 2 page", { exact: true }).fill("3");
  await applyLabels();
  let labeled = await current();
  assert.deepEqual(labeled.pageLabels.labels, ["i", "ii", "1", "2"]);
  await workspace().getByLabel("Page label", { exact: true }).fill("1");
  await workspace().getByLabel("Page label", { exact: true }).press("Enter");
  await page.waitForFunction(() => document.querySelector(".document-workspace:not([hidden]) .page-number span")?.textContent === "3 / 4");
  await workspace().getByLabel("Page navigation mode", { exact: true }).selectOption("position");
  await workspace().getByLabel("Page number", { exact: true }).fill("1");
  await workspace().getByLabel("Page number", { exact: true }).press("Enter");
  await workspace().getByLabel("Page navigation mode", { exact: true }).selectOption("label");
  assert.equal(await workspace().getByLabel("Page label", { exact: true }).inputValue(), "i");
  const oracle = async (data, original) => page.evaluate(async ({ data, original }) => {
    const { loadPdf } = await import("/src/lib/pdf.ts");
    const task = loadPdf(new Uint8Array(data)), beforeTask = original ? loadPdf(new Uint8Array(original)) : null;
    const pdf = await task.promise, before = beforeTask ? await beforeTask.promise : null;
    try {
      let changedChannels = 0;
      if (before) {
        for (let index = 1; index <= pdf.numPages; index++) {
          const pages = [await before.getPage(index), await pdf.getPage(index)];
          const viewport = pages[1].getViewport({ scale: 1 }), canvases = [];
          for (const page of pages) {
            const canvas = document.createElement("canvas");
            canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
            await page.render({ canvas, viewport }).promise; canvases.push(canvas);
          }
          const pixels = canvases.map((canvas) => canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data);
          for (let offset = 0; offset < pixels[0].length; offset++) if (pixels[0][offset] !== pixels[1][offset]) changedChannels++;
          for (const canvas of canvases) canvas.width = canvas.height = 0;
        }
      }
      return { labels: await pdf.getPageLabels(), changedChannels, count: pdf.numPages };
    } finally { await task.destroy(); if (beforeTask) await beforeTask.destroy(); }
  }, { data: Array.from(data), original: original ? Array.from(original) : null });
  const download = page.waitForEvent("download");
  await workspace().getByRole("button", { name: "Download PDF", exact: true }).click();
  const savedPath = join(output, "Actual-labeled.pdf");
  await (await download).saveAs(savedPath);
  const saved = await readFile(savedPath);
  assert.deepEqual(saved, await bytes(labeled));
  const inspection = await oracle(saved, source);
  assert.deepEqual(inspection.labels, ["i", "ii", "1", "2"]);
  assert.equal(inspection.changedChannels, 0);
  evidence.saved = inspection;
  evidence.checks.push("Actual saved PDF.js labels match Roman/decimal sections and every page retains identical pixels.");
  await upload("Label saved.pdf", saved);
  await workspace().getByRole("button", { name: "Organize", exact: true }).click();
  await workspace().getByRole("button", { name: "Select page 1 (physical page 3)", exact: true }).click();
  await workspace().getByRole("button", { name: "Move page earlier", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector(".document-workspace:not([hidden]) .document-busy"));
  await workspace().getByRole("button", { name: "Move page earlier", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector(".document-workspace:not([hidden]) .document-busy"));
  assert.deepEqual((await current()).pageLabels.labels, ["1", "i", "ii", "2"]);
  const extractEvent = page.waitForEvent("download");
  await workspace().getByRole("button", { name: "Extract pages", exact: true }).click();
  await page.getByRole("dialog").getByText(/Use physical page positions or ranges/).waitFor();
  await page.getByRole("dialog").getByLabel("Pages", { exact: true }).fill("4, 1");
  await page.getByRole("dialog").getByRole("button", { name: "Export pages", exact: true }).click();
  const extractedPath = join(output, "Actual-labeled-extraction.pdf");
  await (await extractEvent).saveAs(extractedPath);
  assert.deepEqual((await oracle(await readFile(extractedPath))).labels, ["2", "1"]);
  await workspace().getByRole("button", { name: "Duplicate", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector(".document-workspace:not([hidden]) .document-busy"));
  assert.deepEqual((await current()).pageLabels.labels, ["1", "1", "i", "ii", "2"]);
  evidence.checks.push("Move, extraction and duplication preserve original page-label associations.");
  await workspace().getByRole("button", { name: "Read", exact: true }).click();
  await workspace().getByLabel("Page navigation mode", { exact: true }).selectOption("position");
  await workspace().getByLabel("Page number", { exact: true }).fill("3");
  await workspace().getByLabel("Page number", { exact: true }).press("Enter");
  await workspace().getByLabel("Page navigation mode", { exact: true }).selectOption("label");
  await workspace().getByLabel("Page label", { exact: true }).fill("1");
  await workspace().getByLabel("Page label", { exact: true }).press("Enter");
  await workspace().getByText("This label belongs to more than one page. Choose Page mode and enter its physical position.", { exact: true }).waitFor();
  assert.equal(await workspace().getByLabel("Page label", { exact: true }).inputValue(), "i");
  await openEditor();
  await editor().getByRole("button", { name: "Reset to page positions", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('[aria-modal="true"]'));
  assert.equal((await current()).pageLabels, null);
  await openEditor();
  await editor().getByLabel("Section 1 prefix", { exact: true }).fill("#");
  await editor().getByLabel("Section 1 starting number", { exact: true }).fill("2");
  await applyLabels();
  await workspace().getByLabel("Page label", { exact: true }).fill("#2");
  await workspace().getByLabel("Page label", { exact: true }).press("Enter");
  await page.waitForFunction(() => document.querySelector(".document-workspace:not([hidden]) .page-number span")?.textContent === "1 / 5");
  assert.deepEqual((await current()).pageLabels.labels, ["#2", "#3", "#4", "#5", "#6"]);
  const beforeUnicode = await bytes(await current());
  await openEditor();
  await editor().getByLabel("Section 1 style", { exact: true }).selectOption("letters_upper");
  await editor().getByLabel("Section 1 starting number", { exact: true }).fill("26");
  await editor().getByLabel("Section 1 prefix", { exact: true }).fill("§-");
  for (const [theme, width, height] of [["light", 1440, 1000], ["dark", 980, 800], ["sepia", 820, 700], ["contrast", 1440, 1000]]) {
    await page.setViewportSize({ width, height });
    await page.evaluate((theme) => { document.documentElement.dataset.theme = theme; }, theme);
    await editor().getByRole("button", { name: "Apply labels", exact: true }).focus();
    const dialogBounds = await editor().boundingBox();
    assert.equal(dialogBounds.width, 560);
    const bounds = await editor().getByRole("button", { name: "Apply labels", exact: true }).boundingBox();
    assert.ok(bounds.y >= 0 && bounds.y + bounds.height <= height);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
    const filename = join(output, `labels-${theme}-${width}.png`);
    if (!process.env.FOLIO_PAGE_LABELS_SKIP_VISUAL) {
      await page.screenshot({ path: filename }); evidence.screenshots.push(filename);
    }
  }
  await applyLabels();
  assert.deepEqual((await current()).pageLabels.labels, ["§-Z", "§-AA", "§-BB", "§-CC", "§-DD"]);
  const unicodeInspection = await oracle(await bytes(await current()), beforeUnicode);
  assert.deepEqual(unicodeInspection.labels, ["§-Z", "§-AA", "§-BB", "§-CC", "§-DD"]);
  assert.equal(unicodeInspection.changedChannels, 0);
  evidence.navigationBounds = [];
  for (const [theme, width, height] of [["dark", 980, 800], ["sepia", 820, 700]]) {
    await page.setViewportSize({ width, height });
    await page.evaluate((theme) => { document.documentElement.dataset.theme = theme; }, theme);
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const geometry = await workspace().evaluate((node) => {
      const area = node.querySelector(".document-area").getBoundingClientRect();
      const controls = node.querySelector(".view-controls").getBoundingClientRect();
      const toast = node.querySelector(".toast")?.getBoundingClientRect();
      return { left: controls.left, right: controls.right, top: controls.top,
        areaLeft: area.left, areaRight: area.right, toastBottom: toast?.bottom ?? null };
    });
    assert.ok(geometry.left >= geometry.areaLeft && geometry.right <= geometry.areaRight);
    assert.notEqual(geometry.toastBottom, null, "Measure navigation while its actual success message is visible.");
    assert.ok(geometry.toastBottom < geometry.top,
      "Success feedback must not cover the page-label navigation controls.");
    evidence.navigationBounds.push({ theme, width, height, ...geometry });
    const filename = join(output, `label-navigation-${theme}-${width}.png`);
    if (!process.env.FOLIO_PAGE_LABELS_SKIP_VISUAL) {
      await page.screenshot({ path: filename }); evidence.screenshots.push(filename);
    }
  }
  const malformed = await PDFDocument.load(source);
  malformed.catalog.set(PDFName.of("PageLabels"), malformed.context.obj({ Nums: [1, { S: PDFName.of("D") }] }));
  await page.setViewportSize({ width: 1440, height: 1000 });
  await upload("Malformed labels.pdf", await malformed.save({ useObjectStreams: false }));
  await workspace().getByRole("status").filter({ hasText: "PDF page labels could not be read" }).waitFor();
  assert.equal((await current()).pageLabels.labels, null);
  await openEditor();
  await editor().getByLabel("Section 1 prefix", { exact: true }).fill("Fixed-");
  await applyLabels();
  assert.deepEqual((await current()).pageLabels.labels, ["Fixed-1", "Fixed-2", "Fixed-3", "Fixed-4"]);
  const unprotected = await current();
  const protectedResponse = await api("/protection/export", { documentId: unprotected.id, revision: unprotected.revision,
    authorizationPassword: null, protection: { mode: "protect", openingPassword: "reader", ownerPassword: "owner",
      permissions: { print: "high", copy: true, edit: true, annotate: true, fillForms: true, assemble: true } } });
  await upload("Protected labels.pdf", Buffer.from(await protectedResponse.arrayBuffer()), "reader");
  assert.equal((await current()).passwordProtected, true);
  await openEditor();
  await editor().getByLabel("Section 1 style", { exact: true }).selectOption("roman_lower");
  await editor().getByLabel("Section 1 prefix", { exact: true }).fill("Secure-");
  await applyLabels();
  const protectedInfo = await current();
  assert.deepEqual(protectedInfo.pageLabels.labels, ["Secure-i", "Secure-ii", "Secure-iii", "Secure-iv"]);
  const protectedDownload = page.waitForEvent("download");
  await workspace().getByRole("button", { name: "Download PDF", exact: true }).click();
  const protectedPath = join(output, "Actual-protected-labels.pdf");
  await (await protectedDownload).saveAs(protectedPath);
  const protectedSaved = await readFile(protectedPath);
  assert.deepEqual(protectedSaved, await bytes(protectedInfo));
  await upload("Protected labels saved.pdf", protectedSaved, "reader");
  assert.equal((await current()).passwordProtected, true);
  assert.deepEqual((await current()).pageLabels.labels, ["Secure-i", "Secure-ii", "Secure-iii", "Secure-iv"]);
  assert.deepEqual(evidence.errors, []);
  assert.deepEqual(evidence.externalRequests, []);
  evidence.checks.push("Actual UI label authoring/cancel/draft guards and exact saved/reopened PDF.js labels with zero changed page pixels.",
    "Roman front matter and physical page navigation remain distinct, including numeric and # prefixes.",
    "Reorder/extraction preserve original labels; duplicates are reported instead of silently selecting a page.",
    "Explicit reset and malformed-label repair remain metadata-only; compact dialogs keep actions visible.",
    "Actual protected UI save/reopen keeps the opening password and encrypted Session bytes.");
  await writeFile(join(output, "page-label-evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
  console.log(JSON.stringify({ passed: true, checks: evidence.checks, output }, null, 2));
} catch (error) {
  evidence.failure = error.stack ?? String(error);
  const diagnostics = [writeFile(join(output, "engine.log"), engine?.output ?? "")];
  if (page && !page.isClosed()) diagnostics.push(
    page.screenshot({ path: join(output, "failure.png"), timeout: 5000 }),
    page.content().then((html) => writeFile(join(output, "failure.html"), html)),
    page.locator("body").innerText({ timeout: 5000 }).then((text) => { evidence.lastPage = text.slice(0, 3500); }),
  );
  for (const result of await Promise.allSettled(diagnostics)) {
    if (result.status === "rejected") console.error("Could not capture failure evidence:", result.reason);
  }
  await writeFile(join(output, "page-label-failure.json"), JSON.stringify(evidence, null, 2) + "\n");
  console.error(JSON.stringify({ checks: evidence.checks, errors: evidence.errors, lastPage: evidence.lastPage }, null, 2));
  throw error;
} finally {
  if (browser) await browser.close();
  if (ui) await ui.close();
  for (const child of children.reverse()) {
    if (child.exitCode !== null || child.signalCode !== null) continue;
    child.kill("SIGTERM");
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), new Promise((resolve) => setTimeout(resolve, 3000))]);
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  }
}
