import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { degrees, PDFDict, PDFDocument, PDFName, PDFNumber } from "pdf-lib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const artifacts = process.env.FOLIO_FORMS_ARTIFACTS
  ? resolve(process.env.FOLIO_FORMS_ARTIFACTS) : await mkdtemp(join(tmpdir(), "folio-forms-"));
await mkdir(artifacts, { recursive: true });
const target = resolve(process.env.CARGO_TARGET_DIR ?? join(root, "src-tauri/target/form-designer"));
const env = { ...process.env, CARGO_TARGET_DIR: target, CARGO_INCREMENTAL: "0", CARGO_PROFILE_DEV_DEBUG: "0",
  CARGO_PROFILE_TEST_DEBUG: "0", CARGO_BUILD_JOBS: "2", FOLIO_FORM_EVIDENCE_DIR: artifacts };
const children = [];
let browser, page;
const evidence = { artifacts, externalRequests: [], consoleErrors: [], checks: [], visuals: [] };
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
    child.once("exit", (code) => code === 0 ? resolve() : reject(new Error(`${command}: ${child.output}`)));
  });
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
  const until = Date.now() + 90_000;
  while (Date.now() < until) {
    if (child.startError || child.exitCode !== null) throw new Error(child.startError?.message ?? child.output);
    try { if ((await fetch(url, { signal: AbortSignal.timeout(2000) })).ok) return; }
    catch (error) { if (Date.now() >= until) throw error; }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Preview not ready: ${child.output}`);
}
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
try {
  if (process.env.FOLIO_FORMS_SKIP_BUILD !== "1") {
    await command("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--test", "form_designer", "--quiet"]);
    await command("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--quiet"]);
  }
  const frontendPort = await port(1429), backendPort = await port(19157);
  evidence.ports = { frontendPort, backendPort };
  const ports = { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) };
  const backend = start(join(target, "debug/folio-preview") + (process.platform === "win32" ? ".exe" : ""), [], ports);
  await ready(`http://127.0.0.1:${backendPort}/health`, backend);
  const frontend = start(process.execPath, ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--strictPort"], ports);
  const origin = `http://127.0.0.1:${frontendPort}`;
  await ready(origin, frontend);
  async function api(route, value, extra = {}, success = true) {
    const response = await fetch(`http://127.0.0.1:${backendPort}${route}`, {
      headers: { "X-Folio-Preview": "1", ...(value ? { "Content-Type": "application/json" } : {}), ...extra },
      ...(value ? { method: "POST", body: JSON.stringify(value) } : {}),
    });
    if (success) assert.ok(response.ok, `${route}: ${await response.clone().text()}`);
    else assert.ok(!response.ok, `${route} must reject`);
    return response;
  }
  const workingBytes = async (info) => new Uint8Array(await (await api("/bytes", null, {
    "X-Document-Id": String(info.id), "X-Revision": String(info.revision),
  })).arrayBuffer());
  const active = async () => {
    const snapshot = await (await api("/bootstrap")).json();
    return snapshot.documents.find((document) => document.id === snapshot.activeId);
  };
  const authored = new Uint8Array(await readFile(join(artifacts, "authored.pdf")));
  const flattened = new Uint8Array(await readFile(join(artifacts, "static.pdf")));
  const contract = JSON.parse(await readFile(join(artifacts, "field-contract.json"), "utf8"));
  evidence.sourceSha256 = digest(authored);
  evidence.staticSha256 = digest(flattened);
  const editable = await PDFDocument.load(authored);
  assert.equal(editable.getForm().getFields().length, 9);
  assert.equal(editable.getForm().getTextField("evidence_name").getText(), "Zoë PDF");
  assert.ok(editable.getForm().getCheckBox("evidence_check").isChecked());
  assert.equal(editable.getForm().getRadioGroup("evidence_radio").getSelected(), "Beta");
  assert.deepEqual(editable.getForm().getOptionList("evidence_list").getSelected(), ["Alpha", "Beta"]);
  assert.deepEqual(editable.getForm().getDropdown("evidence_combo").getSelected(), ["Custom choice"]);
  assert.ok(editable.getForm().getDropdown("evidence_combo").acroField.getOptions().some((option) => option.value.decodeText() === ""));
  assert.equal(editable.getForm().getTextField("evidence_comb").getText(), "1234");
  assert.equal(editable.getForm().getTextField("evidence_multiline").getText().replace(/\r/g, "\n"), "Line one\nLine two");
  const staticPdf = await PDFDocument.load(flattened);
  assert.equal(staticPdf.catalog.has(PDFName.of("AcroForm")), false);
  for (const [, object] of staticPdf.context.enumerateIndirectObjects()) {
    if (!(object instanceof PDFDict)) continue;
    assert.notEqual(object.get(PDFName.of("Subtype"))?.toString(), "/Widget");
    assert.equal(object.has(PDFName.of("FT")), false);
    assert.equal(object.has(PDFName.of("DV")), false);
    assert.equal(object.has(PDFName.of("AA")), false);
  }
  evidence.checks.push("Independent pdf-lib reopening: field values, checked state, unsigned field, no static widgets/field dictionaries/actions");
  browser = await chromium.launch({ headless: true, executablePath: process.env.FOLIO_FORMS_BROWSER ?? chromium.executablePath() });
  evidence.browserVersion = browser.version();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce" });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url());
    return route.abort();
  });
  page = await context.newPage();
  page.on("pageerror", (error) => evidence.consoleErrors.push(error.message));
  const draftWrites = [];
  evidence.draftWrites = draftWrites;
  page.on("request", (request) => {
    if (request.url().endsWith("/__folio/draft")) draftWrites.push(request.postDataJSON());
  });
  await page.goto(origin);
  await page.getByRole("button", { name: "Design form fields", exact: true }).waitFor();
  const compareAppearances = async ({ authored, flattened, fields }) => {
    const pdfjs = await import("/node_modules/pdfjs-dist/legacy/build/pdf.mjs");
    pdfjs.GlobalWorkerOptions.workerSrc = "/node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs";
    const options = { standardFontDataUrl: `${location.origin}/vendor/pdfjs/standard_fonts/`,
      wasmUrl: `${location.origin}/vendor/pdfjs/wasm/`, cMapUrl: `${location.origin}/vendor/pdfjs/cmaps/`,
      cMapPacked: true, enableXfa: false, useSystemFonts: false };
    const firstTask = pdfjs.getDocument({ ...options, data: new Uint8Array(authored) });
    const secondTask = pdfjs.getDocument({ ...options, data: new Uint8Array(flattened) });
    const original = await firstTask.promise, copy = await secondTask.promise;
    try {
      const raw = await original.getFieldObjects();
      const entries = raw instanceof Map ? [...raw.entries()] : Object.entries(raw ?? {});
      const result = { fields: entries.map(([name, widgets]) => ({ name, widgets: widgets.map((widget) => ({
        id: widget.id, type: widget.type, value: widget.value, defaultValue: widget.defaultValue, rect: widget.rect,
      })) })), pages: [], staticText: [] };
      for (let index = 0; index < original.numPages; index++) {
        const sourcePage = await original.getPage(index + 1), staticPage = await copy.getPage(index + 1);
        const viewport = sourcePage.getViewport({ scale: 1 }), other = staticPage.getViewport({ scale: 1 });
        if (viewport.width !== other.width || viewport.height !== other.height) throw new Error("Static geometry changed");
        const canvases = [document.createElement("canvas"), document.createElement("canvas")];
        for (let at = 0; at < 2; at++) {
          canvases[at].width = Math.ceil(viewport.width); canvases[at].height = Math.ceil(viewport.height);
          await (at ? staticPage : sourcePage).render({ canvas: canvases[at], viewport: at ? other : viewport }).promise;
        }
        const before = canvases[0].getContext("2d").getImageData(0, 0, canvases[0].width, canvases[0].height).data;
        const after = canvases[1].getContext("2d").getImageData(0, 0, canvases[1].width, canvases[1].height).data;
        const regions = [];
        for (const field of fields) for (const widget of field.widgets.filter((widget) => widget.page === index)) {
          const a = viewport.convertToViewportPoint(widget.rect[0], widget.rect[1]), b = viewport.convertToViewportPoint(widget.rect[2], widget.rect[3]);
          const [left, top, right, bottom] = [Math.max(0, Math.floor(Math.min(a[0], b[0])) - 2), Math.max(0, Math.floor(Math.min(a[1], b[1])) - 2),
            Math.min(canvases[0].width, Math.ceil(Math.max(a[0], b[0])) + 2), Math.min(canvases[0].height, Math.ceil(Math.max(a[1], b[1])) + 2)];
          let pixels = 0, different = 0, totalDifference = 0, maxDifference = 0, beforeInk = 0, afterInk = 0;
          for (let y = top; y < bottom; y++) for (let x = left; x < right; x++) {
            const offset = (y * canvases[0].width + x) * 4;
            const difference = Math.max(...[0, 1, 2, 3].map((channel) => Math.abs(before[offset + channel] - after[offset + channel])));
            pixels++; if (difference > 4) different++; totalDifference += difference;
            maxDifference = Math.max(maxDifference, difference);
            if (Math.min(before[offset], before[offset + 1], before[offset + 2]) < 245) beforeInk++;
            if (Math.min(after[offset], after[offset + 1], after[offset + 2]) < 245) afterInk++;
          }
          regions.push({ field: field.name, pixels, different, maxDifference, meanDifference: totalDifference / pixels, beforeInk, afterInk });
        }
        const annotations = await staticPage.getAnnotations();
        if (annotations.some((annotation) => annotation.annotationType === 20)) throw new Error("Static widget remains");
        const text = (await staticPage.getTextContent()).items.filter((item) => "str" in item).map((item) => item.str).join(" ");
        result.staticText.push(text);
        result.pages.push({ page: index + 1, rotation: sourcePage.rotate, userUnit: sourcePage.userUnit,
          width: viewport.width, height: viewport.height, regions,
          originalPng: canvases[0].toDataURL("image/png").split(",")[1], staticPng: canvases[1].toDataURL("image/png").split(",")[1] });
      }
      return result;
    } finally { await firstTask.destroy(); await secondTask.destroy(); }
  };
  const reopening = await page.evaluate(compareAppearances, { authored: Array.from(authored), flattened: Array.from(flattened), fields: contract.fields });
  evidence.reopening = reopening;
  assert.equal(reopening.fields.length, 9, JSON.stringify(reopening.fields));
  assert.equal(reopening.fields.find((field) => field.name === "evidence_name").widgets.filter((widget) => widget.type === "text").length, 4);
  assert.ok(reopening.fields.find((field) => field.name === "evidence_unsigned").widgets.filter((widget) => widget.type === "signature").every((widget) => widget.value == null));
  for (const field of contract.fields) {
    const actual = reopening.fields.find((item) => item.name === field.name);
    assert.ok(actual.widgets.some((item) => item.id === `${field.id.split(":")[0]}R`));
    for (const widget of field.widgets) assert.ok(actual.widgets.some((item) => item.id === `${widget.id.split(":")[0]}R`));
  }
  for (const rendered of reopening.pages) {
    for (const region of rendered.regions) {
      assert.ok(region.beforeInk > 3 && region.afterInk > 3, `Visible pixels: ${region.field} page ${rendered.page}`);
      assert.ok(region.different <= Math.max(2, region.pixels * 0.005) && region.meanDifference <= 0.2,
        `Appearance mismatch ${region.field} page ${rendered.page}: ${JSON.stringify(region)}`);
    }
    await writeFile(join(artifacts, `original-page-${rendered.page}.png`), Buffer.from(rendered.originalPng, "base64"));
    await writeFile(join(artifacts, `static-page-${rendered.page}.png`), Buffer.from(rendered.staticPng, "base64"));
    delete rendered.originalPng; delete rendered.staticPng;
  }
  assert.ok(reopening.staticText.every((text) => text.includes("Zoë PDF")));
  assert.ok(!reopening.staticText.join(" ").includes("original-synthetic-password"));
  assert.ok(reopening.staticText[1].includes("*".repeat("original-synthetic-password".length)));
  evidence.reopening = reopening;
  evidence.checks.push("PDF.js independent reopening and real page rendering: all four rotations, offset CropBox, UserUnit 1.5/2, repeated fields, masked password and unsigned outline");
  const fractionalPdf = await PDFDocument.create(), fractionalForm = fractionalPdf.getForm();
  for (const [index, rotation] of [0, 90, 180, 270].entries()) {
    const target = fractionalPdf.addPage([500, 700]);
    target.setMediaBox(-40, 20, 500, 700); target.setCropBox(-20, 40, 440, 620);
    target.setRotation(degrees(rotation));
    target.node.set(PDFName.of("UserUnit"), PDFNumber.of([1.25, 1.5, 2.25, 0.75][index]));
    const text = fractionalForm.createTextField(`fractional_text_${index}`);
    text.setText("Original"); text.addToPage(target, { x: 40, y: 500, width: 180, height: 30, borderWidth: 1 });
    const choice = fractionalForm.createDropdown(`fractional_choice_${index}`);
    choice.addOptions(["Alpha", "Beta"]); choice.select("Alpha");
    choice.addToPage(target, { x: 40, y: 430, width: 180, height: 30, borderWidth: 1 });
    const check = fractionalForm.createCheckBox(`fractional_check_${index}`);
    check.addToPage(target, { x: 280, y: 500, width: 22, height: 22, borderWidth: 1 });
  }
  const fractionalSource = await fractionalPdf.save({ useObjectStreams: false });
  const opened = await fetch(`http://127.0.0.1:${backendPort}/open`, { method: "POST",
    headers: { "X-Folio-Preview": "1", "X-File-Name": "fractional-original-synthetic.pdf", "Content-Type": "application/pdf" }, body: fractionalSource });
  assert.ok(opened.ok, await opened.clone().text());
  const fractionalInfo = (await opened.json()).document;
  const fractionalFields = (await (await api("/forms", { documentId: fractionalInfo.id, revision: fractionalInfo.revision })).json()).fields;
  assert.ok(fractionalFields.some((field) => field.widgets.some((widget) => widget.rect.some((value) => value % 1 === 0.5))),
    "The regression must contain real half-unit PDF-lib widget edges");
  const filledInfo = await (await api("/forms/fill", { documentId: fractionalInfo.id, revision: fractionalInfo.revision,
    changes: fractionalFields.map((field) => ({ id: field.id, value: field.kind === "text"
      ? { type: "text", text: "Fractional field" } : field.kind === "choice"
        ? { type: "choice", indices: [1], custom: null } : { type: "button", state: field.options[0].id } })) })).json();
  const fractionalFilled = await workingBytes(filledInfo);
  const fractionalStatic = new Uint8Array(await (await api("/forms/flatten", { documentId: filledInfo.id, revision: filledInfo.revision })).arrayBuffer());
  await writeFile(join(artifacts, "fractional-filled.pdf"), fractionalFilled);
  await writeFile(join(artifacts, "fractional-static.pdf"), fractionalStatic);
  const fractionalComparison = await page.evaluate(compareAppearances, { authored: Array.from(fractionalFilled),
    flattened: Array.from(fractionalStatic), fields: fractionalFields });
  evidence.fractional = fractionalComparison;
  for (const rendered of fractionalComparison.pages) {
    await writeFile(join(artifacts, `fractional-source-page-${rendered.page}.png`), Buffer.from(rendered.originalPng, "base64"));
    await writeFile(join(artifacts, `fractional-static-page-${rendered.page}.png`), Buffer.from(rendered.staticPng, "base64"));
    delete rendered.originalPng; delete rendered.staticPng;
    for (const region of rendered.regions) assert.equal(region.maxDifference, 0,
      `Fractional PDF-lib edge must match exactly after Folio filling: page ${rendered.page} ${JSON.stringify(region)}`);
  }
  await api("/close", { documentId: filledInfo.id, revision: filledInfo.revision, discardChanges: true });
  evidence.checks.push("PDF-lib half-unit edges, fractional UserUnit and all rotations have exact RGBA parity after every text/choice is first filled by Folio");
  const extraCopies = await Promise.all([
    { name: "stale", password: null, terms: ["Current value", "Inherited", "Visible read-only", "*".repeat(19)] },
    { name: "button-transform", password: null, terms: ["ON VALUE"] },
    { name: "aes128", password: "user", terms: ["Inherited"] },
    { name: "aes256", password: "user", terms: ["Inherited"] },
  ].map(async (item) => ({ ...item, data: Array.from(await readFile(join(artifacts, `${item.name}-static.pdf`))) })));
  const extraReopening = await page.evaluate(async (copies) => {
    const { getDocument, GlobalWorkerOptions } = await import("/node_modules/pdfjs-dist/legacy/build/pdf.mjs");
    GlobalWorkerOptions.workerSrc = "/node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs";
    const options = { standardFontDataUrl: `${location.origin}/vendor/pdfjs/standard_fonts/`, wasmUrl: `${location.origin}/vendor/pdfjs/wasm/`,
      cMapUrl: `${location.origin}/vendor/pdfjs/cmaps/`, cMapPacked: true, enableXfa: false };
    const results = [];
    for (const copy of copies) {
      if (copy.password) {
        const wrong = getDocument({ ...options, data: new Uint8Array(copy.data), password: "wrong-original-test-password" });
        let rejected = false;
        try { await wrong.promise; }
        catch (error) { if (error.name !== "PasswordException") throw error; rejected = true; }
        finally { await wrong.destroy(); }
        if (!rejected) throw new Error(`${copy.name} lost password protection`);
      }
      const task = getDocument({ ...options, data: new Uint8Array(copy.data), password: copy.password ?? undefined });
      const pdf = await task.promise;
      try {
        const objects = await pdf.getFieldObjects();
        if (objects && (objects instanceof Map ? objects.size : Object.keys(objects).length)) throw new Error(`${copy.name} retained fields`);
        const page = await pdf.getPage(1), viewport = page.getViewport({ scale: 1.5 });
        const canvas = document.createElement("canvas"); canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
        await page.render({ canvas, viewport }).promise;
        const text = (await page.getTextContent()).items.filter((item) => "str" in item).map((item) => item.str).join(" ");
        for (const term of copy.terms) if (!text.includes(term)) throw new Error(`${copy.name} lost expected visible text ${term}`);
        const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
        let ink = 0;
        for (let offset = 0; offset < pixels.length; offset += 4) if (Math.min(pixels[offset], pixels[offset + 1], pixels[offset + 2]) < 245) ink++;
        if (ink < 10) throw new Error(`${copy.name} rendered no persistent values`);
        results.push({ name: copy.name, passwordProtected: Boolean(copy.password), fieldCount: 0, text, ink,
          png: canvas.toDataURL("image/png").split(",")[1] });
      } finally { await task.destroy(); }
    }
    return results;
  }, extraCopies);
  for (const copy of extraReopening) {
    await writeFile(join(artifacts, `${copy.name}-static-render.png`), Buffer.from(copy.png, "base64"));
    delete copy.png;
  }
  evidence.extraReopening = extraReopening;
  evidence.checks.push("Independent PDF.js rendering verifies stale/missing AP regeneration, visible read-only values, password masks, inherited-font BBox/Matrix button AP, and AES-128/AES-256 password retention");
  await page.getByLabel("Choose PDF files").setInputFiles(join(artifacts, "authored.pdf"));
  await page.getByRole("tab", { name: /authored\.pdf/ }).waitFor();
  await page.getByRole("button", { name: "Design form fields", exact: true }).click();
  const workspace = page.locator(".document-workspace:not([hidden])");
  await workspace.getByLabel("Create field type").selectOption("text");
  await workspace.getByLabel("Partial name", { exact: true }).fill("ui_created");
  await workspace.getByLabel("Accessible label", { exact: true }).fill("UI created field");
  await workspace.getByLabel("Set a default value", { exact: true }).check();
  await workspace.getByLabel("Default text", { exact: true }).fill("Applied ui");
  await workspace.getByLabel("Left (pt)", { exact: true }).fill("320");
  await workspace.getByRole("button", { name: "Create field", exact: true }).click();
  await workspace.getByText("Form design applied to the PDF.", { exact: false }).waitFor();
  let current = await active();
  const applied = await workingBytes(current), appliedPdf = await PDFDocument.load(applied);
  assert.equal(appliedPdf.getForm().getTextField("ui_created").getText(), "Applied ui");
  const appliedRevision = current.revision;
  const undoResponse = page.waitForResponse((response) => response.url().endsWith("/__folio/undo") && response.ok());
  await workspace.getByRole("button", { name: "Undo (Command+Z)", exact: true }).click();
  await undoResponse;
  current = await active();
  assert.notEqual(current.revision, appliedRevision);
  assert.ok(!(await PDFDocument.load(await workingBytes(current))).getForm().getFields().some((field) => field.getName() === "ui_created"));
  const redoResponse = page.waitForResponse((response) => response.url().endsWith("/__folio/redo") && response.ok());
  await workspace.getByRole("button", { name: "Redo (Command+Shift+Z)", exact: true }).click();
  await redoResponse;
  current = await active();
  assert.equal((await PDFDocument.load(await workingBytes(current))).getForm().getTextField("ui_created").getText(), "Applied ui");
  evidence.checks.push("UI creates a genuine independently reopened text field; native-model history undo/redo preserves its value");
  const selected = (await (await api("/forms/designer", { documentId: current.id, revision: current.revision })).json()).fields.find(({ field }) => field.name === "ui_created");
  await workspace.getByLabel("Existing logical field", { exact: true }).selectOption(selected.field.id);
  await workspace.getByRole("button", { name: "Place selected widget on page", exact: true }).click();
  const paper = workspace.locator('.pdf-paper[data-page="1"]');
  await paper.waitFor({ state: "visible" });
  await paper.locator(".form-designer-overlay.capturing").waitFor({ state: "visible" });
  const box = await paper.boundingBox();
  const physicalWidth = current.pages[0].geometry.widthPoints;
  const scale = box.width / physicalWidth;
  evidence.pointer = { box, physicalWidth, scale };
  await page.mouse.move(box.x + 320 * scale, box.y + 80 * scale);
  const gestureWrites = draftWrites.length;
  const gestureAck = page.waitForResponse((response) => response.url().endsWith("/__folio/draft")
    && response.request().postDataJSON().documentId === current.id && response.request().postDataJSON().pending && response.ok(), { timeout: 5000 });
  await page.mouse.down();
  await gestureAck;
  await api("/forms/flatten", { documentId: current.id, revision: current.revision }, {}, false);
  await page.mouse.move(box.x + 520 * scale, box.y + 120 * scale, { steps: 5 });
  await page.mouse.up();
  await api("/forms/flatten", { documentId: current.id, revision: current.revision }, {}, false);
  assert.ok(!draftWrites.slice(gestureWrites).some((write) => write.documentId === current.id && !write.pending),
    "Completing a gesture must not clear the new design draft");
  await workspace.getByLabel("Accessible label", { exact: true }).fill("UI edited label");
  assert.ok(Math.abs(Number(await workspace.getByLabel("Top (pt)", { exact: true }).inputValue()) - 80) < 0.1);
  assert.ok(Math.abs(Number(await workspace.getByLabel("Height (pt)", { exact: true }).inputValue()) - 40) < 0.1);
  await workspace.getByRole("button", { name: "Apply design changes", exact: true }).click();
  await workspace.getByText("Form design applied to the PDF.", { exact: false }).waitFor();
  current = await active();
  const edited = (await (await api("/forms/designer", { documentId: current.id, revision: current.revision })).json()).fields.find(({ field }) => field.name === "ui_created");
  assert.equal(edited.field.id, selected.field.id);
  assert.equal(edited.field.widgets[0].id, selected.field.widgets[0].id);
  assert.equal(edited.field.label, "UI edited label");
  assert.equal((await PDFDocument.load(await workingBytes(current))).getForm().getTextField("ui_created").getText(), "Applied ui");
  await workspace.getByLabel("Existing logical field", { exact: true }).selectOption(edited.field.id);
  await workspace.getByRole("button", { name: "Remove logical field and all widgets", exact: true }).click();
  await workspace.getByRole("button", { name: "Remove field", exact: true }).click();
  await workspace.getByText("Form design applied to the PDF.", { exact: false }).waitFor();
  current = await active();
  assert.ok(!(await PDFDocument.load(await workingBytes(current))).getForm().getFields().some((field) => field.getName() === "ui_created"));
  const restoreResponse = page.waitForResponse((response) => response.url().endsWith("/__folio/undo") && response.ok());
  await workspace.getByRole("button", { name: "Undo (Command+Z)", exact: true }).click();
  await restoreResponse;
  current = await active();
  assert.equal((await PDFDocument.load(await workingBytes(current))).getForm().getTextField("ui_created").getText(), "Applied ui");
  evidence.checks.push("Live gesture and resulting design draft block engine exports without an intermediate draft(false); on-page geometry/property edits preserve IDs/value and removal is undoable");
  await workspace.getByLabel("Create field type").selectOption("text");
  await workspace.getByLabel("Partial name", { exact: true }).fill("keep_design_draft");
  await workspace.getByLabel("Width (pt)", { exact: true }).fill("0");
  const beforeFailure = await active();
  await workspace.getByRole("button", { name: "Create field", exact: true }).click();
  await workspace.getByRole("alert").filter({ hasText: "Width" }).waitFor();
  assert.equal(await workspace.getByLabel("Partial name", { exact: true }).inputValue(), "keep_design_draft");
  assert.equal((await active()).revision, beforeFailure.revision);
  await workspace.getByRole("button", { name: "Download PDF", exact: true }).click();
  await workspace.getByRole("alert").filter({ hasText: "before saving" }).waitFor();
  await api("/forms/flatten", { documentId: beforeFailure.id, revision: beforeFailure.revision }, {}, false);
  await workspace.getByRole("button", { name: "Find in document (Command+F)", exact: true }).click();
  await workspace.getByRole("button", { name: "All tools", exact: true }).click();
  await workspace.getByRole("button", { name: "Design form fields", exact: true }).click();
  assert.equal(await workspace.getByLabel("Partial name", { exact: true }).inputValue(), "keep_design_draft");
  await workspace.getByLabel("Page number", { exact: true }).fill("4");
  await workspace.getByLabel("Page number", { exact: true }).press("Enter");
  await page.waitForFunction(() => !document.querySelector('.document-workspace:not([hidden]) .pdf-paper[data-page="1"]'));
  assert.equal(await workspace.getByLabel("Partial name", { exact: true }).inputValue(), "keep_design_draft");
  await api("/forms/flatten", { documentId: beforeFailure.id, revision: beforeFailure.revision }, {}, false);
  await workspace.getByLabel("Page number", { exact: true }).fill("1");
  await workspace.getByLabel("Page number", { exact: true }).press("Enter");
  const blank = await PDFDocument.create(); blank.addPage([612, 792]);
  const otherPath = join(artifacts, "other-original-synthetic.pdf");
  await writeFile(otherPath, await blank.save({ useObjectStreams: false }));
  await page.getByLabel("Choose PDF files").setInputFiles(otherPath);
  await page.getByRole("tab", { name: /other-original-synthetic\.pdf/ }).waitFor();
  await page.getByRole("tab", { name: /authored\.pdf/ }).click();
  assert.equal(await workspace.getByLabel("Partial name", { exact: true }).inputValue(), "keep_design_draft");
  await workspace.getByLabel("Width (pt)", { exact: true }).fill("200");
  const palettes = ["system", "light", "dark", "sepia", "contrast"];
  for (const theme of palettes) {
    await page.evaluate((theme) => {
      if (theme === "system") delete document.documentElement.dataset.theme;
      else document.documentElement.dataset.theme = theme;
    }, theme);
    await workspace.getByLabel("Partial name", { exact: true }).focus();
    await page.screenshot({ path: join(artifacts, `designer-${theme}.png`) });
    const metrics = await workspace.locator(".designer-intro").evaluate((element) => ({
      width: element.getBoundingClientRect().width, scrollWidth: element.scrollWidth,
      textColor: getComputedStyle(element.querySelector("p")).color,
      background: getComputedStyle(document.querySelector(".tools-panel.panel-form-designer")).backgroundColor,
    }));
    assert.ok(metrics.scrollWidth <= metrics.width + 1, `Panel overflow in ${theme}`);
    const luminance = (color) => {
      const channels = color.match(/[\d.]+/g).slice(0, 3).map((value) => {
        const channel = Number(value) / 255;
        return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
      });
      return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
    };
    const a = luminance(metrics.textColor), b = luminance(metrics.background), contrast = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
    assert.ok(contrast >= 4.5, `Body contrast in ${theme}: ${contrast}`);
    evidence.visuals.push({ theme, ...metrics, contrast });
  }
  await page.setViewportSize({ width: 840, height: 900 });
  await workspace.getByRole("button", { name: "Review design", exact: true }).click();
  await workspace.getByLabel("Partial name", { exact: true }).waitFor({ state: "visible" });
  await page.screenshot({ path: join(artifacts, "designer-narrow.png") });
  assert.ok(await workspace.getByLabel("Partial name", { exact: true }).isVisible());
  evidence.checks.push("Validation/Save failures retain design drafts; engine blocks static export; panel and tab changes preserve settings; five palettes and narrow desktop captured with reduced motion");
  await workspace.getByRole("button", { name: "Discard design draft", exact: true }).click();
  await workspace.getByText("Design draft discarded.", { exact: false }).waitFor();
  current = await active();
  const sourceBeforeExport = await workingBytes(current);
  await workspace.getByRole("button", { name: "Export static form copy", exact: true }).click();
  await page.getByRole("dialog", { name: "Export a separate static form copy?" }).waitFor();
  await page.getByLabel("I understand this is a static copy", { exact: false }).check();
  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export static copy", exact: true }).click();
  const download = await downloadPromise;
  const downloadPath = join(artifacts, "ui-static.pdf");
  await download.saveAs(downloadPath);
  const uiStatic = await PDFDocument.load(await readFile(downloadPath));
  assert.equal(uiStatic.catalog.has(PDFName.of("AcroForm")), false);
  assert.equal(digest(await workingBytes(await active())), digest(sourceBeforeExport));
  assert.equal((await active()).revision, current.revision);
  evidence.checks.push("UI static-copy download contains no AcroForm and does not mutate source bytes, revision or unsaved state");
  assert.deepEqual(evidence.externalRequests, []);
  assert.deepEqual(evidence.consoleErrors, []);
  await writeFile(join(artifacts, "evidence.json"), JSON.stringify(evidence, null, 2));
  console.log(JSON.stringify({ result: "passed", artifacts, checks: evidence.checks, pages: reopening.pages.map(({ regions, ...page }) => ({
    ...page, widgets: regions.length, maxMeanPixelDifference: Math.max(...regions.map((region) => region.meanDifference)),
  })) }, null, 2));
} catch (error) {
  evidence.failure = error.stack;
  if (page) {
    await writeFile(join(artifacts, "failure.html"), await page.content());
    await page.screenshot({ path: join(artifacts, "failure.png") });
  }
  await writeFile(join(artifacts, "failure.json"), JSON.stringify(evidence, null, 2));
  throw error;
} finally {
  if (browser) await browser.close();
  for (const child of children.reverse()) {
    if (child.exitCode !== null || child.startError) continue;
    const exited = new Promise((resolve) => child.once("exit", resolve));
    child.kill("SIGTERM");
    await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 5000))]);
    if (child.exitCode === null) { child.kill("SIGKILL"); await exited; }
  }
}
