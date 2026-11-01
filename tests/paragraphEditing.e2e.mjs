import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { PDFArray, PDFDict, PDFDocument, PDFName, PDFRawStream } from "pdf-lib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const artifacts = process.env.FOLIO_PARAGRAPH_EVIDENCE_DIR
  ? resolve(process.env.FOLIO_PARAGRAPH_EVIDENCE_DIR) : await mkdtemp(join(tmpdir(), "folio-paragraph-"));
const target = resolve(process.env.CARGO_TARGET_DIR ?? join(root, "target/paragraph-build"));
await mkdir(artifacts, { recursive: true });
const env = { ...process.env, CARGO_TARGET_DIR: target, CARGO_INCREMENTAL: "0",
  CARGO_PROFILE_DEV_DEBUG: "0", CARGO_PROFILE_TEST_DEBUG: "0", CARGO_BUILD_JOBS: "2",
  FOLIO_PARAGRAPH_EVIDENCE_DIR: artifacts };
const children = [];
const evidence = { artifacts, checks: [], visuals: [], draftWrites: [], consoleErrors: [], externalRequests: [] };
let browser, page;
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
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
    child.once("error", reject);
    child.once("exit", (code) => code === 0 ? resolve() : reject(new Error(`${command} (${code}): ${child.output}`)));
  });
}
async function freePort(preferred) {
  const socket = createServer();
  try { await new Promise((resolve, reject) => { socket.once("error", reject); socket.listen(preferred, "127.0.0.1", resolve); }); }
  catch (error) { socket.close(); if (error.code === "EADDRINUSE") return freePort(0); throw error; }
  const port = socket.address().port;
  await new Promise((resolve) => socket.close(resolve));
  return port;
}
async function ready(url, child) {
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    if (child.startError || child.exitCode !== null) throw new Error(child.startError?.message ?? child.output);
    try { if ((await fetch(url, { signal: AbortSignal.timeout(2000) })).ok) return; }
    catch (error) { if (Date.now() + 150 >= deadline) throw error; }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`The paragraph helper did not become responsive: ${child.output}`);
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null || child.startError) return;
  child.kill("SIGTERM");
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), new Promise((resolve) => setTimeout(resolve, 5000))]);
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
}
function preservedObjects(pdf) {
  const names = pdf.catalog.lookup(PDFName.of("Names"), PDFDict);
  const embedded = names.lookup(PDFName.of("EmbeddedFiles"), PDFDict).lookup(PDFName.of("Names"), PDFArray);
  const attachment = pdf.context.lookup(embedded.get(1), PDFDict).lookup(PDFName.of("EF"), PDFDict).lookup(PDFName.of("F"), PDFRawStream);
  const annotation = pdf.getPage(0).node.lookup(PDFName.of("Annots"), PDFArray);
  const appearance = pdf.context.lookup(annotation.get(0), PDFDict).lookup(PDFName.of("AP"), PDFDict).lookup(PDFName.of("N"), PDFRawStream);
  return { attachment: digest(attachment.contents), appearance: digest(appearance.contents), annotation: annotation.toString() };
}

