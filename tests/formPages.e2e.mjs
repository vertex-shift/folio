import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { degrees, PDFDocument, PDFName, StandardFonts } from "pdf-lib";
import { chromium } from "playwright";
import { createServer as createViteServer } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));
const output = process.env.FOLIO_FORM_PAGES_EVIDENCE
  ? resolve(process.env.FOLIO_FORM_PAGES_EVIDENCE) : await mkdtemp(join(tmpdir(), "folio-form-pages-"));
await mkdir(output, { recursive: true });
const target = resolve(process.env.CARGO_TARGET_DIR ?? join(root, "src-tauri/target"));
const children = [], evidence = { checks: [], errors: [], externalRequests: [] };
let browser, ui, page;
function start(command, args, extra = {}) {
  const child = spawn(command, args, { cwd: root, env: { ...process.env, ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  child.output = "";
  for (const stream of [child.stdout, child.stderr]) stream.on("data", (value) => { child.output = (child.output + value).slice(-16000); });
  children.push(child);
  return child;
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
  if (process.env.FOLIO_FORM_PAGES_SKIP_BUILD !== "1") {
    const build = start("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml",
      "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--locked"]);
    await new Promise((resolve, reject) => {
      build.once("error", reject);
      build.once("exit", (code) => code === 0 ? resolve() : reject(new Error(build.output)));
    });
  }
  const frontendPort = await freePort(), backendPort = await freePort();
  const engine = start(join(target, `debug/folio-preview${process.platform === "win32" ? ".exe" : ""}`), [],
    { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`Preview did not start: ${engine.output}`)), 15000);
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
  const current = async () => {
    const snapshot = await (await api("/bootstrap")).json();
    return snapshot.documents.find((doc) => doc.id === snapshot.activeId);
  };
  const draftAcknowledged = (id, pending) => page.waitForResponse((response) =>
    response.url().endsWith("/__folio/draft")
    && response.request().postDataJSON().documentId === id
    && response.request().postDataJSON().pending === pending && response.ok());
  const bytes = async () => {
    const info = await current();
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
    ...(process.env.FOLIO_FORM_PAGES_BROWSER ? { executablePath: process.env.FOLIO_FORM_PAGES_BROWSER } : {}) });
  const context = await browser.newContext({ reducedMotion: "reduce", viewport: { width: 1440, height: 1000 }, acceptDownloads: true });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url()); return route.abort();
  });
  page = await context.newPage();
  page.setDefaultTimeout(15000);
  page.on("pageerror", (error) => evidence.errors.push(error.message));
  await page.goto(origin);
  const workspace = () => page.locator(".document-workspace:not([hidden])");
  const settled = async () => {
    await page.waitForFunction(() => {
      const active = document.querySelector(".document-workspace:not([hidden])");
      return document.querySelector('[aria-label="Open PDFs in new tabs"]:not(:disabled)')
        && active && !active.querySelector(".document-busy, .paper-loading");
    });
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  };
  const upload = async (name, data) => {
    await workspace().getByRole("button", { name: "Read", exact: true }).click();
    await workspace().locator('canvas[aria-label^="Rendered PDF page "]').first().waitFor();
    await settled();
    await page.getByLabel("Choose PDF files", { exact: true }).setInputFiles({ name, mimeType: "application/pdf", buffer: data });
    await page.waitForFunction((name) => document.querySelector(".document-tab.selected")?.textContent.includes(name), name);
    await workspace().locator('canvas[aria-label="Rendered PDF page 1"]').waitFor();
    await settled();
  };
  const pdf = await PDFDocument.create(), font = await pdf.embedFont(StandardFonts.Helvetica);
  const pages = ["Front form", "Widget-free instructions", "Repeated form"].map((title) => {
    const page = pdf.addPage([612, 792]);
    page.drawText(title, { x: 50, y: 730, size: 20, font });
    page.setCropBox(10, 10, 592, 772);
    return page;
  });
  pages[2].setRotation(degrees(90));
  pages[2].node.set(PDFName.of("UserUnit"), pdf.context.obj(2));
  const form = pdf.getForm(), text = form.createTextField("person.name");
  for (const page of [pages[0], pages[2]]) text.addToPage(page, { x: 70, y: 600, width: 200, height: 30, font });
  text.setText("Preserved repeated value");
  const checkbox = form.createCheckBox("approved");
  checkbox.addToPage(pages[0], { x: 70, y: 550, width: 20, height: 20 }); checkbox.check();
  const radio = form.createRadioGroup("delivery");
  radio.addOptionToPage("Alpha", pages[0], { x: 120, y: 550, width: 20, height: 20 });
  radio.addOptionToPage("Beta", pages[2], { x: 120, y: 550, width: 20, height: 20 }); radio.select("Beta");
  const choice = form.createDropdown("country");
  choice.addOptions(["Alpha", "Beta"]); choice.select("Beta");
  choice.addToPage(pages[0], { x: 70, y: 480, width: 200, height: 30, font });
  form.updateFieldAppearances(font);
  pdf.catalog.set(PDFName.of("PageLabels"), pdf.context.obj({ Nums: [0, { S: "D", St: 1 }] }));
  const source = Buffer.from(await pdf.save({ useObjectStreams: false }));
  await writeFile(join(output, "source.pdf"), source);
  await upload("Form pages.pdf", source);
  const original = await current();
  const originalFields = await (await api("/forms", { documentId: original.id, revision: original.revision })).json();
  const render = async (data) => page.evaluate(async (data) => {
    const { loadPdf } = await import("/src/lib/pdf.ts");
    const task = loadPdf(new Uint8Array(data));
    try {
      const pdf = await task.promise, pages = [];
      for (let index = 1; index <= pdf.numPages; index++) {
        const page = await pdf.getPage(index), viewport = page.getViewport({ scale: 0.75 });
        const canvas = document.createElement("canvas");
        canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
        await page.render({ canvas, viewport, annotationMode: 2 }).promise;
        const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
        const hash = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", pixels))).map((b) => b.toString(16).padStart(2, "0")).join("");
        pages.push({ width: canvas.width, height: canvas.height, hash });
        canvas.width = canvas.height = 0;
      }
      return { pages, labels: await pdf.getPageLabels() };
    } finally { await task.destroy(); }
  }, Array.from(data));
  const originalPixels = await render(source);
  await workspace().getByRole("button", { name: "Organize", exact: true }).click();
  await workspace().getByRole("button", { name: "Crop pages", exact: true }).click();
  await workspace().getByLabel("Top margin (pt)", { exact: true }).fill("10");
  const appliedDraft = draftAcknowledged(original.id, false);
  await workspace().getByRole("button", { name: "Apply crop", exact: true }).click();
  await appliedDraft;
  await settled();
  const cropped = await bytes();
  assert.notEqual(hash(cropped), hash(source));
  await workspace().getByRole("button", { name: /Undo \(/ }).click(); await settled();
  assert.equal(hash(await bytes()), hash(source));
  await workspace().getByRole("button", { name: /Redo \(/ }).click(); await settled();
  assert.equal(hash(await bytes()), hash(cropped));
  await workspace().getByRole("button", { name: "Reset original crop", exact: true }).click(); await settled();
  assert.deepEqual(await render(await bytes()), originalPixels);
  const unchanged = await current(), unchangedBytes = await bytes();
  const failedDraft = draftAcknowledged(unchanged.id, true);
  await workspace().getByLabel("Top margin (pt)", { exact: true }).fill("200");
  await failedDraft;
  await workspace().getByRole("button", { name: "Apply crop", exact: true }).click();
  await workspace().getByRole("alert").filter({ hasText: /widget.*visible crop/i }).waitFor();
  assert.equal((await current()).revision, unchanged.revision);
  await api("/undo", { documentId: unchanged.id, revision: unchanged.revision }, /pending|draft|Apply or discard/i);
  const discardedDraft = draftAcknowledged(unchanged.id, false);
  await workspace().getByRole("button", { name: "Discard authoring draft", exact: true }).click(); await settled();
  await discardedDraft;
  assert.deepEqual(await bytes(), unchangedBytes);
  evidence.checks.push("UI crop, exact-byte undo/redo and reset preserve rendered appearances; clipping is refused and failed drafts remain protected.");
  await workspace().getByRole("button", { name: "Organize", exact: true }).click();
  await workspace().getByRole("button", { name: "Remove selected page", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Remove page", exact: true }).click();
  await workspace().getByText(/Only pages without form widgets can be deleted/).waitFor();
  assert.deepEqual(await bytes(), unchangedBytes);
  await workspace().getByRole("button", { name: "Duplicate", exact: true }).click();
  await workspace().getByText(/does not yet support interactive forms or XFA/).waitFor();
  assert.deepEqual(await bytes(), unchangedBytes);
  await workspace().getByRole("button", { name: "Select page 2", exact: true }).click();
  await workspace().getByRole("button", { name: "Move page earlier", exact: true }).click(); await settled();
  await workspace().getByRole("button", { name: "Remove selected page", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Remove page", exact: true }).click(); await settled();
  assert.equal((await current()).pageCount, 2);
  const download = page.waitForEvent("download");
  await workspace().getByRole("button", { name: "Download PDF", exact: true }).click();
  const savedPath = join(output, "saved-form-pages.pdf");
  await (await download).saveAs(savedPath);
  const saved = await readFile(savedPath), savedPdf = await PDFDocument.load(saved);
  assert.equal(savedPdf.getForm().getTextField("person.name").getText(), text.getText());
  assert.equal(savedPdf.getForm().getCheckBox("approved").isChecked(), true);
  assert.equal(savedPdf.getForm().getRadioGroup("delivery").getSelected(), "Beta");
  assert.deepEqual(savedPdf.getForm().getDropdown("country").getSelected(), ["Beta"]);
  const pixels = await render(saved);
  assert.deepEqual(pixels.pages, [originalPixels.pages[0], originalPixels.pages[2]]);
  assert.deepEqual(pixels.labels, ["1", "3"]);
  await upload("Saved form pages.pdf", saved);
  const reopened = await current();
  const reopenedFields = await (await api("/forms", { documentId: reopened.id, revision: reopened.revision })).json();
  for (const before of originalFields.fields) {
    const after = reopenedFields.fields.find((field) => field.id === before.id);
    assert.equal(after.name, before.name); assert.deepEqual(after.value, before.value);
    assert.deepEqual(after.widgets, before.widgets.map((widget) => ({ ...widget, page: widget.page === 2 ? 1 : widget.page })));
  }
  evidence.checks.push("UI reorder and widget-free deletion preserve page labels, field/widget identities, repeated values and exact PDF.js pixels after actual download/reopen; widget deletion and duplication are refused atomically.");
  await page.screenshot({ path: join(output, "reopened.png"), fullPage: true });
  const field = reopenedFields.fields.find((field) => field.name === "person.name");
  const changed = await (await api("/forms/fill", { documentId: reopened.id, revision: reopened.revision,
    changes: [{ id: field.id, value: { type: "text", text: "Edited" } }] })).json();
  assert.ok(changed.revision > reopened.revision);
  const filled = await PDFDocument.load(await bytes());
  assert.equal(filled.getForm().getTextField("person.name").getText(), "Edited");
  evidence.checks.push("The reopened logical field remains fillable; both widgets stay interactive, not flattened.");
  assert.deepEqual(evidence.errors, []);
  assert.deepEqual(evidence.externalRequests, []);
  evidence.result = "passed";
} catch (error) {
  evidence.result = "failed"; evidence.failure = error.stack ?? String(error);
  if (page) await page.screenshot({ path: join(output, "failure.png"), fullPage: true }).catch(() => {});
  throw error;
} finally {
  await writeFile(join(output, "evidence.json"), JSON.stringify(evidence, null, 2));
  if (browser) await browser.close();
  if (ui) await ui.close();
  for (const child of children.reverse()) if (child.exitCode === null) child.kill();
  console.log(`Form page workflow ${evidence.result}: ${output}`);
}
