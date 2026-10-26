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

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const temporary = await mkdtemp(join(tmpdir(), "folio-comparison-"));
const artifacts = process.env.FOLIO_COMPARISON_ARTIFACTS ? resolve(process.env.FOLIO_COMPARISON_ARTIFACTS) : temporary;
await mkdir(artifacts, { recursive: true });
const rendererOnly = process.argv.includes("--renderer-only");
const children = [];
const failures = [];
const evidence = { externalRequests: [], themes: [] };
const env = { ...process.env, CARGO_INCREMENTAL: "0", CARGO_PROFILE_DEV_DEBUG: "0", CARGO_PROFILE_TEST_DEBUG: "0", CARGO_BUILD_JOBS: "2" };
let browser;
let frontend;
let page;

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

async function port(preferred) {
  const socket = createServer();
  try {
    await new Promise((resolve, reject) => { socket.once("error", reject); socket.listen(preferred, "127.0.0.1", resolve); });
  } catch (error) {
    if (error.code === "EADDRINUSE" && preferred !== 0) return port(0);
    throw error;
  }
  const value = socket.address().port;
  await new Promise((resolve) => socket.close(resolve));
  return value;
}

async function ready(url, child) {
  const until = Date.now() + 120_000;
  while (Date.now() < until) {
    if (child.startError || child.exitCode !== null) throw new Error(`Comparison helper did not start: ${child.startError ?? child.output}`);
    try { if ((await fetch(url, { signal: AbortSignal.timeout(2000) })).ok) return; } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Comparison helper was not responsive: ${child.output}`);
}

function accounting() {
  const NativeWorker = window.Worker;
  window.__comparisonResources = { activeWorkers: 0, createdWorkers: 0, terminatedWorkers: 0, pdfWorkers: 0, canvases: [], urls: new Set() };
  window.Worker = class extends NativeWorker {
    constructor(url, options) {
      super(url, options);
      this.comparison = String(url).includes("comparison.worker");
      this.pdf = String(url).includes("pdf.worker");
      this.stopped = false;
      if (this.comparison) { window.__comparisonResources.activeWorkers++; window.__comparisonResources.createdWorkers++; }
      if (this.pdf) window.__comparisonResources.pdfWorkers++;
    }
    terminate() {
      if (!this.stopped) {
        this.stopped = true;
        if (this.comparison) {
          window.__comparisonResources.activeWorkers--;
          window.__comparisonResources.terminatedWorkers++;
        }
        if (this.pdf) window.__comparisonResources.pdfWorkers--;
      }
      return super.terminate();
    }
  };
  const create = document.createElement.bind(document);
  document.createElement = (...args) => {
    const element = create(...args);
    if (args[0] === "canvas" && new Error().stack?.includes("/src/features/comparison/engine.ts")) window.__comparisonResources.canvases.push(element);
    return element;
  };
  const createUrl = URL.createObjectURL.bind(URL), revokeUrl = URL.revokeObjectURL.bind(URL);
  URL.createObjectURL = (...args) => { const url = createUrl(...args); window.__comparisonResources.urls.add(url); return url; };
  URL.revokeObjectURL = (url) => { window.__comparisonResources.urls.delete(url); return revokeUrl(url); };
}

try {
  const frontendPort = await port(1428), backendPort = await port(19156);
  const ports = { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) };
  if (!rendererOnly) {
    await command("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--quiet"]);
    const backend = start(join(root, "src-tauri/target/debug/folio-preview") + (process.platform === "win32" ? ".exe" : ""), [], ports);
    await ready(`http://127.0.0.1:${backendPort}/health`, backend);
  }
  frontend = await createViteServer({
    root, logLevel: "error", optimizeDeps: { include: ["pdf-lib"] },
    server: { host: "127.0.0.1", port: frontendPort, strictPort: true,
      proxy: { "/__folio": { target: `http://127.0.0.1:${backendPort}`, rewrite: (path) => path.replace(/^\/__folio/, "") } } },
  });
  await frontend.listen();
  const origin = `http://127.0.0.1:${frontendPort}`;
  const api = async (route, value, extra = {}) => {
    const response = await fetch(`http://127.0.0.1:${backendPort}${route}`, {
      headers: { "X-Folio-Preview": "1", ...(value ? { "Content-Type": "application/json" } : {}), ...extra },
      ...(value ? { method: "POST", body: JSON.stringify(value) } : {}),
    });
    assert.ok(response.ok, `${route}: ${await response.clone().text()}`);
    return response;
  };
  const info = async () => (await (await api("/bootstrap")).json()).documents;
  const bytes = async (document) => new Uint8Array(await (await api("/bytes", null,
    { "X-Document-Id": String(document.id), "X-Revision": String(document.revision) })).arrayBuffer());
  browser = await chromium.launch({ headless: true,
    ...(process.env.FOLIO_COMPARISON_BROWSER ? { executablePath: process.env.FOLIO_COMPARISON_BROWSER } : {}) });
  evidence.browserVersion = browser.version();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce", acceptDownloads: true });
  await context.addInitScript(accounting);
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url());
    return route.abort();
  });
  page = await context.newPage();
  page.setDefaultTimeout(20_000);
  page.on("pageerror", (error) => failures.push(error.message));
  if (rendererOnly) await page.route(`${origin}/`, (route) => route.fulfill({ contentType: "text/html", body: "<!doctype html><title>Local comparison renderer checks</title>" }));
  await page.goto(origin);
  if (!rendererOnly) {
    await page.getByRole("img", { name: "Rendered PDF page 1", exact: true }).waitFor();
    await page.waitForFunction(() => [...document.querySelectorAll("button")].some((button) => button.getAttribute("aria-label") === "Compare open PDFs" && !button.disabled));
  }
  const readerWorkers = await page.evaluate(() => window.__comparisonResources.pdfWorkers);
  evidence.renderer = await page.evaluate(async () => {
    const { checkComparisonRenderer } = await import("/tests/comparison.browser.ts");
    let timer;
    try {
      return await Promise.race([checkComparisonRenderer(), new Promise((_resolve, reject) => {
        timer = window.setTimeout(() => reject(new Error("The comparison renderer checks exceeded 60 seconds.")), 60_000);
      })]);
    } finally { window.clearTimeout(timer); }
  });
  assert.deepEqual(await page.evaluate(() => ({
    active: window.__comparisonResources.activeWorkers,
    released: window.__comparisonResources.canvases.every((canvas) => canvas.width === 0 && canvas.height === 0),
  })), { active: 0, released: true });
  assert.equal(await page.evaluate(() => window.__comparisonResources.pdfWorkers), readerWorkers, "An isolated PDF.js comparison snapshot worker was not released.");
  if (!rendererOnly) {
    await page.getByRole("button", { name: "Compare open PDFs", exact: true }).click();
    await page.getByRole("heading", { name: "Review what changed between two PDFs" }).waitFor();
    assert.ok(await page.getByRole("button", { name: "Open another PDF", exact: true }).count() >= 1);
    const fixtures = await page.evaluate(async () => (await import("/tests/comparison.browser.ts")).comparisonFixtures());
    const paths = {};
    for (const [key, value] of Object.entries(fixtures)) {
      const path = join(artifacts, `comparison-${key}.pdf`);
      await writeFile(path, new Uint8Array(value.bytes));
      paths[key] = path;
    }
    await page.getByLabel("Choose PDF files").setInputFiles([paths.before, paths.after]);
    await page.getByRole("tab", { name: /comparison-after\.pdf/ }).waitFor();
    const tabs = (await info()).filter((document) => document.name.startsWith("comparison-"));
    const before = tabs.find((document) => document.name === "comparison-before.pdf");
    const after = tabs.find((document) => document.name === "comparison-after.pdf");
    assert.ok(before && after);
    await page.getByLabel("Before PDF", { exact: true }).selectOption(String(before.id));
    await page.getByLabel("After PDF", { exact: true }).selectOption(String(after.id));
    await page.getByLabel("Render resolution", { exact: true }).selectOption("72");
    const compare = page.getByRole("button", { name: "Compare working PDFs", exact: true });
    const finished = page.getByRole("heading", { name: "5 indexed pages have differences", exact: true });
    const complete = async () => {
      await page.waitForFunction(() => document.querySelector(".comparison-summary") || document.querySelector(".comparison-message.is-error"), null, { timeout: 90_000 });
      const error = page.locator(".comparison-message.is-error");
      assert.equal(await error.count(), 0, await error.count() ? await error.innerText() : "");
    };
    await compare.click();
    await complete();
    await finished.waitFor({ timeout: 90_000 });
    assert.match(await page.locator(".comparison-main").innerText(), /Not a claim that the PDFs are identical/);
    assert.match(await page.locator(".comparison-text").innerText(), /12/);
    assert.match(await page.locator(".comparison-text").innerText(), /15/);
    await page.getByRole("button", { name: "Next changed page", exact: true }).click();
    assert.match(await page.locator(".comparison-page-detail").innerText(), /Page 3: Pixels/);
    await page.getByRole("button", { name: "Overlay", exact: true }).click();
    await page.getByLabel("After opacity", { exact: true }).fill("70");
    assert.equal(await page.locator(".comparison-images img").count(), 3);

    for (const theme of process.env.FOLIO_COMPARISON_SKIP_VISUAL ? [] : ["system", "light", "dark", "sepia", "contrast"]) {
      await page.evaluate((value) => { document.documentElement.dataset.theme = value === "system" ? "light" : value; }, theme);
      for (const [label, width, height] of [["desktop", 1440, 1000], ["compact", 900, 800]]) {
        await page.setViewportSize({ width, height });
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${theme}/${label} overflowed horizontally`);
        assert.ok(await page.evaluate(() => [...document.querySelectorAll(".comparison-paper img")].every((image) => getComputedStyle(image).animationName === "none")), "PDF comparison pixels were animated");
        await page.screenshot({ path: join(artifacts, `comparison-${theme}-${label}.png`) });
        evidence.themes.push(`${theme}/${label}`);
      }
    }
    await page.evaluate(() => { delete document.documentElement.dataset.theme; });
    await page.setViewportSize({ width: 1440, height: 1000 });
    const download = page.waitForEvent("download");
    await page.getByRole("button", { name: "Export JSON report", exact: true }).click();
    const reportPath = join(artifacts, "comparison-report.json");
    await (await download).saveAs(reportPath);
    const report = JSON.parse(await readFile(reportPath, "utf8"));
    assert.equal(report.schemaVersion, 1);
    assert.equal(report.matching, "page_index");
    assert.equal(report.pages.length, 6);
    assert.equal(report.pages[4].textChanged, false);
    assert.ok(report.pages[4].pixels.changed > 0);
    assert.equal(report.pages[5].presence, "removed");
    assert.ok(report.scope.includes("not") && report.scope.includes("signature"));
    assert.equal(report.plan.before.documentId, before.id);
    assert.equal(report.plan.after.documentId, after.id);
    assert.ok((await readFile(reportPath)).length <= 4 * 1024 * 1024);
    assert.deepEqual(await bytes(before), new Uint8Array(fixtures.before.bytes));
    assert.deepEqual(await bytes(after), new Uint8Array(fixtures.after.bytes));
    evidence.report = { schema: report.schema, version: report.schemaVersion, pages: report.pages.map((row) =>
      ({ index: row.index, presence: row.presence, textChanged: row.textChanged, changedPixels: row.pixels.changed, geometry: row.geometryChanges })) };

    await page.getByRole("button", { name: "Return to PDFs", exact: true }).click();
    await page.getByRole("tab", { name: /comparison-after\.pdf/ }).click();
    const workspace = page.locator(".document-workspace:not([hidden])");
    await workspace.getByLabel("Page number", { exact: true }).fill("3");
    await workspace.getByLabel("Page number", { exact: true }).press("Enter");
    await page.getByRole("button", { name: "Compare open PDFs", exact: true }).click();
    await finished.waitFor();
    assert.equal(await page.getByLabel("After PDF", { exact: true }).inputValue(), String(after.id));
    assert.equal(await page.getByLabel("After opacity", { exact: true }).inputValue(), "70");
    await page.getByRole("button", { name: "Return to PDFs", exact: true }).click();
    assert.equal(await workspace.getByLabel("Page number", { exact: true }).inputValue(), "3");
    await workspace.getByRole("button", { name: "Rotate current page clockwise", exact: true }).click();
    await workspace.getByRole("button", { name: "Undo (Command+Z)", exact: true }).click();
    await page.getByRole("button", { name: "Compare open PDFs", exact: true }).click();
    await page.getByText("Stale comparison.", { exact: true }).waitFor();
    assert.equal(await page.getByRole("button", { name: "Export JSON report", exact: true }).isDisabled(), true);
    evidence.undoResurrectionStale = true;

    await page.getByRole("button", { name: "Clear comparison", exact: true }).click();
    const gate = { entered: null, release: null };
    const entered = new Promise((resolve) => { gate.entered = resolve; });
    const released = new Promise((resolve) => { gate.release = resolve; });
    await page.route("**/__folio/comparison/render-bytes", async (route) => { gate.entered(); await released; await route.continue(); });
    await compare.click();
    await entered;
    await page.getByRole("button", { name: "Cancel comparison", exact: true }).click();
    gate.release();
    await page.getByText("Comparison canceled. Neither source PDF was changed.", { exact: true }).waitFor();
    await page.unroute("**/__folio/comparison/render-bytes");
    assert.equal(await page.locator(".comparison-images").count(), 0);
    evidence.canceledUi = true;

    await page.getByRole("button", { name: "Clear comparison", exact: true }).click();
    await page.getByLabel("Choose PDF files").setInputFiles(paths.fileOnly);
    await page.getByRole("tab", { name: /comparison-fileOnly\.pdf/ }).waitFor();
    const only = (await info()).find((document) => document.name === "comparison-fileOnly.pdf");
    await page.getByLabel("After PDF", { exact: true }).selectOption(String(only.id));
    await compare.click();
    await complete();
    await page.getByRole("heading", { name: "No observed differences in compared content", exact: true }).waitFor({ timeout: 90_000 });
    assert.match(await page.locator(".comparison-summary").innerText(), /Not a claim that the PDFs are identical/);
    evidence.fileOnlyHonestUi = true;
    await page.getByRole("button", { name: "Clear comparison", exact: true }).click();
    await page.getByRole("button", { name: "Return to PDFs", exact: true }).click();
    await page.getByRole("tab", { name: /comparison-before\.pdf/ }).click();
    if (await workspace.getByRole("button", { name: "All tools", exact: true }).getAttribute("aria-expanded") !== "true") {
      await workspace.getByRole("button", { name: "All tools", exact: true }).click();
    }
    await workspace.getByRole("button", { name: "Document properties", exact: true }).click();
    await workspace.getByLabel("Title", { exact: true }).fill("Unapplied comparison safety fixture");
    await page.getByRole("tab", { name: /comparison-after\.pdf/ }).click();
    await page.getByRole("button", { name: "Compare open PDFs", exact: true }).click();
    assert.equal(await compare.isDisabled(), true);
    assert.match(await page.locator(".comparison-blocked").innerText(), /pending writing, form, metadata/);
    let denied = await fetch(`http://127.0.0.1:${backendPort}/bytes`, { headers: {
      "X-Folio-Preview": "1", "X-Document-Id": String(before.id), "X-Revision": "0",
    } });
    assert.equal(denied.status, 400, "Comparison silently cleared a peer metadata draft.");
    await page.getByRole("button", { name: "Return to PDFs", exact: true }).click();
    await page.getByRole("tab", { name: /comparison-before\.pdf/ }).click();
    await workspace.getByRole("button", { name: "Discard draft", exact: true }).click();
    await workspace.getByRole("button", { name: "All tools", exact: true }).click();
    await workspace.getByRole("button", { name: "Fill form fields", exact: true }).click();
    await workspace.getByRole("textbox", { name: "Synthetic.appearance", exact: true }).fill("Draft not in PDF");
    await page.getByRole("tab", { name: /comparison-after\.pdf/ }).click();
    await page.getByRole("button", { name: "Compare open PDFs", exact: true }).click();
    assert.equal(await compare.isDisabled(), true);
    denied = await fetch(`http://127.0.0.1:${backendPort}/bytes`, { headers: {
      "X-Folio-Preview": "1", "X-Document-Id": String(before.id), "X-Revision": "0",
    } });
    assert.equal(denied.status, 400, "Comparison silently cleared a peer form draft.");
    await page.getByRole("button", { name: "Return to PDFs", exact: true }).click();
    await page.getByRole("tab", { name: /comparison-before\.pdf/ }).click();
    await workspace.getByRole("button", { name: "Discard unapplied changes", exact: true }).click();
    await page.getByRole("button", { name: "Discard form changes", exact: true }).click();
    evidence.peerDraftsRetained = ["metadata", "forms"];

    for (const copy of [true, false]) {
      const protectedBytes = new Uint8Array(await (await api("/protection/export", {
        documentId: before.id, revision: 0, authorizationPassword: null,
        protection: { mode: "protect", openingPassword: "reader", ownerPassword: "owner",
          permissions: { print: "none", copy, edit: false, annotate: false, fillForms: false, assemble: false } },
      })).arrayBuffer());
      const path = join(artifacts, `comparison-copy-${copy}.pdf`);
      await writeFile(path, protectedBytes);
      await page.getByLabel("Choose PDF files").setInputFiles(path);
      await page.getByLabel("PDF password", { exact: true }).fill("reader");
      await page.getByRole("button", { name: "Unlock PDF", exact: true }).click();
      await page.getByRole("tab", { name: new RegExp(`comparison-copy-${copy}\\.pdf`) }).waitFor();
      const protectedTab = (await info()).find((document) => document.name === `comparison-copy-${copy}.pdf`);
      assert.ok(protectedTab && protectedTab.readOnlyReason && protectedTab.passwordProtected);
      await page.getByRole("button", { name: "Compare open PDFs", exact: true }).click();
      await page.getByLabel("After PDF", { exact: true }).selectOption(String(protectedTab.id));
      if (copy) {
        await compare.click();
        await page.getByRole("heading", { name: "No observed differences in compared content", exact: true }).waitFor({ timeout: 90_000 });
        const exportReport = page.getByRole("button", { name: "Export JSON report", exact: true });
        assert.equal(await exportReport.isDisabled(), true);
        await page.getByRole("checkbox", { name: /I understand the report contains unencrypted extracted content/ }).check();
        assert.equal(await exportReport.isEnabled(), true);
        const protectedDownload = page.waitForEvent("download");
        await exportReport.click();
        await (await protectedDownload).saveAs(join(artifacts, "protected-comparison-report.json"));
        assert.deepEqual(await bytes(protectedTab), protectedBytes);
        await page.getByRole("button", { name: "Clear comparison", exact: true }).click();
      } else {
        assert.equal(await compare.isDisabled(), true);
        assert.match(await page.locator(".comparison-blocked").innerText(), /does not permit copying\/extraction/);
      }
      await page.getByRole("button", { name: "Return to PDFs", exact: true }).click();
    }
    evidence.protectedReadOnlyCompared = true;
    evidence.protectedReportWarningRequired = true;
    evidence.copyRestrictedBlocked = true;
    const state = await info();
    assert.ok(state.filter((document) => [before.id, after.id].includes(document.id)).every((document) => !document.dirty && !document.canUndo && document.revision === 0));
    evidence.sourceSha256 = [fixtures.before.bytes, fixtures.after.bytes].map((value) => createHash("sha256").update(new Uint8Array(value)).digest("hex"));
  }
  evidence.resources = await page.evaluate(() => ({
    activeComparisonWorkers: window.__comparisonResources.activeWorkers,
    createdComparisonWorkers: window.__comparisonResources.createdWorkers,
    terminatedComparisonWorkers: window.__comparisonResources.terminatedWorkers,
    engineCanvasesReleased: window.__comparisonResources.canvases.every((canvas) => canvas.width === 0 && canvas.height === 0),
    comparisonBlobUrls: window.__comparisonResources.urls.size,
  }));
  assert.equal(evidence.resources.activeComparisonWorkers, 0);
  assert.equal(evidence.resources.createdComparisonWorkers, evidence.resources.terminatedComparisonWorkers);
  assert.ok(evidence.resources.engineCanvasesReleased);
  assert.equal(evidence.resources.comparisonBlobUrls, 0);
  assert.deepEqual(failures, []);
  assert.deepEqual(evidence.externalRequests, []);
  await writeFile(join(artifacts, "comparison-evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
  console.log(JSON.stringify(evidence, null, 2));
  console.log(`Local comparison checks passed${rendererOnly ? " (renderer only)" : ""}.`);
} catch (error) {
  if (page && !page.isClosed()) {
    await writeFile(join(artifacts, "comparison-failure.json"), JSON.stringify({
      error: error.message, browserErrors: failures, body: (await page.locator("body").innerText()).slice(0, 20000),
      controls: await page.locator("select").evaluateAll((elements) => elements.map((element) => ({
        name: element.getAttribute("aria-label"), label: Array.from(element.labels ?? [], (label) => label.textContent), disabled: element.disabled,
      }))),
    }, null, 2) + "\n");
    await page.screenshot({ path: join(artifacts, "comparison-failure.png") });
  }
  throw error;
} finally {
  await browser?.close();
  await frontend?.close();
  for (const child of children.reverse()) {
    if (child.exitCode === null && child.pid) {
      child.kill("SIGTERM");
      await Promise.race([new Promise((resolve) => child.once("exit", resolve)), new Promise((resolve) => setTimeout(resolve, 3000))]);
      if (child.exitCode === null) child.kill("SIGKILL");
    }
  }
  await rm(temporary, { recursive: true, force: true });
}