try {
  if (process.env.FOLIO_PARAGRAPH_SKIP_BUILD !== "1") {
    await command("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--test", "paragraph", "--quiet"]);
    await command("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--quiet"]);
  }
  const frontendPort = await freePort(1436), backendPort = await freePort(19164);
  const ports = { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) };
  const engine = start(join(target, `debug/folio-preview${process.platform === "win32" ? ".exe" : ""}`), [], ports);
  await ready(`http://127.0.0.1:${backendPort}/health`, engine);
  const ui = start(process.execPath, ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--strictPort"], ports);
  const origin = `http://127.0.0.1:${frontendPort}`;
  await ready(origin, ui);
  evidence.ports = { frontendPort, backendPort, enginePid: engine.pid, uiPid: ui.pid };
  async function request(route, options = {}, expectedFailure) {
    const response = await fetch(`http://127.0.0.1:${backendPort}${route}`, {
      ...options, headers: { "X-Folio-Preview": "1", ...options.headers },
    });
    if (expectedFailure) {
      assert.equal(response.ok, false, `${route} must reject`);
      assert.match((await response.json()).error, expectedFailure, route);
    } else assert.ok(response.ok, `${route}: ${await response.clone().text()}`);
    return response;
  }
  const api = (route, input, failure) => request(route, input === undefined ? {} : {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(input),
  }, failure);
  const ref = (info) => ({ documentId: info.id, revision: info.revision });
  const snapshot = async () => (await (await api("/bootstrap")).json());
  const current = async (id) => (await snapshot()).documents.find((info) => info.id === id);
  const bytes = async (info, failure) => {
    const response = await request("/bytes", { headers: { "X-Document-Id": String(info.id), "X-Revision": String(info.revision) } }, failure);
    return failure ? null : new Uint8Array(await response.arrayBuffer());
  };
  const inspect = async (info) => (await (await api("/paragraphs", { ...ref(info), page: 0 })).json());
  const source = new Uint8Array(await readFile(join(artifacts, "source.pdf")));
  const open = async (data, name) => {
    const outcome = await (await request("/open", { method: "POST", headers: {
      "Content-Type": "application/pdf", "X-File-Name": encodeURIComponent(name) }, body: data })).json();
    assert.equal(outcome.status, "opened");
    return outcome.document;
  };
  await api("/bootstrap");
  const peer = await open(source, "Paragraph peer.pdf");
  const initial = await open(source, "Paragraph source.pdf");
  const inspected = await inspect(initial), block = inspected.blocks[0];
  assert.equal(block.sourceLines.length, 3);
  assert.equal(block.editable, true);
  evidence.source = { sha256: digest(source), block, epoch: inspected.epoch, preserved: preservedObjects(await PDFDocument.load(source)) };

  browser = await chromium.launch({ headless: true,
    ...(process.env.FOLIO_PARAGRAPH_BROWSER ? { executablePath: process.env.FOLIO_PARAGRAPH_BROWSER } : {}) });
  evidence.browserVersion = browser.version();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce", colorScheme: "light", acceptDownloads: true });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url()); return route.abort();
  });
  page = await context.newPage();
  page.setDefaultTimeout(20_000);
  page.on("pageerror", (error) => evidence.consoleErrors.push(error.message));
  page.on("request", (request) => { if (request.url().endsWith("/__folio/draft")) evidence.draftWrites.push(request.postDataJSON()); });
  await page.goto(origin);
  const workspace = () => page.locator(".document-workspace:not([hidden])");
  const panel = () => workspace().getByRole("complementary", { name: "Edit existing text", exact: true });
  const text = () => panel().getByRole("textbox", { name: "Paragraph text", exact: true });
  const field = (name) => panel().getByRole("spinbutton", { name, exact: true });
  const settled = async () => {
    await page.waitForFunction(() => {
      const tab = document.querySelector('[role="tab"][aria-selected="true"]');
      const workspace = tab && document.getElementById(tab.getAttribute("aria-controls"));
      return workspace && !workspace.hidden
        && workspace.querySelector('canvas[aria-label="Rendered PDF page 1"]')
        && !workspace.querySelector(".document-busy, .paper-loading")
        && !document.querySelector(".tab-action-status");
    });
  };
  const selectTab = async (name) => {
    await page.getByRole("tab", { name, exact: true }).click();
    await page.waitForFunction((name) => document.querySelector('[role="tab"][aria-selected="true"]')?.getAttribute("aria-label") === name, name);
    await settled();
  };
  const ack = (id, pending) => page.waitForResponse((response) => response.url().endsWith("/__folio/draft") && response.ok()
    && response.request().postDataJSON().documentId === id && response.request().postDataJSON().pending === pending);
  const readyLayout = async () => {
    await panel().locator(".paragraph-layout-ready").waitFor();
    await panel().locator('.paragraph-pdf-preview[aria-busy="false"]').waitFor();
  };
  const targetButton = () => workspace().getByRole("button", { name: "Select source paragraph 1, page 1", exact: true });
  const unloadBlocked = () => page.evaluate(() => {
    const event = new Event("beforeunload", { cancelable: true }); window.dispatchEvent(event); return event.defaultPrevented;
  });
  await settled();
  async function render(data, cacheKey, allowed) {
    return page.evaluate(async ({ data, cacheKey, allowed }) => {
      const { loadPdf } = await import("/src/lib/pdf.ts");
      const task = loadPdf(new Uint8Array(data));
      const pdf = await task.promise, result = [];
      window.__paragraphPixels ??= new Map();
      try {
        for (let index = 0; index < pdf.numPages; index++) {
          const page = await pdf.getPage(index + 1), viewport = page.getViewport({ scale: 1 });
          const canvas = document.createElement("canvas");
          canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
          await page.render({ canvas, viewport }).promise;
          const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
          const text = await page.getTextContent();
          const items = text.items.filter((item) => "str" in item).map((item) => ({
            text: item.str, baseline: viewport.convertToViewportPoint(item.transform[4], item.transform[5]), width: item.width * page.userUnit,
          }));
          const hash = [...new Uint8Array(await crypto.subtle.digest("SHA-256", pixels))].map((value) => value.toString(16).padStart(2, "0")).join("");
          let changed = 0, outside = 0;
          if (cacheKey) window.__paragraphPixels.set(`${cacheKey}:${index}`, { pixels: pixels.slice(), width: canvas.width, height: canvas.height });
          else {
            const before = window.__paragraphPixels.get(`source:${index}`);
            if (!before || before.width !== canvas.width || before.height !== canvas.height) throw new Error("Independent PDF.js page geometry changed.");
            for (let offset = 0; offset < pixels.length; offset += 4) {
              if (pixels[offset] === before.pixels[offset] && pixels[offset + 1] === before.pixels[offset + 1] && pixels[offset + 2] === before.pixels[offset + 2]) continue;
              changed++;
              const x = offset / 4 % canvas.width, y = Math.floor(offset / 4 / canvas.width);
              if (index !== 0 || !allowed || x < allowed[0] || y < allowed[1] || x >= allowed[0] + allowed[2] || y >= allowed[1] + allowed[3]) outside++;
            }
          }
          result.push({ index, width: canvas.width, height: canvas.height, hash, text: items.map((item) => item.text).join(" "), items, changed, outside,
            ...(index === 0 ? { png: canvas.toDataURL("image/png").split(",")[1] } : {}) });
          canvas.width = 0; canvas.height = 0;
        }
      } finally { await task.destroy(); }
      return result;
    }, { data: Array.from(data), cacheKey, allowed });
  }
  const baseline = await render(source, "source");
  await writeFile(join(artifacts, "original-page.png"), Buffer.from(baseline[0].png, "base64"));
  await workspace().getByRole("button", { name: "Write", exact: true }).click();
  assert.equal(await panel().getByRole("button", { name: "Paragraph reflow", exact: true }).getAttribute("aria-pressed"), "true");
  await panel().locator(".paragraph-source").first().click();
  await text().waitFor();
  const replacement = "A true source paragraph now wraps into measured lines. Café Ω Привет remain searchable. NEW-REFLOW-51973.\nAn explicit final line.";
  await Promise.all([ack(initial.id, true), text().fill(replacement)]);
  for (const [name, value] of [["X (pt)", "40"], ["Y (pt)", "60"], ["Width (pt)", "230"], ["Height (pt)", "140"]]) await field(name).fill(value);
  await readyLayout();
  assert.equal(await unloadBlocked(), true, "unapplied reflow must guard quitting");
  const pending = /pending|Apply or discard|unsaved/i;
  await bytes(initial, pending);
  for (const route of ["/undo", "/redo", "/preflight/inspect"]) await api(route, ref(initial), pending);
  await api("/close", { ...ref(initial), discardChanges: false }, pending);
  await api("/ocr/begin", { ...ref(initial), options: { pages: [0], dpi: 150, languages: ["eng"], layout: "horizontal_print", segmentation: "auto", existingText: "skip" } }, pending);
  await api("/comparison/begin", { before: ref(initial), after: ref(peer), settings: { dpi: 72, tolerance: 0 } }, pending);
  await api("/archive/begin", { ...ref(initial), pages: [0], dpi: 150, acknowledgeLoss: true, allowUnencrypted: false, acknowledgeUnsigned: false }, pending);
  await api("/signatures/export", { ...ref(initial), identity: [], password: "", fieldId: null }, pending);
  for (const kind of ["print", "png", "text", "docx", "optimize"]) {
    await api("/output/begin", { ...ref(initial), kind, pages: [0], dpi: ["print", "png"].includes(kind) ? 150 : null,
      quality: null, allowUnencrypted: false }, pending);
  }
  evidence.checks.push("Real backend aggregate guards: save bytes, undo/redo, close, OCR, comparison, signing, print/image/text/DOCX/optimization, preflight and archive.");
  await page.keyboard.press("Meta+s");
  assert.equal((await current(initial.id)).revision, initial.revision);
  await selectTab("Paragraph peer.pdf");
  assert.equal(await unloadBlocked(), true, "inactive paragraph draft must still guard quitting");
  assert.equal(await page.locator(".document-workspace[hidden] .paragraph-pdf-preview canvas").count(), 0, "inactive previews release their canvases");
  await bytes(initial, pending);
  await selectTab("Paragraph source.pdf");
  assert.equal(await text().inputValue(), replacement);
  assert.equal(await field("Width (pt)").inputValue(), "230");
  await workspace().getByRole("button", { name: "Show document properties", exact: true }).click();
  await workspace().getByRole("button", { name: "Review paragraph", exact: true }).click();
  assert.equal(await text().inputValue(), replacement);
  await text().fill("");
  await selectTab("Paragraph peer.pdf");
  await bytes(initial, pending);
  await selectTab("Paragraph source.pdf");
  assert.equal(await text().inputValue(), "", "cleared text is a persistent intentional draft");
  await field("X (pt)").fill("");
  await panel().getByText(/Enter finite X, Y, width and height/).waitFor();
  assert.equal(await field("X (pt)").inputValue(), "");
  await field("X (pt)").fill("40");
  await text().fill(replacement);
  await field("Height (pt)").fill("5");
  await panel().getByText(/Overflow - cannot apply/).waitFor();
  assert.equal(await panel().getByRole("button", { name: "Apply paragraph reflow", exact: true }).isEnabled(), false);
  await field("Height (pt)").fill("140"); await readyLayout();
  const hit = await targetButton().boundingBox();
  await page.mouse.move(hit.x + hit.width / 2, hit.y + hit.height / 2); await page.mouse.down();
  await page.waitForFunction(() => document.querySelector('[role="tab"][aria-selected="true"]')?.disabled === true);
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await selectTab("Paragraph peer.pdf");
  await bytes(initial, pending);
  assert.equal(evidence.draftWrites.filter((item) => item.documentId === initial.id).at(-1).pending, true, "overlay teardown cannot clear paragraph text/style drafts");
  await selectTab("Paragraph source.pdf");
  await Promise.all([ack(initial.id, false), panel().getByRole("button", { name: "Discard paragraph draft", exact: true }).click()]);
  await settled();
  await workspace().locator(".paragraph-draft-banner").waitFor({ state: "hidden" });
  assert.equal(digest(await bytes(initial)), digest(source));
  await page.waitForFunction(() => {
    const event = new Event("beforeunload", { cancelable: true }); window.dispatchEvent(event); return !event.defaultPrevented;
  });
  assert.equal(await unloadBlocked(), false);
  await targetButton().click();
  const selected = await targetButton().boundingBox();
  const held = ack(initial.id, true);
  await page.mouse.move(selected.x + selected.width / 2, selected.y + selected.height / 2); await page.mouse.down();
  await held; await bytes(initial, pending); await api("/undo", ref(initial), pending);
  const released = ack(initial.id, false); await page.mouse.up(); await released;
  assert.equal(digest(await bytes(initial)), digest(source), "a canceled/no-motion live gesture is not a PDF edit");
  evidence.checks.push("Draft text/style/cleared values survive tabs, panels and gesture teardown; live gestures alone guard history/output/quit; explicit discard preserves exact original bytes.");
  await text().fill(replacement);
  for (const [name, value] of [["X (pt)", "40"], ["Y (pt)", "60"], ["Width (pt)", "230"], ["Height (pt)", "140"]]) await field(name).fill(value);
  await readyLayout();
  await targetButton().focus(); await page.keyboard.press("ArrowRight");
  assert.equal(Number(await field("X (pt)").inputValue()), 41);
  await workspace().getByRole("button", { name: "Resize selected source paragraph", exact: true }).focus();
  await page.keyboard.press("ArrowDown");
  assert.equal(Number(await field("Height (pt)").inputValue()), 141);
  await readyLayout();
  const change = { epoch: inspected.epoch, page: 0, path: block.path, blockId: block.id, expected: block.text,
    text: replacement, placement: [41, 60, 230, 141], size: 12, leading: block.leading, alignment: "left", font: "noto" };
  const preview = await (await api("/paragraphs/preview", { ...ref(initial), change })).json();
  assert.equal(preview.layout.fits, true);
  await writeFile(join(artifacts, "actual-preview.pdf"), new Uint8Array(preview.renderPdf));
  await panel().getByRole("button", { name: "Apply paragraph reflow", exact: true }).click(); await settled();
  const applied = await current(initial.id), edited = await bytes(applied);
  await writeFile(join(artifacts, "ui-reflow.pdf"), edited);
  const allowed = [Math.min(block.placement[0], 41) - 2, Math.min(block.placement[1], 60) - 2,
    Math.max(block.placement[0] + block.placement[2], 271) - Math.min(block.placement[0], 41) + 4,
    Math.max(block.placement[1] + block.placement[3], 201) - Math.min(block.placement[1], 60) + 4];
  const rendered = await render(edited, null, allowed);
  assert.ok(rendered[0].changed > 100); assert.equal(rendered[0].outside, 0);
  assert.match(rendered[0].text, /NEW-REFLOW-\s*51973/, "PDF.js may report a soft break after an existing hyphen");
  assert.ok(!rendered[0].text.includes("Original first line"));
  assert.ok(!rendered[0].text.includes("with a second line"));
  assert.ok(rendered[0].text.includes("Independent right column")); assert.ok(rendered[0].text.includes("KEEP-FOOTER-90451"));
  for (let index = 1; index < rendered.length; index++) {
    assert.equal(rendered[index].changed, 0); assert.equal(rendered[index].text, baseline[index].text);
  }
  const lines = preview.layout.lines.filter((line) => line.text.trim());
  const actualLines = rendered[0].items.filter((item) => item.text.trim()).slice(0, lines.length);
  assert.equal(actualLines.length, lines.length);
  for (const [index, item] of actualLines.entries()) {
    assert.equal(item.text.trim(), lines[index].text.trim(), "Every actual PDF.js line must match the measured source layout without character loss");
    assert.ok(item.baseline[0] >= 40.99 && item.baseline[0] <= 271.01 && item.baseline[1] >= 59.99 && item.baseline[1] <= 201.01);
    assert.ok(item.width <= 230.03, "independent PDF.js advances fit the physical paragraph box");
  }
  assert.deepEqual(preservedObjects(await PDFDocument.load(edited)), evidence.source.preserved);
  await writeFile(join(artifacts, "reflow-page.png"), Buffer.from(rendered[0].png, "base64"));
  evidence.rendering = rendered.map(({ png, items, ...rest }) => rest);
  evidence.layout = preview.layout;
  evidence.checks.push("Independent real PDF.js Unicode extraction, measured physical line bounds and pixel diff: old source gone, real wrapping, zero changed pixels outside target and all other rotated/UserUnit pages; image/AP/attachment preserved.");
  const stable = (await inspect(applied)).blocks[0].id;
  await workspace().getByRole("button", { name: "Undo (Command+Z)", exact: true }).click(); await settled();
  const undone = await current(initial.id);
  assert.equal(digest(await bytes(undone)), digest(source));
  await api("/paragraphs/edit", { ...ref(undone), change }, /epoch/);
  await workspace().getByRole("button", { name: "Redo (Command+Shift+Z)", exact: true }).click(); await settled();
  assert.equal(digest(await bytes(await current(initial.id))), digest(edited));
  await page.getByLabel("Choose PDF files", { exact: true }).setInputFiles(join(artifacts, "ui-reflow.pdf"));
  await page.getByRole("tab", { name: "ui-reflow.pdf", exact: true }).waitFor();
  await page.waitForFunction(() => document.querySelector('[role="tab"][aria-selected="true"]')?.getAttribute("aria-label") === "ui-reflow.pdf");
  await settled();
  const reopened = (await snapshot()).documents.find((info) => info.name === "ui-reflow.pdf");
  assert.equal((await inspect(reopened)).blocks[0].id, stable);
  await workspace().getByRole("button", { name: "Write", exact: true }).click();
  await panel().locator(".paragraph-source").first().click();
  assert.equal(await text().inputValue(), replacement);
  await panel().getByRole("combobox", { name: "Paragraph font", exact: true }).selectOption("original");
  await text().fill("A short second edit. Café Ω Привет."); await readyLayout();
  await panel().getByRole("button", { name: "Apply paragraph reflow", exact: true }).click(); await settled();
  const second = await current(reopened.id);
  assert.equal((await inspect(second)).blocks[0].id, stable);
  const secondBytes = await bytes(second); await writeFile(join(artifacts, "second-original-font-edit.pdf"), secondBytes);
  const fonts = async (data) => (await PDFDocument.load(data)).context.enumerateIndirectObjects()
    .filter(([, value]) => value instanceof PDFDict && value.get(PDFName.of("Subtype"))?.toString() === "/Type0")
    .map(([ref]) => ref.toString());
  assert.deepEqual(await fonts(secondBytes), await fonts(edited), "Original mode reuses original encoded font resources");
  evidence.checks.push("UI Apply is one exact-byte undo/redo step; old epoch cannot resurrect; saved source reopens with stable logical identity and can be edited again using the original CID font resource.");

  if (process.env.FOLIO_PARAGRAPH_VISUALS !== "0") {
    await panel().locator(".paragraph-source").first().click();
    await panel().getByRole("combobox", { name: "Paragraph font", exact: true }).selectOption("noto");
    await text().fill(replacement);
    await field("Text size (pt)").fill("14"); await field("Leading (pt)").fill("19");
    await field("Width (pt)").fill("250"); await field("Height (pt)").fill("170"); await readyLayout();
    const capture = async (name) => {
      await page.screenshot({ path: join(artifacts, name) }); evidence.visuals.push(name);
      const state = await panel().getByRole("button", { name: "Apply paragraph reflow", exact: true }).evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        return { visible: bounds.top >= 0 && bounds.bottom <= innerHeight,
          unobstructed: element.contains(document.elementFromPoint(bounds.left + bounds.width / 2, bounds.top + bounds.height / 2)),
          bodyOverflow: document.documentElement.scrollWidth > innerWidth };
      });
      assert.equal(state.visible, true); assert.equal(state.unobstructed, true); assert.equal(state.bodyOverflow, false);
    };
    const chooseTheme = async (label) => {
      await page.getByRole("button", { name: "Appearance", exact: true }).click();
      await page.getByRole("radio", { name: label, exact: true }).check();
      await page.keyboard.press("Escape");
      await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    };
    await text().scrollIntoViewIfNeeded(); await capture("desktop-editor.png");
    await panel().locator(".paragraph-layout-review").scrollIntoViewIfNeeded(); await capture("desktop-layout.png");
    evidence.themes = [];
    for (const label of ["System", "Light", "Dark", "Sepia", "High Contrast"]) {
      await chooseTheme(label);
      evidence.themes.push(await page.evaluate(() => ({ theme: document.documentElement.dataset.theme,
        field: getComputedStyle(document.querySelector(".document-workspace:not([hidden]) .paragraph-text-field textarea")).color,
        background: getComputedStyle(document.querySelector(".document-workspace:not([hidden]) .paragraph-text-field textarea")).backgroundColor })));
      if (label === "Dark") await capture("desktop-dark-layout.png");
      if (label === "High Contrast") await capture("desktop-contrast-layout.png");
    }
    await chooseTheme("Light");
    await page.setViewportSize({ width: 800, height: 900 });
    await workspace().getByRole("button", { name: "Review paragraph", exact: true }).click(); await readyLayout();
    await text().scrollIntoViewIfNeeded(); await capture("compact-editor.png");
    await panel().locator(".paragraph-layout-review").scrollIntoViewIfNeeded(); await capture("compact-layout.png");
    await panel().getByRole("button", { name: "Select paragraph on page", exact: true }).click();
    assert.equal(await panel().count(), 0);
    await page.screenshot({ path: join(artifacts, "compact-on-page.png") }); evidence.visuals.push("compact-on-page.png");
    evidence.checks.push("One bounded desktop/compact visual batch with all five theme states, actual PDF preview, reduced motion and unobstructed fixed commit controls.");
  }
  assert.deepEqual(evidence.consoleErrors, []);
  assert.deepEqual(evidence.externalRequests, []);
  evidence.status = "passed";
  console.log(`Paragraph source/UI/PDF.js checks passed. Evidence: ${artifacts}`);
} catch (error) {
  evidence.status = "failed"; evidence.failure = error.stack ?? String(error);
  if (page) {
    evidence.failureState = await page.evaluate(() => ({
      tab: document.querySelector('[role="tab"][aria-selected="true"]')?.getAttribute("aria-label"),
      busy: [...document.querySelectorAll(".document-busy")].map((element) => ({ text: element.textContent, visible: Boolean(element.getClientRects().length) })),
      loading: document.querySelector(".document-workspace:not([hidden]) .paper-loading")?.textContent,
      errors: [...document.querySelectorAll('[role="alert"]')].filter((element) => element.getClientRects().length).map((element) => element.textContent),
    })).catch(() => null);
    await writeFile(join(artifacts, "failure.html"), await page.content().catch(() => ""));
  }
  throw error;
} finally {
  if (browser) await browser.close();
  for (const child of children.reverse()) await stop(child);
  evidence.helpersStopped = children.every((child) => child.exitCode !== null || child.signalCode !== null || child.startError);
  await writeFile(join(artifacts, "paragraph-evidence.json"), JSON.stringify(evidence, null, 2));
}
