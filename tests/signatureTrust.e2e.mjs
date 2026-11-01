import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { PDFDict, PDFDocument, PDFName } from "pdf-lib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const fixtureRoot = process.env.FOLIO_TRUST_FIXTURES;
if (!fixtureRoot) throw new Error("Set FOLIO_TRUST_FIXTURES to the original public native-signature/CRL/RFC3161 fixture directory. No private keys or P12s are read by this test.");
const fixture = (name) => join(resolve(fixtureRoot), name);
const temporary = await mkdtemp(join(tmpdir(), "folio-signature-trust-"));
const artifacts = process.env.FOLIO_TRUST_ARTIFACTS ? resolve(process.env.FOLIO_TRUST_ARTIFACTS) : temporary;
await mkdir(artifacts, { recursive: true });
const children = [];
const evidence = { externalRequests: [], checks: [], screenshots: [], sourceHashes: {}, layouts: [] };
const env = { ...process.env, CARGO_INCREMENTAL: "0", CARGO_PROFILE_DEV_DEBUG: "0",
  CARGO_PROFILE_TEST_DEBUG: "0", CARGO_BUILD_JOBS: "2" };
let browser, page;
let releaseHeld;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const ref = (info) => ({ documentId: info.id, revision: info.revision });

function start(command, args, extra = {}) {
  const child = spawn(command, args, { cwd: root, env: { ...env, ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  child.output = "";
  for (const stream of [child.stdout, child.stderr]) stream.on("data", (data) => { child.output = (child.output + data).slice(-24000); });
  child.on("error", (error) => { child.startError = error; });
  children.push(child);
  return child;
}

async function command(executable, args) {
  const child = start(executable, args);
  await new Promise((done, fail) => {
    child.once("error", fail);
    child.once("exit", (code) => code === 0 ? done() : fail(new Error(`${executable} failed (${code}): ${child.output}`)));
  });
  return child.output.trim();
}

async function freePort() {
  const server = createServer();
  await new Promise((done, fail) => { server.once("error", fail); server.listen(0, "127.0.0.1", done); });
  const port = server.address().port;
  await new Promise((done) => server.close(done));
  return port;
}

async function ready(url, child) {
  const until = Date.now() + 120_000;
  while (Date.now() < until) {
    if (child.startError || child.exitCode !== null) throw new Error(`Owned preview failed: ${child.startError ?? child.output}`);
    try { if ((await fetch(url, { signal: AbortSignal.timeout(1500) })).ok) return; } catch {}
    await new Promise((done) => setTimeout(done, 100));
  }
  throw new Error(`Owned preview did not become responsive: ${child.output}`);
}

async function independentlyVerify(bytes, name) {
  const document = await PDFDocument.load(bytes, { updateMetadata: false });
  let count = 0;
  for (const [, value] of document.context.enumerateIndirectObjects()) {
    if (!(value instanceof PDFDict) || value.get(PDFName.of("Type"))?.toString() !== "/Sig") continue;
    const [start, first, second, length] = value.lookup(PDFName.of("ByteRange")).asArray().map((number) => number.asNumber());
    assert.equal(start, 0);
    const content = Buffer.concat([bytes.slice(0, first), bytes.slice(second, second + length)]);
    const cms = value.lookup(PDFName.of("Contents")).asBytes();
    const cmsPath = join(temporary, `${name}-${count}.cms`);
    const dataPath = join(temporary, `${name}-${count}.data`);
    const output = join(temporary, `${name}-${count}.verified`);
    await writeFile(cmsPath, cms); await writeFile(dataPath, content);
    await command("openssl", ["cms", "-verify", "-binary", "-inform", "DER", "-noverify", "-in", cmsPath,
      "-content", dataPath, "-out", output]);
    assert.deepEqual(await readFile(output), content);
    count++;
  }
  assert.ok(count > 0);
  evidence.checks.push({ independentCms: name, signatures: count });
}

try {
  evidence.openssl = await command("openssl", ["version"]);
  for (const name of ["good-signer", "revoked-signer"]) {
    await command("openssl", ["ts", "-verify", "-data", fixture(`actual-native-${name}.signature-value.bin`),
      "-in", fixture(`actual-native-${name}.tsr`), "-CAfile", fixture("root.pem"),
      "-untrusted", fixture("issuer.pem")]);
  }
  await command("openssl", ["verify", "-trusted", fixture("root.pem"), "-untrusted", fixture("issuer.pem"),
    "-CRLfile", fixture("current-complete-chain-crls.pem"), "-crl_check_all",
    "-no-CAfile", "-no-CApath", "-no-CAstore", fixture("good-signer.pem")]);
  evidence.checks.push({ independentTimestampImprints: 2, independentCompleteCurrentCrlPath: true });
  if (process.env.FOLIO_TRUST_SKIP_BUILD !== "1") {
    await command("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--quiet"]);
  }
  const backendPort = await freePort(), frontendPort = await freePort();
  const ports = { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) };
  const target = resolve(root, env.CARGO_TARGET_DIR ?? "src-tauri/target");
  const backend = start(join(target, "debug", `folio-preview${process.platform === "win32" ? ".exe" : ""}`), [], ports);
  await ready(`http://127.0.0.1:${backendPort}/health`, backend);
  const frontend = start(process.execPath, ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--strictPort"], ports);
  const origin = `http://127.0.0.1:${frontendPort}`;
  await ready(origin, frontend);
  evidence.ports = ports;
  const api = async (route, value, extra = {}, expected = 200) => {
    const response = await fetch(`http://127.0.0.1:${backendPort}${route}`, {
      ...(value ? { method: "POST", body: JSON.stringify(value) } : {}),
      headers: { "X-Folio-Preview": "1", ...(value ? { "Content-Type": "application/json" } : {}), ...extra },
    });
    assert.equal(response.status, expected, `${route}: ${await response.clone().text()}`);
    return response;
  };
  const snapshot = async () => (await api("/bootstrap")).json();
  const inspect = async (info) => (await api("/signatures/inspect", ref(info))).json();
  const sourceBytes = async (info) => Buffer.from(await (await api("/bytes", null,
    { "X-Document-Id": String(info.id), "X-Revision": String(info.revision) })).arrayBuffer());
  browser = await chromium.launch({ headless: true,
    ...(process.env.FOLIO_SIGNATURE_BROWSER ? { executablePath: process.env.FOLIO_SIGNATURE_BROWSER } : {}) });
  evidence.browser = browser.version();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce" });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url());
    return route.abort();
  });
  page = await context.newPage();
  page.setDefaultTimeout(20_000);
  const errors = [];
  let evidenceAdds = 0;
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => { if (request.url().endsWith("/__folio/signatures/evidence/add")) evidenceAdds++; });
  await page.goto(origin);
  const active = () => page.locator(".document-workspace:not([hidden])");
  const statuses = () => active().locator(".signature-evidence-result > h4");
  const waitStatus = (name) => statuses().filter({ hasText: name }).waitFor();
  const opened = [];

  async function openFile(path, name) {
    const [chooser] = await Promise.all([page.waitForEvent("filechooser"),
      page.getByRole("button", { name: "Open PDFs in new tabs", exact: true }).click()]);
    const [response] = await Promise.all([page.waitForResponse((response) => response.url().endsWith("/__folio/open") && response.request().method() === "POST"),
      chooser.setFiles(path)]);
    assert.equal(response.status(), 200);
    const info = (await snapshot()).documents.find((document) => document.name === name);
    assert.ok(info, name);
    await page.getByRole("tab").filter({ hasText: name }).waitFor();
    await page.waitForFunction((name) => [...document.querySelectorAll(".document-tab.selected")].some((tab) => tab.textContent.includes(name)), name);
    opened.push(info);
    return info;
  }

  async function showSignatures() {
    await active().getByRole("button", { name: "Show digital signatures", exact: true }).click();
    await active().locator(".signature-entry, .panel-empty h3").first().waitFor();
    await page.waitForFunction(() => !document.querySelector(".document-workspace:not([hidden]) .signature-progress"));
  }

  async function disclosure(selector) {
    const details = active().locator(selector);
    if (await details.getAttribute("open") === null) await details.locator(":scope > summary").click();
  }

  async function importFile(kind, path, expected = 200) {
    if (kind === "anchor") await disclosure(".signature-anchors:not(.signature-evidence-store)");
    if (kind === "crl") await disclosure(".signature-evidence-store");
    const name = kind === "anchor" ? "Add public root CA" : kind === "crl" ? "Import full CRL" : "Import timestamp for signature 1";
    const [chooser] = await Promise.all([page.waitForEvent("filechooser"), active().getByRole("button", { name, exact: true }).click()]);
    const [response] = await Promise.all([
      page.waitForResponse((response) => response.url().endsWith(`/__folio/signatures/${kind === "anchor" ? "anchors" : "evidence"}/add`)),
      chooser.setFiles(path),
    ]);
    assert.equal(response.status(), expected, await response.text());
    await page.waitForFunction(() => !document.querySelector(".document-workspace:not([hidden]) .signature-progress"));
    await active().getByText("Signed bytes intact", { exact: true }).waitFor();
  }

  const goodName = "actual-native-good-signer.pdf";
  const good = await openFile(fixture(goodName), goodName);
  const goodBytes = await readFile(fixture(goodName));
  await independentlyVerify(goodBytes, "original-good");
  await showSignatures();
  await waitStatus("Revocation not checked");
  await disclosure(".signature-anchors:not(.signature-evidence-store)");
  const [cancel] = await Promise.all([page.waitForEvent("filechooser"), active().getByRole("button", { name: "Add public root CA", exact: true }).click()]);
  await cancel.setFiles([]);
  await active().getByLabel("Choose public root CA certificate", { exact: true }).dispatchEvent("cancel");
  assert.equal((await inspect(good)).trustStore.revision, 0);
  await active().getByText(/selection canceled/).waitFor();
  await importFile("anchor", fixture("root.pem"));
  await waitStatus("Revocation evidence incomplete");
  await importFile("crl", fixture("issuer-current.crl.pem"));
  assert.equal((await inspect(good)).signatures[0].revocation.status, "incomplete");
  await importFile("crl", fixture("root-current.crl.pem"));
  await waitStatus("No revocation listed in current imported CRLs");
  await importFile("timestamp", fixture("actual-native-good-signer.tsr"));
  await waitStatus("Timestamp verified with custom evidence");
  const beforeWrong = await inspect(good);
  await importFile("timestamp", fixture("actual-native-revoked-signer.tsr"), 400);
  await active().getByRole("alert").filter({ hasText: /message imprint does not match/ }).waitFor();
  assert.equal((await inspect(good)).trustStore.revision, beforeWrong.trustStore.revision);
  const [saved] = await Promise.all([page.waitForEvent("download"),
    active().getByRole("button", { name: "Download PDF", exact: true }).click()]);
  const savedPath = join(artifacts, "Actual-native-signed-source-saved-exactly.pdf");
  await saved.saveAs(savedPath);
  assert.deepEqual(await readFile(savedPath), goodBytes);
  await independentlyVerify(await readFile(savedPath), "saved-good");

  const controls = [
    ["actual-native-good-signer-embedded-time.pdf", "intact", "verified_custom_store", "Timestamp verified with custom evidence"],
    ["actual-native-good-signer-wrong-imprint.pdf", "intact", "invalid", "Timestamp verification failed"],
    ["actual-native-good-signer-untrusted-time.pdf", "intact", "untrusted", "Timestamp TSA path is not trusted"],
    ["actual-native-good-signer-future-time.pdf", "intact", "future", "Timestamp time is in the future"],
    ["actual-native-good-signer-unknown-attribute.pdf", "unsupported", "not_checked", "Trusted time not checked"],
  ];
  for (const [name, integrity, time, label] of controls) {
    const info = await openFile(fixture(name), name);
    await showSignatures(); await waitStatus(label);
    const report = await inspect(info);
    assert.equal(report.signatures[0].integrity, integrity);
    assert.equal(report.signatures[0].trustedTime.status, time);
    if (time === "future") {
      assert.equal(report.signatures[0].trustedTime.timestamps[0].cryptographicIntegrity, "intact");
      assert.equal(report.signatures[0].trustedTime.timestamps[0].imprintMatches, true);
    }
    if (integrity === "unsupported") assert.ok(await active().getByRole("button", { name: "Import timestamp for signature 1", exact: true }).isDisabled());
    assert.deepEqual(await sourceBytes(info), await readFile(fixture(name)));
    evidence.checks.push({ realPdf: name, integrity, trustedTime: time, exactSourceHash: hash(await sourceBytes(info)) });
  }
  const revokedName = "actual-native-revoked-signer.pdf";
  const revoked = await openFile(fixture(revokedName), revokedName);
  await showSignatures();
  await waitStatus("Certificate revoked in current imported evidence");
  await importFile("timestamp", fixture("actual-native-revoked-signer.tsr"));
  await waitStatus("Timestamp verified with custom evidence");
  assert.equal((await inspect(revoked)).signatures[0].revocation.status, "revoked");
  evidence.checks.push({ validTimeNeverClearsSignerRevocation: true });

  await page.getByRole("tab").filter({ hasText: goodName }).click();
  await waitStatus("Timestamp verified with custom evidence");
  let captureHeld;
  const captured = new Promise((done) => { captureHeld = done; });
  const held = new Promise((done) => { releaseHeld = done; });
  let holdNext = true;
  await page.route("**/__folio/signatures/inspect", async (route) => {
    const request = route.request().postDataJSON();
    if (!holdNext || request.documentId !== good.id) return route.continue();
    holdNext = false;
    const response = await route.fetch();
    captureHeld(await response.json());
    await held;
    await route.fulfill({ response });
  });
  await active().getByRole("button", { name: "Refresh checks", exact: true }).click();
  const old = await captured;
  await page.getByRole("tab").filter({ hasText: revokedName }).click();
  await waitStatus("Timestamp verified with custom evidence");
  const issuer = old.trustStore.evidence.find((item) => item.kind === "crl" && item.issuer.includes("intermediate"));
  assert.ok(issuer);
  await disclosure(".signature-evidence-store");
  await active().getByRole("button", { name: `Remove imported crl ${issuer.fingerprint}`, exact: true }).click();
  await waitStatus("Timestamp TSA evidence incomplete");
  releaseHeld();
  await page.unroute("**/__folio/signatures/inspect");
  assert.equal(await page.locator(".signature-evidence-result > h4").filter({ hasText: "Timestamp verified with custom evidence" }).count(), 0);
  await page.getByRole("tab").filter({ hasText: goodName }).click();
  await waitStatus("Timestamp TSA evidence incomplete");
  await api("/signatures/evidence/remove", { ...ref(good), epoch: old.epoch, trustRevision: old.trustStore.revision, fingerprint: issuer.fingerprint }, {}, 400);
  await importFile("crl", fixture("issuer-current.crl.pem"));
  await waitStatus("Timestamp verified with custom evidence");
  let current = await inspect(good);
  await api("/draft", { documentId: good.id, pending: true });
  await api("/signatures/evidence/remove", { ...ref(good), epoch: current.epoch, trustRevision: current.trustStore.revision,
    fingerprint: current.trustStore.evidence.find((item) => item.kind === "timestamp").fingerprint }, {}, 400);
  await api("/draft", { documentId: good.id, pending: false });
  await active().getByRole("button", { name: "Refresh checks", exact: true }).click();
  await waitStatus("Timestamp verified with custom evidence");
  evidence.checks.push({ removalInvalidatesInactiveAndLateReports: true, staleStoreAndSameRevisionDraftEpochRefused: true });

  const draftPdf = await PDFDocument.create();
  const draftPage = draftPdf.addPage([320, 240]);
  draftPdf.getForm().createTextField("Evidence draft retained").addToPage(draftPage, { x: 30, y: 140, width: 240, height: 32 });
  const draftPath = join(artifacts, "Evidence-draft-fixture.pdf");
  const draftBytes = Buffer.from(await draftPdf.save({ useObjectStreams: false }));
  await writeFile(draftPath, draftBytes);
  const draft = await openFile(draftPath, "Evidence-draft-fixture.pdf");
  await active().getByRole("button", { name: "Show form fields", exact: true }).click();
  await active().getByRole("textbox", { name: "Evidence draft retained", exact: true }).fill("Protected unapplied evidence-context draft");
  await showSignatures();
  assert.ok(await active().getByRole("button", { name: "Export signed copy", exact: true }).isDisabled());
  current = await inspect(draft);
  await disclosure(".signature-evidence-store");
  const [stalePicker] = await Promise.all([page.waitForEvent("filechooser"),
    active().getByRole("button", { name: "Import full CRL", exact: true }).click()]);
  const addsBeforeInactive = evidenceAdds;
  await page.getByRole("tab").filter({ hasText: goodName }).click();
  await stalePicker.setFiles(fixture("issuer-current.crl.pem"));
  await page.locator(".document-workspace[hidden] .signature-failure").filter({ hasText: /draft lifetime.*changed/ }).waitFor({ state: "attached" });
  assert.equal(evidenceAdds, addsBeforeInactive);
  assert.equal((await inspect(good)).trustStore.revision, current.trustStore.revision);
  await page.getByRole("tab").filter({ hasText: "Evidence-draft-fixture.pdf" }).click();
  await active().getByRole("button", { name: "Show form fields", exact: true }).click();
  assert.equal(await active().getByRole("textbox", { name: "Evidence draft retained", exact: true }).inputValue(), "Protected unapplied evidence-context draft");
  evidence.checks.push({ inactivePickerNeverImports: true, realFormDraftPreserved: true });

  await page.getByRole("tab").filter({ hasText: goodName }).click();
  await waitStatus("Timestamp verified with custom evidence");
  const freshResponse = page.waitForResponse((response) => response.url().endsWith("/__folio/signatures/inspect")
    && response.request().postDataJSON().documentId === good.id && response.status() === 200);
  await active().getByRole("button", { name: "Refresh checks", exact: true }).click();
  current = await (await freshResponse).json();
  await waitStatus("Timestamp verified with custom evidence");
  const beforeExpiry = current.checkedAtUnixMs;
  await page.waitForResponse(async (response) => {
    if (!response.url().endsWith("/__folio/signatures/inspect") || response.status() !== 200) return false;
    const report = await response.json();
    return report.documentId === good.id && report.checkedAtUnixMs >= current.recheckAtUnixMs;
  }, { timeout: 75_000 });
  await waitStatus("Timestamp verified with custom evidence");
  evidence.checks.push({ actualClockRecheckDeadline: current.recheckAtUnixMs, checkedAtBeforeExpiry: beforeExpiry,
    actualClockRefresh: true });

  async function theme(label, resolvedTheme) {
    await page.getByRole("button", { name: "Appearance", exact: true }).click();
    await page.getByRole("radio", { name: new RegExp(`^${label}$`, "i") }).check();
    await page.keyboard.press("Escape");
    await page.waitForFunction((theme) => document.documentElement.dataset.theme === theme, resolvedTheme);
  }
  for (const [label, id, width, height] of [
    ["Light", "light", 1440, 1000], ["Dark", "dark", 900, 760],
    ["Sepia", "sepia", 390, 844], ["High Contrast", "contrast", 1440, 1000],
  ]) {
    await page.setViewportSize({ width, height });
    if (!(await active().locator(".signatures-panel").isVisible())) await showSignatures();
    await theme(label, id);
    await waitStatus("Timestamp verified with custom evidence");
    await statuses().filter({ hasText: "Timestamp verified with custom evidence" }).scrollIntoViewIfNeeded();
    const layout = await active().locator(".signatures-panel").evaluate((element) => ({
      panelLeft: element.getBoundingClientRect().left, panelRight: element.getBoundingClientRect().right,
      width: innerWidth, rootOverflow: document.documentElement.scrollWidth > innerWidth,
      horizontalOverflow: element.scrollWidth > element.clientWidth,
    }));
    assert.ok(!layout.rootOverflow && !layout.horizontalOverflow);
    assert.ok(layout.panelLeft >= 0 && layout.panelRight <= width);
    const name = `signature-trust-${id}-${width}.png`;
    await page.screenshot({ path: join(artifacts, name) });
    evidence.screenshots.push(name); evidence.layouts.push({ theme: id, ...layout });
    assert.deepEqual(await sourceBytes(good), goodBytes);
  }
  await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" });
  await theme("System", "dark");
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await page.waitForFunction(() => document.documentElement.dataset.theme === "light");
  for (const info of opened.filter((info) => info.id !== draft.id)) {
    const bytes = await sourceBytes(info);
    evidence.sourceHashes[info.name] = hash(bytes);
    const expected = info.name === goodName ? goodBytes : await readFile(fixture(info.name));
    assert.deepEqual(bytes, expected);
  }
  assert.deepEqual(evidence.externalRequests, []);
  assert.deepEqual(errors, []);
  assert.ok(!backend.output.includes("PRIVATE KEY"));
  evidence.noNativeUserAppLaunch = true;
  evidence.noPrivateKeysRead = true;
  await writeFile(join(artifacts, "signature-trust-ui-evidence.json"), JSON.stringify(evidence, null, 2));
  console.log(JSON.stringify(evidence, null, 2));
} catch (error) {
  if (page && !page.isClosed()) console.error("Owned trust UI failure:\n", (await page.locator("body").innerText()).slice(-18000));
  throw error;
} finally {
  releaseHeld?.();
  await browser?.close();
  for (const child of children) {
    if (child.exitCode === null) child.kill("SIGTERM");
  }
  await Promise.all(children.map((child) => child.exitCode === null ? new Promise((done) => child.once("exit", done)) : Promise.resolve()));
  await rm(temporary, { recursive: true, force: true });
}
