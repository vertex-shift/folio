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
import { PDFDocument, PDFDict, PDFName, PDFRawStream, PDFRef } from "pdf-lib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const temporary = await mkdtemp(join(tmpdir(), "folio-archive-"));
const artifacts = process.env.FOLIO_ARCHIVE_ARTIFACTS ? resolve(process.env.FOLIO_ARCHIVE_ARTIFACTS) : temporary;
await mkdir(artifacts, { recursive: true });
const children = [];
const errors = [], externalRequests = [];
const evidence = { visual: [], externalRequests };
let frontend, browser, page;
const env = { ...process.env, CARGO_INCREMENTAL: "0", CARGO_PROFILE_DEV_DEBUG: "0", CARGO_PROFILE_TEST_DEBUG: "0", CARGO_BUILD_JOBS: "2" };
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");

function start(command, args, extra = {}) {
  const child = spawn(command, args, { cwd: root, env: { ...env, ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  child.output = "";
  for (const stream of [child.stdout, child.stderr]) stream.on("data", (data) => { child.output = (child.output + data).slice(-4 * 1024 * 1024); });
  child.on("error", (error) => { child.startError = error; });
  children.push(child); return child;
}
async function command(command, args, validation = false) {
  const child = start(command, args);
  await new Promise((resolve, reject) => { child.on("error", reject); child.on("exit", (code) => code === 0 || validation && code === 1 ? resolve() : reject(new Error(`${command} failed (${code}): ${child.output}`))); });
  return child.output;
}
async function port(preferred) {
  const socket = createServer();
  try { await new Promise((resolve, reject) => { socket.once("error", reject); socket.listen(preferred, "127.0.0.1", resolve); }); }
  catch (error) { if (error.code === "EADDRINUSE" && preferred) return port(0); throw error; }
  const value = socket.address().port;
  await new Promise((resolve) => socket.close(resolve)); return value;
}
async function ready(url, child) {
  const until = Date.now() + 120_000;
  while (Date.now() < until) {
    if (child.startError || child.exitCode !== null) throw new Error(`Archival helper did not start: ${child.startError ?? child.output}`);
    try { if ((await fetch(url, { signal: AbortSignal.timeout(2000) })).ok) return; } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Archival helper is not responsive: ${child.output}`);
}
function accounting() {
  const OriginalWorker = window.Worker;
  window.__archiveResources = { pdfWorkers: 0, canvases: [], urls: new Set() };
  window.Worker = class extends OriginalWorker {
    constructor(url, options) { super(url, options); this.pdf = String(url).includes("pdf.worker"); this.stopped = false; if (this.pdf) window.__archiveResources.pdfWorkers++; }
    terminate() { if (!this.stopped) { this.stopped = true; if (this.pdf) window.__archiveResources.pdfWorkers--; } return super.terminate(); }
  };
  const create = document.createElement.bind(document);
  document.createElement = (...args) => {
    const element = create(...args);
    if (args[0] === "canvas" && new Error().stack?.includes("/src/features/preflight/render.ts")) window.__archiveResources.canvases.push(element);
    return element;
  };
  const createUrl = URL.createObjectURL.bind(URL), revokeUrl = URL.revokeObjectURL.bind(URL);
  URL.createObjectURL = (...args) => { const url = createUrl(...args); window.__archiveResources.urls.add(url); return url; };
  URL.revokeObjectURL = (url) => { window.__archiveResources.urls.delete(url); return revokeUrl(url); };
}
async function validate(path, compliant) {
  const verifier = process.env.FOLIO_VERAPDF;
  if (!verifier) throw new Error("Set FOLIO_VERAPDF to a local veraPDF CLI. This acceptance check never downloads validators or uploads documents.");
  const output = await command(verifier, ["--format", "xml", "--flavour", "2b", path], true);
  await writeFile(`${path}.verapdf.xml`, output);
  const report = output.match(/<validationReport\b[^>]*\bisCompliant="(true|false)"/);
  assert.ok(report, `veraPDF did not produce a validationReport for ${path}`);
  assert.equal(report[1] === "true", compliant, output);
  const failedRules = output.match(/failedRules="(\d+)"/)?.[1];
  const failedChecks = output.match(/failedChecks="(\d+)"/)?.[1];
  if (compliant) { assert.equal(failedRules, "0"); assert.equal(failedChecks, "0"); }
  else { assert.ok(Number(failedRules) > 0 && Number(failedChecks) > 0, "The negative control did not exercise failing rules/checks."); }
  return { path, sha256: sha(await readFile(path)), compliant, failedRules: Number(failedRules), failedChecks: Number(failedChecks) };
}
async function structuralCheck(bytes) {
  const pdf = await PDFDocument.load(bytes, { updateMetadata: false });
  assert.equal(pdf.context.trailerInfo.Encrypt, undefined);
  assert.equal(pdf.context.trailerInfo.Info, undefined);
  assert.equal(pdf.getPageCount(), 4);
  for (const key of ["AcroForm", "StructTreeRoot", "MarkInfo", "Names", "OpenAction", "Outlines", "Perms"]) assert.equal(pdf.catalog.has(PDFName.of(key)), false);
  const intent = pdf.catalog.lookup(PDFName.of("OutputIntents")).lookup(0, PDFDict);
  const profile = intent.lookup(PDFName.of("DestOutputProfile"), PDFRawStream);
  assert.equal(sha(profile.contents), "384b832de3412066743b52a75ee906b6fb9fb8d9e09e936fc2c43223815c6e0a");
  const metadata = pdf.catalog.lookup(PDFName.of("Metadata"), PDFRawStream).contents;
  const xml = new TextDecoder("utf-8", { fatal: true }).decode(metadata);
  assert.match(xml, /<pdfaid:part>2<\/pdfaid:part>/);
  assert.match(xml, /<pdfaid:conformance>B<\/pdfaid:conformance>/);
  assert.doesNotMatch(xml, /ORIGINAL|pdfuaid/);
  let images = 0;
  for (const [, object] of pdf.context.enumerateIndirectObjects()) {
    const dictionary = object instanceof PDFRawStream ? object.dict : object instanceof PDFDict ? object : null;
    if (!dictionary) continue;
    assert.notEqual(dictionary.get(PDFName.of("Type"))?.toString(), "/Font");
    assert.notEqual(dictionary.get(PDFName.of("Type"))?.toString(), "/Sig");
    assert.notEqual(dictionary.get(PDFName.of("Type"))?.toString(), "/EmbeddedFile");
    if (dictionary.get(PDFName.of("Subtype"))?.toString() === "/Image") images++;
  }
  assert.equal(images, 4);
  return { pages: 4, imageObjects: images, profileSha256: sha(profile.contents), utf8Xmp: true, sourceObjectsAbsent: true };
}

try {
  const frontendPort = await port(1435), backendPort = await port(19163);
  evidence.ports = { frontendPort, backendPort };
  await command("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--quiet"]);
  const backend = start(join(root, "src-tauri/target/debug/folio-preview"), [], { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) });
  await ready(`http://127.0.0.1:${backendPort}/health`, backend);
  frontend = await createViteServer({ root, logLevel: "error", optimizeDeps: { include: ["pdf-lib"] },
    server: { host: "127.0.0.1", port: frontendPort, strictPort: true,
      proxy: { "/__folio": { target: `http://127.0.0.1:${backendPort}`, rewrite: (path) => path.replace(/^\/__folio/, "") } } } });
  await frontend.listen();
  const origin = `http://127.0.0.1:${frontendPort}`;
  const api = async (route, value, headers = {}, body = undefined) => {
    const response = await fetch(`http://127.0.0.1:${backendPort}${route}`, {
      headers: { "X-Folio-Preview": "1", Origin: origin, ...(value ? { "Content-Type": "application/json" } : {}), ...headers },
      ...(value || body ? { method: "POST", body: body ?? JSON.stringify(value) } : {}),
    });
    return response;
  };
  const json = async (route, value) => { const response = await api(route, value); assert.ok(response.ok, `${route}: ${await response.clone().text()}`); return response.json(); };
  const sourceBytes = async (info) => {
    const response = await api("/bytes", null, { "X-Document-Id": String(info.id), "X-Revision": String(info.revision) });
    assert.ok(response.ok, await response.clone().text()); return new Uint8Array(await response.arrayBuffer());
  };
  browser = await chromium.launch({ headless: true, executablePath: process.env.FOLIO_ARCHIVE_BROWSER
    ?? "/Users/adityavaish/.copilot/session-state/19181025-f105-4d9f-90c9-1c035d6e1f52/files/comparison-browser-manual/chrome-mac/headless_shell" });
  evidence.browserVersion = browser.version();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce", acceptDownloads: true });
  await context.addInitScript(accounting);
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    externalRequests.push(route.request().url()); return route.abort();
  });
  page = await context.newPage();
  page.setDefaultTimeout(30_000);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(origin);
  await page.getByRole("img", { name: "Rendered PDF page 1", exact: true }).waitFor();
  const fixtures = await page.evaluate(async () => (await import("/tests/preflightArchive.browser.ts")).archiveFixtures());
  for (const [name, bytes] of Object.entries(fixtures)) await writeFile(join(artifacts, `synthetic-${name}.pdf`), new Uint8Array(bytes));
  await page.getByLabel("Choose PDF files").setInputFiles(join(artifacts, "synthetic-source.pdf"));
  await page.getByRole("tab", { name: /synthetic-source\.pdf/ }).waitFor();
  let snapshot = await json("/bootstrap");
  const info = snapshot.documents.find((document) => document.name === "synthetic-source.pdf");
  assert.ok(info);
  const originalHash = sha(await sourceBytes(info));
  const sourceState = JSON.stringify(info);
  const allTools = page.getByRole("button", { name: "All tools", exact: true });
  if (await allTools.getAttribute("aria-expanded") !== "true") await allTools.click();
  await page.getByRole("button", { name: "Technical preflight", exact: true }).click();
  await page.getByRole("button", { name: "Inspect current PDF", exact: true }).click();
  await page.locator(".preflight-report-source").waitFor();
  assert.match(await page.locator(".preflight-panel").innerText(), /NOT VALIDATED/);
  assert.match(await page.locator(".preflight-panel").innerText(), /UNVERIFIED declaration/);
  const downloadReport = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export JSON report", exact: true }).click();
  const reportPath = join(artifacts, "actual-ui-preflight.json");
  await (await downloadReport).saveAs(reportPath);
  const report = JSON.parse(await readFile(reportPath, "utf8"));
  assert.equal(report.documentId, info.id); assert.equal(report.revision, info.revision);
  assert.ok(report.findings.some((finding) => finding.category === "fonts" && finding.state === "absent"));
  assert.ok(report.findings.some((finding) => finding.property === "pdfuaid:part" && finding.state === "fact"));
  evidence.preflight = { path: reportPath, observations: report.findings.length, inputConformance: report.inputConformance };
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some((button) => button.textContent === "Inspect current PDF" && !button.disabled));
  const toast = page.getByRole("button", { name: "Dismiss notification", exact: true });
  if (await toast.isVisible()) await toast.click();

  // The single batched visual round covers both surfaces, both viewports and all themes.
  if (!process.env.FOLIO_ARCHIVE_SKIP_VISUAL) {
    for (const theme of ["system", "light", "dark", "sepia", "contrast"]) {
      await page.evaluate((value) => { document.documentElement.dataset.theme = value === "system" ? "light" : value; }, theme);
      for (const [label, width, height] of [["desktop", 1440, 1000], ["compact", 700, 640]]) {
        await page.setViewportSize({ width, height });
        await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
        if (!await page.locator(".preflight-panel").isVisible()) {
          await allTools.click();
          await page.getByRole("button", { name: "Technical preflight", exact: true }).click();
          await page.getByRole("button", { name: "Inspect current PDF", exact: true }).click();
          await page.locator(".preflight-report-source").waitFor();
        }
        await page.locator(".preflight-panel").evaluate((element) => { element.scrollTop = 0; });
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${theme}/${label} horizontal overflow`);
        await page.screenshot({ path: join(artifacts, `preflight-${theme}-${label}.png`) });
        await page.getByRole("button", { name: "Create archival copy...", exact: true }).click();
        const dialog = page.getByRole("dialog", { name: "Create PDF/A-2b archival copy" });
        await dialog.waitFor();
        assert.ok(await dialog.locator(".dialog-actions").evaluate((element) => {
          const bounds = element.getBoundingClientRect(); return bounds.top >= 0 && bounds.bottom <= innerHeight;
        }), "Archive actions left the visible viewport.");
        await page.getByRole("checkbox", { name: /I accept rasterization/ }).focus();
        assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("type")), "checkbox");
        await page.screenshot({ path: join(artifacts, `archive-${theme}-${label}.png`) });
        await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
        await dialog.waitFor({ state: "detached" });
        evidence.visual.push(`${theme}/${label}`);
      }
    }
    await page.evaluate(() => { document.documentElement.dataset.theme = "light"; });
    await page.setViewportSize({ width: 1440, height: 1000 });
  }
  evidence.archives = [];
  for (const dpi of [150, 300]) {
    await page.getByRole("button", { name: "Create archival copy...", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Create PDF/A-2b archival copy" });
    await dialog.waitFor();
    await dialog.getByRole("combobox", { name: /Resolution/ }).selectOption(String(dpi));
    assert.equal(await dialog.getByRole("button", { name: "Create archival copy", exact: true }).isEnabled(), false);
    await dialog.getByRole("checkbox", { name: /I accept rasterization/ }).check();
    const beforeWorkers = await page.evaluate(() => window.__archiveResources.pdfWorkers);
    const downloaded = page.waitForEvent("download", { timeout: 90_000 });
    await dialog.getByRole("button", { name: "Create archival copy", exact: true }).click();
    const path = join(artifacts, `actual-ui-archive-${dpi}.pdf`);
    const download = await downloaded;
    await download.saveAs(path);
    await dialog.waitFor({ state: "detached", timeout: 90_000 });
    const bytes = await readFile(path);
    const structure = await structuralCheck(bytes);
    const pixels = await page.evaluate(async ({ source, copy, dpi }) =>
      (await import("/tests/preflightArchive.browser.ts")).compareArchivePixels(source, copy, dpi),
    { source: fixtures.source, copy: [...bytes], dpi });
    const validation = await validate(path, true);
    assert.equal(await page.evaluate(() => window.__archiveResources.pdfWorkers), beforeWorkers, "An isolated archive PDF.js worker leaked.");
    assert.ok(await page.evaluate(() => window.__archiveResources.canvases.every((canvas) => canvas.width === 0 && canvas.height === 0)), "Archival canvas pixels were retained.");
    assert.equal(sha(await sourceBytes(info)), originalHash);
    snapshot = await json("/bootstrap");
    assert.equal(JSON.stringify(snapshot.documents.find((document) => document.id === info.id)), sourceState);
    evidence.archives.push({ dpi, structure, pixels, validation });
  }
  evidence.negativeControls = [await validate(join(artifacts, "synthetic-source.pdf"), false)];
  const actual = await readFile(join(artifacts, "actual-ui-archive-150.pdf"));
  for (const kind of ["missing-profile", "invalid-profile", "missing-xmp", "false-xmp"]) {
    const pdf = await PDFDocument.load(actual, { updateMetadata: false });
    const intent = pdf.catalog.lookup(PDFName.of("OutputIntents")).lookup(0, PDFDict);
    const profileRef = intent.get(PDFName.of("DestOutputProfile"));
    assert.ok(profileRef instanceof PDFRef);
    if (kind === "missing-profile") {
      intent.delete(PDFName.of("DestOutputProfile"));
      for (const [, object] of pdf.context.enumerateIndirectObjects()) {
        if (object instanceof PDFRawStream && object.dict.get(PDFName.of("Subtype"))?.toString() === "/Image") object.dict.set(PDFName.of("ColorSpace"), PDFName.of("DeviceRGB"));
      }
      pdf.context.delete(profileRef);
    }
    if (kind === "invalid-profile") pdf.context.assign(profileRef, pdf.context.stream(new Uint8Array(3024), { N: 3 }));
    if (kind === "missing-xmp") pdf.catalog.delete(PDFName.of("Metadata"));
    if (kind === "false-xmp") pdf.catalog.set(PDFName.of("Metadata"), pdf.context.register(pdf.context.stream(new TextEncoder().encode(
      '<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/" pdfaid:part="2" pdfaid:conformance="Z"/></rdf:RDF></x:xmpmeta>'
    ), { Type: "Metadata", Subtype: "XML" })));
    const path = join(artifacts, `negative-${kind}.pdf`);
    await writeFile(path, await pdf.save({ useObjectStreams: false }));
    evidence.negativeControls.push(await validate(path, false));
  }
  const plan = await json("/archive/begin", { documentId: info.id, revision: info.revision, pages: [0, 1, 2, 3], dpi: 150, acknowledgeLoss: true, acknowledgeUnsigned: true, allowUnencrypted: false });
  evidence.cancellations = await page.evaluate(async ({ source, plan }) =>
    (await import("/tests/preflightArchive.browser.ts")).cancellationAndGeometryChecks(source, plan), { source: fixtures.source, plan });
  await json("/archive/cancel", { archiveId: plan.archiveId });
  await json("/draft", { documentId: info.id, pending: true });
  assert.equal((await api("/archive/check", { archiveId: plan.archiveId })).ok, false);
  await json("/draft", { documentId: info.id, pending: false });
  assert.equal((await api("/preflight/check", { inspectionId: report.inspectionId })).ok, false);
  for (const kind of ["boundary", "xfa"]) {
    const response = await api("/open", null, { "X-File-Name": `synthetic-${kind}.pdf`, "Content-Type": "application/pdf" }, new Uint8Array(fixtures[kind]));
    assert.ok(response.ok, await response.clone().text());
    const opened = (await response.json()).document;
    const request = { documentId: opened.id, revision: opened.revision, pages: [0], dpi: 150, acknowledgeLoss: true, acknowledgeUnsigned: true, allowUnencrypted: false };
    if (kind === "xfa") assert.equal((await api("/archive/begin", request)).ok, false);
    else {
      const plan = await json("/archive/begin", request);
      let message;
      try {
        await page.evaluate(async ({ source, plan }) => {
          const { loadPdf } = await import("/src/lib/pdf.ts");
          const { renderArchivePages } = await import("/src/features/preflight/render.ts");
          const loading = loadPdf(new Uint8Array(source));
          try { await renderArchivePages(await loading.promise, plan, async () => { throw new Error("Unexpected stage"); }, () => {}, new AbortController().signal, () => {}); }
          finally { await loading.destroy(); }
        }, { source: fixtures.boundary, plan });
      } catch (error) { message = error.message; }
      assert.match(message, /inconsistent or unsupported|pixel-rounding boundary/);
      await json("/archive/cancel", { archiveId: plan.archiveId });
      evidence.ceilBoundaryRefused = true;
    }
  }
  await page.getByLabel("Choose PDF files").setInputFiles(join(artifacts, "actual-ui-archive-150.pdf"));
  await page.getByRole("tab", { name: /actual-ui-archive-150\.pdf/ }).waitFor();
  await page.getByText(/Folio archival raster copy: read-only/).waitFor();
  evidence.reopenedReadOnly = true;
  const protectedFixtures = join(temporary, "protected-fixtures");
  await command("cargo", ["run", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--example", "archive_fixtures", "--quiet", "--", protectedFixtures]);
  const openBytes = async (name, bytes) => {
    const response = await api("/open", null, { "X-File-Name": name, "Content-Type": "application/pdf" }, bytes);
    assert.ok(response.ok, await response.clone().text()); return response.json();
  };
  const archiveRequest = (source, extra = {}) => ({ documentId: source.id, revision: source.revision, pages: [0], dpi: 150,
    acknowledgeLoss: true, acknowledgeUnsigned: false, allowUnencrypted: false, ...extra });
  const assertRefused = async (route, value, pattern) => {
    const response = await api(route, value); assert.equal(response.ok, false);
    const error = await response.json(); assert.match(error.error, pattern); return error.error;
  };
  const ownerBytes = await readFile(join(protectedFixtures, "owner-required.pdf"));
  const challenge = await openBytes("owner-required.pdf", ownerBytes);
  assert.equal(challenge.status, "password_required");
  const wrong = await json("/unlock", { requestId: challenge.requestId, password: "wrong" });
  assert.equal(wrong.status, "password_required"); assert.equal(wrong.incorrect, true);
  const user = (await json("/unlock", { requestId: challenge.requestId, password: "user" })).document;
  assert.equal(user.permissions.ownerAuthenticated, false); assert.equal(user.permissions.copyAllowed, true);
  const userRefusal = await assertRefused("/archive/begin", archiveRequest(user, { allowUnencrypted: true }), /actual owner password/);
  const userReport = await json("/preflight/inspect", { documentId: user.id, revision: user.revision });
  await assertRefused("/preflight/report", { inspectionId: userReport.inspectionId, allowUnencrypted: false }, /unencrypted document properties/);
  assert.ok((await api("/preflight/report", { inspectionId: userReport.inspectionId, allowUnencrypted: true })).ok);
  const noOwner = await openBytes("no-owner-authentication.pdf", ownerBytes);
  assert.equal(noOwner.status, "password_required");
  await json("/open/cancel", { requestId: noOwner.requestId });
  const ownerChallenge = await openBytes("owner.pdf", ownerBytes);
  const owner = (await json("/unlock", { requestId: ownerChallenge.requestId, password: "owner" })).document;
  assert.equal(owner.permissions.ownerAuthenticated, true);
  await assertRefused("/archive/begin", archiveRequest(owner), /acknowledge an unencrypted/);
  const restrictedChallenge = await openBytes("copy-restricted.pdf", await readFile(join(protectedFixtures, "copy-restricted.pdf")));
  const restricted = (await json("/unlock", { requestId: restrictedChallenge.requestId, password: "user" })).document;
  await assertRefused("/archive/begin", archiveRequest(restricted, { allowUnencrypted: true }), /does not allow copying/);
  await assertRefused("/preflight/inspect", { documentId: restricted.id, revision: restricted.revision }, /does not allow copying/);
  evidence.protection = { wrongPasswordRefused: true, noOwnerAuthorizationRefused: true, permissiveOpeningPasswordRefused: userRefusal,
    copyRestrictionsEnforced: true, reportDisclosureEnforced: true };
  evidence.additionalArchives = [];
  const signedBytes = await readFile(join(protectedFixtures, "cryptographically-signed.pdf"));
  await writeFile(join(artifacts, "synthetic-cryptographically-signed.pdf"), signedBytes);
  const signed = (await openBytes("cryptographically-signed.pdf", signedBytes)).document;
  assert.match(signed.readOnlyReason, /Digitally signed/);
  const signatureReport = await json("/signatures/inspect", { documentId: signed.id, revision: signed.revision });
  assert.equal(signatureReport.signatures.length, 1);
  assert.equal(signatureReport.signatures[0].integrity, "intact");
  assert.equal(signatureReport.signatures[0].coverage, "whole_document");
  evidence.signedSourceReport = signatureReport;
  await assertRefused("/archive/begin", archiveRequest(signed), /Explicitly acknowledge a fresh unsigned/);
  for (const [name, source, request] of [
    ["owner-authorized", owner, archiveRequest(owner, { allowUnencrypted: true })],
    ["signed-unsigned-copy", signed, archiveRequest(signed, { acknowledgeUnsigned: true })],
  ]) {
    const before = sha(await sourceBytes(source));
    const plan = await json("/archive/begin", request);
    const downloaded = page.waitForEvent("download", { timeout: 90_000 });
    const rendering = page.evaluate(async (plan) => (await import("/tests/preflightArchive.browser.ts")).renderAndDownloadArchive(plan), plan);
    const path = join(artifacts, `actual-engine-download-${name}.pdf`);
    await (await downloaded).saveAs(path); await rendering;
    evidence.additionalArchives.push(await validate(path, true));
    const pdf = await PDFDocument.load(await readFile(path), { updateMetadata: false });
    assert.equal(pdf.context.trailerInfo.Encrypt, undefined);
    assert.equal(pdf.catalog.has(PDFName.of("AcroForm")), false);
    assert.equal(sha(await sourceBytes(source)), before);
  }
  await assertRefused("/edit", { documentId: signed.id, revision: signed.revision, edit: { type: "rotate", page: 0, degrees: 90 } }, /Digitally signed/);
  evidence.protection.signedSourceStillReadOnly = true;
  assert.deepEqual(errors, []);
  assert.deepEqual(externalRequests, []);
  evidence.sourceHashBefore = originalHash;
  evidence.sourceHashAfter = sha(await sourceBytes(info));
  evidence.resources = await page.evaluate(() => ({
    retainedArchivalCanvases: window.__archiveResources.canvases.filter((canvas) => canvas.width || canvas.height).length,
    retainedDownloadUrls: window.__archiveResources.urls.size,
  }));
  assert.equal(evidence.resources.retainedArchivalCanvases, 0);
  assert.equal(evidence.resources.retainedDownloadUrls, 0);
  await writeFile(join(artifacts, "acceptance.json"), JSON.stringify(evidence, null, 2) + "\n");
  await rm(join(artifacts, "failure.json"), { force: true });
  await rm(join(artifacts, "failure-dom.txt"), { force: true });
  console.log(JSON.stringify(evidence, null, 2));
} catch (error) {
  if (page && !page.isClosed()) await writeFile(join(artifacts, "failure-dom.txt"), await page.locator("body").innerText());
  await writeFile(join(artifacts, "failure.json"), JSON.stringify({ message: error.message, stack: error.stack, evidence, errors }, null, 2) + "\n");
  throw error;
} finally {
  if (browser) await browser.close();
  if (frontend) await frontend.close();
  for (const child of children) if (child.exitCode === null) {
    child.kill("SIGTERM");
    await new Promise((resolve) => child.once("exit", resolve));
  }
  await rm(temporary, { recursive: true, force: true });
}
