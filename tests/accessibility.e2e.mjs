import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer as createViteServer } from "vite";
import { PDFDocument, PDFArray, PDFDict, PDFName, PDFString, StandardFonts } from "pdf-lib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const temporary = await mkdtemp(join(tmpdir(), "folio-semantic-"));
const artifacts = process.env.FOLIO_ACCESSIBILITY_ARTIFACTS ? resolve(process.env.FOLIO_ACCESSIBILITY_ARTIFACTS) : temporary;
await mkdir(artifacts, { recursive: true });
const target = process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : join(root, "target/accessibility-build");
const env = { ...process.env, CARGO_TARGET_DIR: target, CARGO_INCREMENTAL: "0", CARGO_PROFILE_DEV_DEBUG: "0", CARGO_PROFILE_TEST_DEBUG: "0", CARGO_BUILD_JOBS: "2" };
const children = [], errors = [], externalRequests = [];
const evidence = { manualSemanticsAreNotCertified: true, pdfUaClaimed: false, externalRequests, browserErrors: errors, visual: [] };
let frontend, browser, page;
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const ALT = "One crimson square, deliberately described by the human author.";
const MANAGED = "Reflowed original heading: Café, Ελληνικά, Кириллица.\n\nAn explicit hard paragraph break remains.";

function start(command, args, extra = {}) {
  const child = spawn(command, args, { cwd: root, env: { ...env, ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  child.output = "";
  for (const stream of [child.stdout, child.stderr]) stream.on("data", (data) => { child.output = (child.output + data).slice(-2 * 1024 * 1024); });
  child.on("error", (error) => { child.startError = error; });
  children.push(child); return child;
}
async function command(command, args) {
  const child = start(command, args);
  await new Promise((resolve, reject) => { child.on("error", reject); child.on("exit", (code) => code === 0 ? resolve() : reject(new Error(`${command} failed (${code}): ${child.output}`))); });
  return child.output;
}
async function freePort(preferred) {
  const socket = createServer();
  try { await new Promise((resolve, reject) => { socket.once("error", reject); socket.listen(preferred, "127.0.0.1", resolve); }); }
  catch (error) { if (error.code === "EADDRINUSE" && preferred) return freePort(0); throw error; }
  const value = socket.address().port;
  await new Promise((resolve) => socket.close(resolve)); return value;
}
async function ready(url, child) {
  const until = Date.now() + 120_000;
  while (Date.now() < until) {
    if (child.startError || child.exitCode !== null) throw new Error(`Semantic helper did not start: ${child.startError ?? child.output}`);
    try { if ((await fetch(url, { signal: AbortSignal.timeout(2000) })).ok) return; } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Semantic helper is not responsive: ${child.output}`);
}
async function fixture() {
  const pdf = await PDFDocument.create();
  const font = await pdf.embedFont(StandardFonts.Helvetica);
  pdf.setTitle("Original independently authored semantic fixture");
  const image = pdf.context.register(pdf.context.stream(new Uint8Array([200, 30, 60]), {
    Type: PDFName.of("XObject"), Subtype: PDFName.of("Image"), Width: 1, Height: 1,
    ColorSpace: PDFName.of("DeviceRGB"), BitsPerComponent: 8,
  }));
  const first = pdf.addPage([612, 792]);
  first.node.set(PDFName.of("Resources"), pdf.context.obj({ Font: { F1: font.ref }, XObject: { Im: image } }));
  first.node.set(PDFName.of("Contents"), pdf.context.register(pdf.context.flateStream(
    "q BT /F1 20 Tf 1 0 0 1 32 720 Tm (Original semantic heading) Tj ET Q\n" +
    "BT /F1 11 Tf 1 0 0 1 320 650 Tm (Right drawn first, read second.) Tj ET\n" +
    "BT /F1 11 Tf 1 0 0 1 32 650 Tm (Left must be read first.) Tj ET\n" +
    "q 60 0 0 60 32 520 cm /Im Do Q\n" +
    "0.123456789 0.4 0.7 rg 31.123456789 400 80 20 re f\n" +
    "BT /F1 11 Tf 1 0 0 1 32 350 Tm (End of original searchable source.) Tj ET\n")));
  const second = pdf.addPage([612, 792]);
  second.node.set(PDFName.of("Resources"), pdf.context.obj({ Font: { F1: font.ref } }));
  second.node.set(PDFName.of("Contents"), pdf.context.register(pdf.context.flateStream(
    "BT /F1 12 Tf 1 0 0 1 32 720 Tm (Deuxieme page: Bonjour.) Tj ET")));
  return pdf.save({ useObjectStreams: false });
}
function assignments(report) {
  const entries = report.candidates.map((candidate, index) => ({
    candidateId: candidate.id, role: index === 0 ? "H1" : candidate.kind === "text" ? "P" : candidate.kind === "graphic" ? "Artifact" : "Figure",
    alt: candidate.kind === "figure" ? ALT : null, language: candidate.page === 1 ? "fr-FR" : null,
  }));
  [entries[1], entries[2]] = [entries[2], entries[1]];
  return { documentId: report.documentId, revision: report.revision, sourceEpoch: report.sourceEpoch,
    title: "Reviewed semantic document", language: "en-US", entries, reviewed: true };
}
async function malformed(bytes, variant) {
  const pdf = await PDFDocument.load(bytes, { updateMetadata: false });
  const rootRef = pdf.catalog.get(PDFName.of("StructTreeRoot")), tree = pdf.context.lookup(rootRef, PDFDict);
  const doc = pdf.context.lookup(tree.lookup(PDFName.of("K"), PDFArray).get(0), PDFDict);
  const kids = doc.lookup(PDFName.of("K"), PDFArray);
  const firstRef = kids.get(0), first = pdf.context.lookup(firstRef, PDFDict);
  if (variant === "cycle") first.set(PDFName.of("K"), pdf.context.obj([firstRef]));
  else if (variant === "shared") kids.push(firstRef);
  else if (variant === "parent") {
    const parent = tree.lookup(PDFName.of("ParentTree"), PDFDict);
    parent.set(PDFName.of("Nums"), pdf.context.obj([0, [rootRef]]));
  } else if (variant === "alt") {
    for (const ref of kids.asArray()) {
      const node = pdf.context.lookup(ref, PDFDict);
      if (node.get(PDFName.of("S"))?.toString() === "/Figure") node.set(PDFName.of("Alt"), PDFString.of(""));
    }
  } else tree.set(PDFName.of("RoleMap"), pdf.context.obj({ LoopA: PDFName.of("LoopB"), LoopB: PDFName.of("LoopA") }));
  return pdf.save({ useObjectStreams: false });
}
function assertNodes(actual, expectedText) {
  assert.deepEqual(actual.pages[0].nodes.map((node) => node.role), ["H1", "P", "P", "Figure", "P"]);
  assert.equal(actual.pages[0].nodes[0].text, expectedText.replace(/\s+/gu, " ").trim());
  assert.equal(actual.pages[0].nodes[1].text, "Left must be read first.");
  assert.equal(actual.pages[0].nodes[2].text, "Right drawn first, read second.");
  assert.equal(actual.pages[0].nodes[3].alt, ALT);
  assert.ok(actual.pages[0].nodes.every((node) => node.contentIds.length > 0));
  const ids = actual.pages.flatMap((page) => page.nodes.flatMap((node) => node.contentIds));
  assert.equal(new Set(ids).size, ids.length, "actual PDF.js content bindings are unique");
  assert.equal(actual.pages[0].language, "en-US"); assert.equal(actual.pdfUaClaim, null);
  assert.equal(actual.pages[1].nodes[0].language, "fr-FR");
  assert.match(actual.pages[1].nodes[0].text, /Bonjour/);
}

try {
  const frontendPort = await freePort(1437), backendPort = await freePort(19165);
  evidence.ports = { frontendPort, backendPort };
  if (!process.env.FOLIO_ACCESSIBILITY_SKIP_BUILD) await command("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--quiet"]);
  const backend = start(join(target, "debug/folio-preview"), [], { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) });
  await ready(`http://127.0.0.1:${backendPort}/health`, backend);
  frontend = await createViteServer({ root, logLevel: "error", server: { host: "127.0.0.1", port: frontendPort, strictPort: true,
    proxy: { "/__folio": { target: `http://127.0.0.1:${backendPort}`, rewrite: (path) => path.replace(/^\/__folio/, "") } } } });
  await frontend.listen();
  const origin = `http://127.0.0.1:${frontendPort}`;
  const api = (route, value, headers = {}, body = undefined) => fetch(`http://127.0.0.1:${backendPort}${route}`, {
    headers: { "X-Folio-Preview": "1", Origin: origin, ...(value ? { "Content-Type": "application/json" } : {}), ...headers },
    ...(value || body ? { method: "POST", body: body ?? JSON.stringify(value) } : {}),
  });
  const json = async (route, value) => {
    const response = await api(route, value);
    assert.ok(response.ok, `${route}: ${await response.clone().text()}`); return response.json();
  };
  const open = async (bytes, name) => {
    const response = await api("/open", null, { "Content-Type": "application/pdf", "X-File-Name": encodeURIComponent(name) }, bytes);
    const result = await response.json(); assert.equal(result.status, "opened", JSON.stringify(result)); return result.document;
  };
  const sourceBytes = async (info, rendererOnly = false) => {
    const response = await api(rendererOnly ? "/render-bytes" : "/bytes", null, { "X-Document-Id": String(info.id), "X-Revision": String(info.revision) });
    assert.ok(response.ok, await response.clone().text()); return new Uint8Array(await response.arrayBuffer());
  };
  browser = await chromium.launch({ headless: true, executablePath: process.env.FOLIO_ACCESSIBILITY_BROWSER
    ?? "/Users/adityavaish/.copilot/session-state/19181025-f105-4d9f-90c9-1c035d6e1f52/files/comparison-browser-manual/chrome-mac/headless_shell" });
  evidence.browserVersion = browser.version();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce", acceptDownloads: true });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    externalRequests.push(route.request().url()); return route.abort();
  });
  page = await context.newPage(); page.setDefaultTimeout(30_000);
  const workspace = page.locator(".document-workspace:not([hidden])");
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(origin);
  await page.getByRole("img", { name: "Rendered PDF page 1", exact: true }).waitFor();
  const file = join(artifacts, "Original-semantic-source.pdf"), original = await fixture();
  await writeFile(file, original);
  await page.getByLabel("Choose PDF files").setInputFiles(file);
  await page.getByRole("tab", { name: /Original-semantic-source\.pdf/, selected: true }).waitFor();
  let snapshot = await json("/bootstrap");
  const info = snapshot.documents.find((document) => document.name === "Original-semantic-source.pdf");
  assert.ok(info); const state = JSON.stringify(info);
  const tools = page.getByRole("button", { name: "All tools", exact: true });
  async function panel() {
    const action = workspace.getByRole("button", { name: "Show accessibility and reading order", exact: true });
    await action.click(); await workspace.locator(".accessibility-panel").waitFor();
  }
  await panel();
  await page.getByRole("button", { name: "Inspect structure and source", exact: true }).click();
  await page.getByRole("button", { name: "Review source semantics", exact: true }).waitFor();
  await page.getByRole("button", { name: "Review source semantics", exact: true }).click();
  await page.getByLabel("Tagged copy title", { exact: true }).fill("Reviewed semantic document");
  await page.getByLabel("Document language tag", { exact: true }).fill("en-US");
  assert.equal(await page.getByRole("button", { name: "Prepare reviewed tagged copy", exact: true }).isEnabled(), false);
  const setRole = async (index, role) => {
    await workspace.locator(".semantic-order > li").nth(index).getByRole("button").click();
    await page.getByLabel(`Role for item ${index + 1}`, { exact: true }).selectOption(role);
  };
  await setRole(0, "H1"); await setRole(1, "P"); await setRole(2, "P");
  await page.getByRole("button", { name: "Move item 3 earlier", exact: true }).click();
  assert.match(await workspace.locator(".semantic-order > li").nth(1).innerText(), /Left must be read first/);
  await setRole(3, "Figure");
  await page.getByLabel("Human alternative for item 4", { exact: true }).fill(ALT);
  await setRole(4, "Artifact"); await setRole(5, "P"); await setRole(6, "P");
  await page.getByLabel("Item language (optional override)", { exact: true }).fill("fr-FR");
  await page.getByLabel(/I reviewed every role/).check();
  await page.getByRole("button", { name: "Prepare reviewed tagged copy", exact: true }).waitFor({ state: "visible" });
  assert.equal(await page.getByRole("button", { name: "Prepare reviewed tagged copy", exact: true }).isEnabled(), true);
  const originalTab = page.getByRole("tab", { name: /Original-semantic-source\.pdf/ });
  await page.getByRole("tab", { name: /Folio quick start\.pdf/ }).click();
  await page.getByRole("tab", { name: /Folio quick start\.pdf/, selected: true }).waitFor();
  await originalTab.click();
  await page.getByRole("tab", { name: /Original-semantic-source\.pdf/, selected: true }).waitFor();
  await panel();
  assert.equal(await page.getByLabel("Tagged copy title", { exact: true }).inputValue(), "Reviewed semantic document");
  assert.equal(await page.getByLabel("Document language tag", { exact: true }).inputValue(), "en-US");
  assert.equal(await page.getByLabel(/I reviewed every role/).isChecked(), true);
  await page.getByRole("button", { name: "Download PDF", exact: true }).click();
  await page.getByText(/before saving the original PDF/).waitFor();
  assert.equal(sha(await sourceBytes(info, true)), sha(original), "blocked ordinary save leaves exact source bytes");
  for (const [route, request] of [
    ["/accessibility/inspect", { documentId: info.id, revision: 0 }],
    ["/preflight/inspect", { documentId: info.id, revision: 0 }],
    ["/ocr/begin", { documentId: info.id, revision: 0, options: { pages: [0], dpi: 300, language: "eng", existingText: "skip" } }],
  ]) {
    const response = await api(route, request); assert.equal(response.ok, false, `${route} must reject semantic draft`);
  }
  evidence.drafts = { inactiveTabRetained: true, ordinarySaveBlocked: true, inspectionPreflightOcrBlocked: true };
  if (!process.env.FOLIO_ACCESSIBILITY_SKIP_VISUAL && !process.env.FOLIO_ACCESSIBILITY_SKIP_AUTHORING_VISUAL) {
    for (const theme of ["system", "light", "dark", "sepia", "contrast"]) for (const [name, width, height] of [["desktop", 1440, 1000], ["compact", 700, 640]]) {
      await page.evaluate((theme) => { document.documentElement.dataset.theme = theme === "system" ? "light" : theme; }, theme);
      await page.setViewportSize({ width, height }); await panel();
      await page.getByRole("button", { name: "Prepare reviewed tagged copy", exact: true }).scrollIntoViewIfNeeded();
      const metrics = await workspace.locator(".semantic-commit").evaluate((element) => {
        const box = element.getBoundingClientRect();
        return { top: box.top, bottom: box.bottom, height: innerHeight, horizontalOverflow: document.documentElement.scrollWidth > innerWidth };
      });
      assert.ok(metrics.bottom <= metrics.height + 1 && metrics.top >= 0 && !metrics.horizontalOverflow, `${theme}/${name} fixed semantic actions`);
      const filename = `semantic-authoring-${theme}-${name}.png`;
      await page.screenshot({ path: join(artifacts, filename) }); evidence.visual.push(filename);
    }
  }
  await page.setViewportSize({ width: 1440, height: 1000 }); await panel();
  await page.getByRole("button", { name: "Prepare reviewed tagged copy", exact: true }).click();
  await page.getByRole("button", { name: "Save separate tagged copy", exact: true }).waitFor();
  const downloaded = page.waitForEvent("download");
  await page.getByRole("button", { name: "Save separate tagged copy", exact: true }).click();
  const outputPath = join(artifacts, "Actual-UI-tagged-copy.pdf");
  await (await downloaded).saveAs(outputPath);
  await page.getByRole("button", { name: "Revise retained recipe", exact: true }).waitFor();
  const output = await readFile(outputPath);
  assert.equal(sha(await sourceBytes(info)), sha(original));
  snapshot = await json("/bootstrap");
  assert.equal(JSON.stringify(snapshot.documents.find((document) => document.id === info.id)), state);
  const actual = await page.evaluate(async (bytes) => (await import("/tests/accessibility.browser.ts")).semanticEvidence(bytes), Array.from(output));
  assert.equal(actual.title, "Reviewed semantic document"); assertNodes(actual, "Original semantic heading");
  const pixels = await page.evaluate(async ({ source, output }) => (await import("/tests/accessibility.browser.ts")).compareSemanticPixels(source, output), { source: Array.from(original), output: Array.from(output) });
  assert.ok(pixels.every((page) => page.changedPixels === 0), JSON.stringify(pixels));
  evidence.actualUiCopy = { path: outputPath, sha256: sha(output), structure: actual, pixels, sourceStateUnchanged: true };
  await page.getByLabel("Choose PDF files").setInputFiles(outputPath);
  await page.getByRole("tab", { name: /Actual-UI-tagged-copy\.pdf/, selected: true }).waitFor();
  await panel(); await page.getByRole("button", { name: "Inspect structure and source", exact: true }).click();
  await page.getByRole("button", { name: "Read tagged content", exact: true }).waitFor();
  await page.getByRole("button", { name: "Read tagged content", exact: true }).click();
  await workspace.locator(".semantic-document").waitFor();
  assert.equal(await workspace.locator(".document-area .textLayer, .document-area canvas").count(), 0);
  assert.equal(await workspace.locator(".semantic-document h1").innerText(), "Original semantic heading");
  const dom = await workspace.locator(".semantic-document").evaluate((element) => ({
    language: element.getAttribute("lang"), paragraphs: [...element.querySelectorAll("p")].map((node) => node.textContent),
    figureAlts: [...element.querySelectorAll('[role="img"]')].map((node) => node.getAttribute("aria-label")),
    overrides: [...element.querySelectorAll("[lang]")].map((node) => node.getAttribute("lang")),
  }));
  assert.equal(dom.language, "en-US"); assert.deepEqual(dom.figureAlts, [ALT]); assert.ok(dom.overrides.includes("fr-FR"));
  assert.equal(dom.paragraphs[0], "Left must be read first."); assert.equal(dom.paragraphs[1], "Right drawn first, read second.");
  await page.getByRole("button", { name: "Start reading", exact: true }).click();
  assert.equal(await page.evaluate(() => document.activeElement?.tagName), "H1");
  await page.getByRole("button", { name: "Next item", exact: true }).click();
  assert.equal(await page.evaluate(() => document.activeElement?.textContent), "Left must be read first.");
  await page.getByLabel("Go to tagged heading", { exact: true }).selectOption({ index: 1 });
  assert.equal(await page.evaluate(() => document.activeElement?.tagName), "H1");
  await writeFile(join(artifacts, "semantic-reader-aria.txt"), await workspace.locator(".structured-reader").ariaSnapshot());
  evidence.reader = { dom, noDuplicateCanvasText: true, nextItemKeyboardFocus: true, headingKeyboardFocus: true, nativeVoiceOverTested: false };
  if (!process.env.FOLIO_ACCESSIBILITY_SKIP_VISUAL) for (const theme of ["system", "light", "dark", "sepia", "contrast"]) for (const [name, width, height] of [["desktop", 1440, 1000], ["compact", 700, 640]]) {
    await page.evaluate((theme) => { document.documentElement.dataset.theme = theme === "system" ? "light" : theme; }, theme);
    await page.setViewportSize({ width, height });
    await workspace.locator(".structured-reader").evaluate((element) => { element.scrollTop = 0; });
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    const filename = `semantic-reader-${theme}-${name}.png`;
    await page.screenshot({ path: join(artifacts, filename) }); evidence.visual.push(filename);
  }
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole("button", { name: "Visual PDF", exact: true }).click();
  await page.getByRole("img", { name: "Rendered PDF page 1", exact: true }).waitFor();
  assert.match(await workspace.locator(".readonly-banner").innerText(), /Folio tagged copy/);

  const managedSource = await open(original, "Managed-paragraph-source.pdf");
  const blocks = await json("/paragraphs", { documentId: managedSource.id, revision: 0, page: 0 });
  const block = blocks.blocks[0];
  const reflow = await json("/paragraphs/edit", { documentId: managedSource.id, revision: 0, change: {
    epoch: blocks.epoch, page: 0, path: block.path, blockId: block.id, expected: block.text, text: MANAGED,
    placement: [32, 40, 250, 140], size: 12, leading: 18, alignment: "left", font: "noto",
  } });
  const reflowBytes = await sourceBytes(reflow);
  await writeFile(join(artifacts, "Actual-reflow-source.pdf"), reflowBytes);
  const managedReport = await json("/accessibility/inspect", { documentId: reflow.id, revision: reflow.revision });
  assert.ok(managedReport.authoringAllowed, JSON.stringify(managedReport.issues));
  assert.equal(managedReport.candidates[0].text, MANAGED);
  const managedPlan = await json("/accessibility/begin", assignments(managedReport));
  const managedResponse = await api("/accessibility/file", { jobId: managedPlan.jobId });
  assert.ok(managedResponse.ok, await managedResponse.clone().text());
  const managedOutput = new Uint8Array(await managedResponse.arrayBuffer());
  await json("/accessibility/cancel", { jobId: managedPlan.jobId });
  const managedPath = join(artifacts, "Actual-managed-paragraph-tagged.pdf");
  await writeFile(managedPath, managedOutput);
  const managedActual = await page.evaluate(async (bytes) => (await import("/tests/accessibility.browser.ts")).semanticEvidence(bytes), Array.from(managedOutput));
  assertNodes(managedActual, MANAGED);
  const managedPixels = await page.evaluate(async ({ source, output }) => (await import("/tests/accessibility.browser.ts")).compareSemanticPixels(source, output),
    { source: Array.from(reflowBytes), output: Array.from(managedOutput) });
  assert.ok(managedPixels.every((page) => page.changedPixels === 0));
  evidence.managedParagraph = { sha256: sha(managedOutput), structure: managedActual, pixels: managedPixels, exactLogicalBreaksInDom: false };
  await page.getByLabel("Choose PDF files").setInputFiles(managedPath);
  await page.getByRole("tab", { name: /Actual-managed-paragraph-tagged\.pdf/, selected: true }).waitFor();
  await panel();
  await page.getByRole("button", { name: "Inspect structure and source", exact: true }).click();
  await page.getByRole("button", { name: "Read tagged content", exact: true }).click();
  await workspace.locator(".semantic-document h1").waitFor();
  assert.equal(await workspace.locator(".semantic-document h1").textContent(), MANAGED);
  assert.equal(await workspace.locator(".semantic-document h1").innerText(), MANAGED);
  evidence.managedParagraph = { sha256: sha(managedOutput), structure: managedActual, pixels: managedPixels, exactLogicalBreaksInDom: true };
  const malformedCases = [];
  for (const variant of ["cycle", "shared", "parent", "alt", "rolemap"]) {
    const malformedBytes = await malformed(output, variant);
    await writeFile(join(artifacts, `malformed-${variant}.pdf`), malformedBytes);
    const broken = await open(malformedBytes, `Malformed-${variant}.pdf`);
    const report = await json("/accessibility/inspect", { documentId: broken.id, revision: broken.revision });
    assert.equal(report.status, "malformed_or_unsupported"); assert.equal(report.complete, false); assert.equal(report.structure.length, 0);
    malformedCases.push({ variant, issues: report.issues });
  }
  evidence.malformed = malformedCases;

  const cancelFile = join(artifacts, "Cancel-semantic-source.pdf");
  await writeFile(cancelFile, original); await page.getByLabel("Choose PDF files").setInputFiles(cancelFile);
  await page.getByRole("tab", { name: /Cancel-semantic-source\.pdf/, selected: true }).waitFor();
  await panel();
  let releaseResponse, startedResponse, canceledInspectionId;
  const started = new Promise((resolve) => { startedResponse = resolve; });
  await page.route("**/__folio/accessibility/inspect", async (route) => {
    const response = await route.fetch();
    canceledInspectionId = (await response.json()).inspectionId;
    startedResponse();
    await new Promise((resolve) => { releaseResponse = () => resolve(); });
    await route.fulfill({ response });
  });
  await page.getByRole("button", { name: "Inspect structure and source", exact: true }).click();
  await started;
  await page.getByRole("button", { name: "Cancel current operation", exact: true }).click();
  releaseResponse();
  await page.waitForFunction(() => {
    const button = [...document.querySelectorAll(".document-workspace:not([hidden]) .accessibility-panel button")].find((button) => button.textContent === "Inspect structure and source");
    return button && !button.disabled;
  });
  await page.unroute("**/__folio/accessibility/inspect");
  const staleChecks = await api("/accessibility/inspection/check", { inspectionId: canceledInspectionId });
  assert.equal(staleChecks.ok, false, "canceling after the real response must release its actual retained inspection ID");
  evidence.cancellation = { staleInspectionNotPresented: await page.getByRole("button", { name: "Review source semantics", exact: true }).count() === 0, operationCanceled: true };
  assert.ok(evidence.cancellation.staleInspectionNotPresented);
  assert.equal(errors.length, 0, JSON.stringify(errors)); assert.equal(externalRequests.length, 0);
  await writeFile(join(artifacts, "accessibility-evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
  console.log(JSON.stringify({ success: true, artifacts, pages: actual.pages.length,
    changedPixels: pixels.map((page) => page.changedPixels), managedChangedPixels: managedPixels.map((page) => page.changedPixels),
    domReading: true, malformedControls: malformedCases.length, externalRequests: 0, manualSemanticsAreNotCertified: true }, null, 2));
} catch (error) {
  if (page) await page.screenshot({ path: join(artifacts, "failure.png") }).catch(() => {});
  if (page) await writeFile(join(artifacts, "failure-aria.txt"), await page.locator("body").ariaSnapshot()).catch(() => {});
  await writeFile(join(artifacts, "failure.json"), JSON.stringify({ error: String(error), stack: error.stack, evidence, helpers: children.map((child) => ({ pid: child.pid, output: child.output })) }, null, 2));
  throw error;
} finally {
  if (browser) await browser.close();
  if (frontend) await frontend.close();
  for (const child of children) {
    if (child.exitCode !== null || !child.pid) continue;
    child.kill("SIGTERM");
    await new Promise((resolve) => { const timeout = setTimeout(resolve, 5000); child.once("exit", () => { clearTimeout(timeout); resolve(); }); });
    if (child.exitCode === null) child.kill("SIGKILL");
  }
  await rm(temporary, { recursive: true });
}
