import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { degrees, PDFDict, PDFDocument, PDFName, PDFNumber, PDFString, StandardFonts } from "pdf-lib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const temporary = await mkdtemp(join(tmpdir(), "folio-signatures-"));
const artifacts = process.env.FOLIO_SIGNATURE_ARTIFACTS ? resolve(process.env.FOLIO_SIGNATURE_ARTIFACTS) : temporary;
await mkdir(artifacts, { recursive: true });
const identityFolder = join(temporary, "identities");
await mkdir(identityFolder);
const password = "Synthetic runtime identity only";
const children = [];
const pageErrors = [];
const evidence = { externalRequests: [], signatureCounts: [], exactPrefixes: [], independentCmsChecks: [], screenshots: [] };
const env = { ...process.env, CARGO_INCREMENTAL: "0", CARGO_PROFILE_DEV_DEBUG: "0",
  CARGO_PROFILE_TEST_DEBUG: "0", CARGO_BUILD_JOBS: "2" };
let browser;
let page;

function start(command, args, extra = {}) {
  const child = spawn(command, args, { cwd: root, env: { ...env, ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  child.output = "";
  for (const stream of [child.stdout, child.stderr]) stream.on("data", (data) => { child.output = (child.output + data).slice(-32000); });
  child.on("error", (error) => { child.startError = error; });
  children.push(child);
  return child;
}

async function command(executable, args) {
  const child = start(executable, args);
  await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code) => code === 0 ? resolve() : reject(new Error(`${executable} failed (${code}): ${child.output}`)));
  });
  return child.output.trim();
}

async function independentlyVerify(bytes, label) {
  const document = await PDFDocument.load(bytes, { updateMetadata: false });
  let count = 0;
  for (const [, value] of document.context.enumerateIndirectObjects()) {
    if (!(value instanceof PDFDict) || value.get(PDFName.of("Type"))?.toString() !== "/Sig") continue;
    const [start, first, second, length] = value.lookup(PDFName.of("ByteRange")).asArray().map((value) => value.asNumber());
    assert.equal(start, 0);
    const data = Buffer.concat([bytes.slice(0, first), bytes.slice(second, second + length)]);
    const dataPath = join(temporary, `${label}-${count}.data`);
    const cmsPath = join(temporary, `${label}-${count}.cms`);
    const verifiedPath = join(temporary, `${label}-${count}.verified`);
    await writeFile(dataPath, data);
    await writeFile(cmsPath, value.lookup(PDFName.of("Contents")).asBytes());
    await command("openssl", ["cms", "-verify", "-binary", "-inform", "DER", "-noverify", "-in", cmsPath,
      "-content", dataPath, "-out", verifiedPath]);
    assert.deepEqual(await readFile(verifiedPath), data);
    count++;
  }
  assert.ok(count > 0);
  evidence.independentCmsChecks.push({ file: label, signatures: count });
}

async function freePort(preferred) {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(preferred, "127.0.0.1", resolve);
  }).catch(async (error) => {
    if (error.code !== "EADDRINUSE") throw error;
    await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  });
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}

async function ready(url, child) {
  const until = Date.now() + 120_000;
  while (Date.now() < until) {
    if (child.startError || child.exitCode !== null) throw new Error(`Isolated test server did not start: ${child.startError ?? child.output}`);
    try { if ((await fetch(url, { signal: AbortSignal.timeout(2000) })).ok) return; } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Isolated test server was not ready: ${child.output}`);
}

async function identity() {
  const path = (name) => join(identityFolder, name);
  for (const name of ["root", "intermediate", "leaf"]) {
    await command("openssl", ["genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", path(`${name}.key`)]);
  }
  await command("openssl", ["req", "-new", "-x509", "-key", path("root.key"), "-out", path("root.pem"),
    "-subj", "/CN=Folio runtime custom CA", "-days", "2", "-sha256",
    "-addext", "basicConstraints=critical,CA:TRUE,pathlen:1",
    "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-addext", "subjectKeyIdentifier=hash"]);
  for (const [name, issuer, serial, extensions] of [
    ["intermediate", "root", "2", "basicConstraints=critical,CA:TRUE,pathlen:0\nkeyUsage=critical,keyCertSign,cRLSign"],
    ["leaf", "intermediate", "3", "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,nonRepudiation\nextendedKeyUsage=1.3.6.1.5.5.7.3.36"],
  ]) {
    await command("openssl", ["req", "-new", "-key", path(`${name}.key`), "-out", path(`${name}.csr`), "-subj", `/CN=Folio runtime ${name}`]);
    await writeFile(path(`${name}.ext`), `${extensions}\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid,issuer\n`);
    await command("openssl", ["x509", "-req", "-in", path(`${name}.csr`), "-CA", path(`${issuer}.pem`), "-CAkey", path(`${issuer}.key`),
      "-set_serial", serial, "-days", "2", "-sha256", "-extfile", path(`${name}.ext`), "-out", path(`${name}.pem`)]);
  }
  await writeFile(path("chain.pem"), Buffer.concat([await readFile(path("intermediate.pem")), await readFile(path("root.pem"))]));
  await command("openssl", ["pkcs12", "-export", "-inkey", path("leaf.key"), "-in", path("leaf.pem"), "-certfile", path("chain.pem"),
    "-passout", `pass:${password}`, "-out", path("identity.p12")]);
  return { file: path("identity.p12"), root: path("root.pem"), privateKey: path("leaf.key") };
}

async function fixture() {
  const document = await PDFDocument.create();
  const font = await document.embedFont(StandardFonts.Helvetica);
  const first = document.addPage([500, 720]);
  first.drawText("Local approval workflow", { x: 45, y: 650, size: 20, font });
  const text = document.getForm().createTextField("Review note");
  text.setText("Original form value");
  text.addToPage(first, { x: 45, y: 520, width: 300, height: 32, font });
  const second = document.addPage([500, 700]);
  second.setMediaBox(-50, 20, 500, 700);
  second.setCropBox(-30, 70, 420, 540);
  second.setRotation(degrees(90));
  second.node.set(PDFName.of("UserUnit"), PDFNumber.of(1.5));
  second.drawText("Approval page with crop, rotation and physical scale", { x: 40, y: 500, size: 12, font });
  const form = document.catalog.lookup(PDFName.of("AcroForm"));
  const fields = form.lookup(PDFName.of("Fields"));
  const slots = [];
  for (let index = 0; index < 2; index++) {
    const field = document.context.obj({ FT: PDFName.of("Sig"), T: PDFString.of(`Approval ${index + 1}`), Ff: 0 });
    const fieldRef = document.context.register(field);
    const appearance = document.context.register(document.context.flateStream(
      "BT /F0 10 Tf 4 25 Td (Awaiting local approval) Tj ET", {
        Type: "XObject", Subtype: "Form", BBox: [0, 0, 260, 70], Resources: { Font: { F0: font.ref } },
      }));
    const widget = document.context.obj({
      Type: PDFName.of("Annot"), Subtype: PDFName.of("Widget"), Parent: fieldRef, P: second.ref,
      Rect: [40, 130 + index * 130, 300, 200 + index * 130], F: 4, AP: { N: appearance },
    });
    const widgetRef = document.context.register(widget);
    field.set(PDFName.of("Kids"), document.context.obj([widgetRef]));
    fields.push(fieldRef);
    second.node.addAnnot(widgetRef);
    slots.push({ fieldId: `${fieldRef.objectNumber}:${fieldRef.generationNumber}`, widgetId: `${widgetRef.objectNumber}:${widgetRef.generationNumber}` });
  }
  await document.attach(Buffer.from("Original workflow attachment"), "workflow.txt");
  const bytes = await document.save({ useObjectStreams: false });
  const path = join(artifacts, "Workflow-with-slots.pdf");
  await writeFile(path, bytes);
  return { path, bytes: new Uint8Array(bytes), slots };
}

try {
  evidence.opensslVersion = await command("openssl", ["version"]);
  const selectedIdentity = await identity();
  const source = await fixture();
  if (process.env.FOLIO_SIGNATURE_SKIP_BUILD !== "1") {
    await command("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--quiet"]);
  }
  const backendPort = await freePort(19158), frontendPort = await freePort(1430);
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
      headers: { "X-Folio-Preview": "1", ...(value ? { "Content-Type": "application/json" } : {}), ...extra },
      ...(value ? { method: "POST", body: JSON.stringify(value) } : {}),
    });
    assert.equal(response.status, expected, `${route}: ${await response.clone().text()}`);
    if (expected !== 200) {
      const error = await response.clone().text();
      assert.ok(!error.includes(password) && !error.includes("PRIVATE KEY") && !error.includes(identityFolder));
    }
    return response;
  };
  const revision = (info) => ({ documentId: info.id, revision: info.revision });
  const currentBytes = async (info, render = false) => new Uint8Array(await (await api(render ? "/render-bytes" : "/bytes", null, {
    "X-Document-Id": String(info.id), "X-Revision": String(info.revision),
  })).arrayBuffer());
  const snapshot = async () => (await api("/bootstrap")).json();
  const inspect = async (info) => (await api("/signatures/inspect", revision(info))).json();
  browser = await chromium.launch({ headless: true,
    ...(process.env.FOLIO_SIGNATURE_BROWSER ? { executablePath: process.env.FOLIO_SIGNATURE_BROWSER } : {}) });
  evidence.browserVersion = browser.version();
  console.log(`Signature browser: ${evidence.browserVersion}`);
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce" });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url());
    return route.abort();
  });
  page = await context.newPage();
  page.setDefaultTimeout(20_000);
  page.on("pageerror", (error) => { pageErrors.push(error.message); console.error(error.message); });
  await page.goto(origin);
  const active = () => page.locator(".document-workspace:not([hidden])");
  const openFile = async (path, name) => {
    await page.waitForFunction(() => {
      const button = document.querySelector('button[aria-label="Open PDFs in new tabs"]');
      return button && !button.disabled;
    }, null, { timeout: 20_000 }).catch(async (error) => {
      console.error("Opening guard:", await page.locator("body").innerText(), frontend.output, backend.output);
      throw error;
    });
    const [selected] = await Promise.all([
      page.waitForEvent("filechooser", { timeout: 20_000 }),
      page.getByRole("button", { name: "Open PDFs in new tabs", exact: true }).click({ timeout: 20_000 }),
    ]);
    const [response] = await Promise.all([
      page.waitForResponse((response) => response.url().endsWith("/__folio/open") && response.request().method() === "POST"),
      selected.setFiles(path),
    ]);
    assert.equal(response.status(), 200);
    const opened = (await snapshot()).documents.find((document) => document.name === name);
    assert.ok(opened, `The uploaded PDF did not open: ${name}`);
    await page.waitForFunction((name) => [...document.querySelectorAll(".document-tab.selected")].some((tab) => tab.textContent.includes(name)), name);
  };
  await openFile(source.path, "Workflow-with-slots.pdf");
  let sourceInfo = (await snapshot()).documents.find((info) => info.name === "Workflow-with-slots.pdf");
  assert.equal(sourceInfo.readOnlyReason, null);
  const blankReport = await inspect(sourceInfo);
  assert.equal(blankReport.signatures.length, 0);
  assert.deepEqual(blankReport.emptyFields.map((field) => field.id), source.slots.map((slot) => slot.fieldId));
  await active().getByRole("button", { name: "Show form fields", exact: true }).click();
  await active().getByRole("textbox", { name: "Review note", exact: true }).fill("Unapplied review draft");
  await active().getByRole("button", { name: "Show digital signatures", exact: true }).click();
  await active().getByText("No signatures found", { exact: true }).waitFor();
  assert.equal(await active().getByRole("button", { name: "Export signed copy", exact: true }).isDisabled(), true);
  const pfx = Array.from(await readFile(selectedIdentity.file));
  await api("/signatures/export", { ...revision(sourceInfo), identity: pfx, password }, {}, 400);
  assert.deepEqual(await currentBytes(sourceInfo, true), source.bytes);
  await active().getByRole("button", { name: "Show form fields", exact: true }).click();
  assert.equal(await active().getByRole("textbox", { name: "Review note", exact: true }).inputValue(), "Unapplied review draft");
  await active().getByRole("button", { name: "Apply form changes", exact: true }).click();
  await active().getByText("No unapplied form changes.", { exact: true }).waitFor();
  await active().getByRole("button", { name: "Show digital signatures", exact: true }).click();
  sourceInfo = (await snapshot()).documents.find((info) => info.id === sourceInfo.id);
  const unsaved = await currentBytes(sourceInfo);
  assert.equal(sourceInfo.dirty, true);
  const sourceState = JSON.stringify(sourceInfo);
  await active().getByRole("button", { name: "Export signed copy", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Signature location", { exact: true }).selectOption(source.slots[0].fieldId);
  await dialog.getByLabel("Choose PKCS#12 signing identity", { exact: true }).setInputFiles(selectedIdentity.file);
  await dialog.getByLabel("Identity password", { exact: true }).fill("Deliberately wrong runtime password");
  await dialog.getByRole("checkbox").check();
  await dialog.getByRole("button", { name: "Sign and export copy", exact: true }).click();
  await dialog.getByRole("alert").filter({ hasText: "could not be unlocked" }).waitFor();
  assert.equal(await dialog.getByLabel("Identity password", { exact: true }).inputValue(), "");
  assert.deepEqual(await currentBytes(sourceInfo), unsaved);
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  assert.deepEqual(await currentBytes(sourceInfo), unsaved);
  await active().getByRole("button", { name: "Export signed copy", exact: true }).click();
  await dialog.getByLabel("Signature location", { exact: true }).selectOption(source.slots[0].fieldId);
  await dialog.getByLabel("Choose PKCS#12 signing identity", { exact: true }).setInputFiles(selectedIdentity.file);
  await dialog.getByLabel("Identity password", { exact: true }).fill(password);
  await dialog.getByRole("checkbox").check();
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    dialog.getByRole("button", { name: "Sign and export copy", exact: true }).click(),
  ]);
  const firstPath = join(artifacts, "First-signed.pdf");
  await download.saveAs(firstPath);
  const firstBytes = new Uint8Array(await readFile(firstPath));
  await independentlyVerify(firstBytes, "First-signed");
  assert.deepEqual(firstBytes.slice(0, unsaved.length), unsaved);
  evidence.exactPrefixes.push({ original: unsaved.length, output: firstBytes.length });
  assert.equal(JSON.stringify((await snapshot()).documents.find((info) => info.id === sourceInfo.id)), sourceState);
  assert.deepEqual(await currentBytes(sourceInfo), unsaved);
  const appearance = await page.evaluate(async ({ original, signed }) => {
    const { loadPdf } = await import("/src/lib/pdf.ts");
    const { OPS } = await import("/node_modules/pdfjs-dist/legacy/build/pdf.mjs");
    const renders = [];
    const records = [];
    for (const bytes of [original, signed]) {
      const task = loadPdf(new Uint8Array(bytes));
      try {
        const pdf = await task.promise;
        const fields = await pdf.getFieldObjects();
        if (!(fields instanceof Map)) throw new Error("PDF.js v6 fields must use the Map contract.");
        const selected = fields.get("Approval 1")?.[0];
        const page = await pdf.getPage(2);
        const annotations = await page.getAnnotations();
        const signature = annotations.find((annotation) => annotation.fieldName === "Approval 1");
        const viewport = page.getViewport({ scale: 1 });
        const canvas = document.createElement("canvas");
        canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
        await page.render({ canvas, viewport, annotationMode: 1 }).promise;
        const list = await page.getOperatorList({ annotationMode: 1 });
        let text = "";
        for (let index = 0; index < list.fnArray.length; index++) {
          if (list.fnArray[index] === OPS.showText) {
            text += list.argsArray[index][0].map((glyph) => typeof glyph === "object" ? glyph.unicode ?? "" : "").join("");
          }
        }
        const rect = [...viewport.convertToViewportPoint(40, 130), ...viewport.convertToViewportPoint(300, 200)];
        const x = Math.floor(Math.min(rect[0], rect[2])), y = Math.floor(Math.min(rect[1], rect[3]));
        const width = Math.ceil(Math.abs(rect[2] - rect[0])), height = Math.ceil(Math.abs(rect[3] - rect[1]));
        const pixels = canvas.getContext("2d").getImageData(x, y, width, height).data;
        renders.push(Array.from(pixels));
        records.push({ fields: fields.size, selected: Boolean(selected), hasAppearance: signature?.hasAppearance,
          text, width: viewport.width, height: viewport.height, image: canvas.toDataURL("image/png") });
      } finally { await task.destroy(); }
    }
    let changed = 0;
    for (let index = 0; index < renders[0].length; index++) if (renders[0][index] !== renders[1][index]) changed++;
    return { records, changed };
  }, { original: Array.from(unsaved), signed: Array.from(firstBytes) });
  assert.ok(appearance.records.every((record) => record.selected));
  assert.match(appearance.records[1].text, /Digitally signed/);
  assert.ok(appearance.changed > 100, "The persistent selected-widget appearance must change, not remain unsigned.");
  await writeFile(join(artifacts, "signed-rotated-widget.png"), Buffer.from(appearance.records[1].image.split(",")[1], "base64"));
  evidence.appearance = { changedChannels: appearance.changed, pageWidth: appearance.records[1].width,
    pageHeight: appearance.records[1].height, signedText: appearance.records[1].text };
  await openFile(firstPath, "First-signed.pdf");
  const firstInfo = (await snapshot()).documents.find((info) => info.name === "First-signed.pdf");
  assert.ok(firstInfo.readOnlyReason);
  await active().getByRole("button", { name: "Show digital signatures", exact: true }).click();
  await active().getByText("Signed bytes intact", { exact: true }).waitFor();
  await active().getByText("Custom CA store (0)", { exact: true }).click();
  const [cancelCa] = await Promise.all([
    page.waitForEvent("filechooser"),
    active().getByRole("button", { name: "Add public root CA", exact: true }).click(),
  ]);
  await cancelCa.setFiles([]);
  assert.equal((await inspect(firstInfo)).trustStore.revision, 0);
  const [chooseCa] = await Promise.all([
    page.waitForEvent("filechooser"),
    active().getByRole("button", { name: "Add public root CA", exact: true }).click(),
  ]);
  await chooseCa.setFiles(selectedIdentity.root);
  await active().getByText("Chain anchored in your custom CA store", { exact: true }).waitFor();
  const trusted = await inspect(firstInfo);
  assert.equal(trusted.signatures[0].chain.status, "anchored_custom_store");
  evidence.customChain = trusted.signatures[0].chain.status;
  const mirrorPath = join(artifacts, "Mirror-signed.pdf");
  await writeFile(mirrorPath, firstBytes);
  await openFile(mirrorPath, "Mirror-signed.pdf");
  await active().getByRole("button", { name: "Show digital signatures", exact: true }).click();
  await active().getByText("Chain anchored in your custom CA store", { exact: true }).waitFor();
  await active().getByText("Custom CA store (1)", { exact: true }).click();
  await active().getByRole("button", { name: /^Remove custom CA / }).click();
  await active().getByText("Certificate-chain trust not checked", { exact: true }).waitFor();
  assert.equal(await page.locator(".signature-custom-anchor").count(), 0, "Inactive tabs must not retain removed-anchor conclusions.");
  await page.getByRole("tab").filter({ hasText: "First-signed.pdf" }).click();
  await active().getByText("Certificate-chain trust not checked", { exact: true }).waitFor();
  await active().getByRole("button", { name: "Export co-signed copy", exact: true }).click();
  await dialog.getByLabel("Choose PKCS#12 signing identity", { exact: true }).setInputFiles(selectedIdentity.file);
  await dialog.getByLabel("Identity password", { exact: true }).fill(password);
  await dialog.getByRole("checkbox").check();
  const [secondDownload] = await Promise.all([
    page.waitForEvent("download"),
    dialog.getByRole("button", { name: "Sign and export copy", exact: true }).click(),
  ]);
  const secondPath = join(artifacts, "Second-signed.pdf");
  await secondDownload.saveAs(secondPath);
  const secondBytes = new Uint8Array(await readFile(secondPath));
  await independentlyVerify(secondBytes, "Second-signed");
  assert.deepEqual(secondBytes.slice(0, firstBytes.length), firstBytes);
  evidence.exactPrefixes.push({ original: firstBytes.length, output: secondBytes.length });
  assert.deepEqual(await currentBytes(firstInfo), firstBytes);
  await openFile(secondPath, "Second-signed.pdf");
  const secondInfo = (await snapshot()).documents.find((info) => info.name === "Second-signed.pdf");
  const secondReport = await inspect(secondInfo);
  assert.equal(secondReport.signatures.length, 2);
  assert.ok(secondReport.signatures.every((signature) => signature.integrity === "intact"));
  assert.equal(secondReport.signatures.filter((signature) => signature.coverage === "whole_document").length, 1);
  evidence.signatureCounts.push(1, 2);
  const bytesBeforeFailures = await currentBytes(secondInfo);
  for (const request of [
    { ...revision(secondInfo), identityPath: selectedIdentity.file, password },
    { ...revision(secondInfo), identity: pfx, password, fieldId: 42 },
    { documentId: secondInfo.id, revision: secondInfo.revision + 1, identity: pfx, password },
    { ...revision(secondInfo), identity: new Array(4 * 1024 * 1024 + 1).fill(0), password },
  ]) await api("/signatures/export", request, {}, 400);
  for (const request of [
    { ...revision(secondInfo), trustRevision: 2, path: selectedIdentity.root },
    { ...revision(secondInfo), trustRevision: 2, certificate: Array.from(await readFile(selectedIdentity.privateKey)) },
    { ...revision(secondInfo), trustRevision: 2, certificate: new Array(64 * 1024 + 1).fill(0) },
  ]) await api("/signatures/anchors/add", request, {}, 400);
  assert.deepEqual(await currentBytes(secondInfo), bytesBeforeFailures);
  const tampered = firstBytes.slice(); tampered[7] = tampered[7] === 55 ? 54 : 55;
  const invalid = await (await fetch(`http://127.0.0.1:${backendPort}/open`, {
    method: "POST", headers: { "X-Folio-Preview": "1", "X-File-Name": "Tampered-runtime.pdf" }, body: tampered,
  })).json();
  assert.equal((await inspect(invalid.document)).signatures[0].integrity, "invalid");
  await api("/signatures/export", { ...revision(invalid.document), identity: pfx, password }, {}, 400);
  await active().getByRole("button", { name: "Show digital signatures", exact: true }).click();
  await active().getByText("Later bytes are not signed", { exact: true }).waitFor();
  await page.screenshot({ path: join(artifacts, "signatures-light-desktop.png") });
  evidence.screenshots.push("signatures-light-desktop.png");
  await active().getByRole("button", { name: "Export co-signed copy", exact: true }).click();
  await page.evaluate(() => { document.documentElement.dataset.theme = "dark"; });
  await page.setViewportSize({ width: 900, height: 760 });
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  assert.ok(await dialog.evaluate((element) => element.getBoundingClientRect().bottom <= innerHeight && element.getBoundingClientRect().top >= 0));
  await page.screenshot({ path: join(artifacts, "signatures-dark-compact.png") });
  evidence.screenshots.push("signatures-dark-compact.png");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.evaluate(() => { document.documentElement.dataset.theme = "contrast"; });
  await page.screenshot({ path: join(artifacts, "signatures-high-contrast-desktop.png") });
  evidence.screenshots.push("signatures-high-contrast-desktop.png");
  assert.deepEqual(evidence.externalRequests, []);
  assert.deepEqual(pageErrors, []);
  assert.ok(!backend.output.includes(password) && !backend.output.includes("PRIVATE KEY"));
  evidence.sourceStateUnchanged = true;
  evidence.anchorRemovalInvalidatesInactiveTabs = true;
  evidence.passwordClearedOnFailure = true;
  evidence.failuresPreserveBytes = true;
  evidence.noPrivateDataInErrorsOrLogs = true;
  evidence.sha256 = createHash("sha256").update(secondBytes).digest("hex");
  await writeFile(join(artifacts, "signature-evidence.json"), JSON.stringify(evidence, null, 2));
  console.log(JSON.stringify(evidence, null, 2));
} catch (error) {
  if (page && !page.isClosed()) console.error("Signature UI failure state:\n", (await page.locator("body").innerText()).slice(-18000));
  throw error;
} finally {
  await browser?.close();
  for (const child of children) {
    if (child.exitCode !== null || child.startError) continue;
    child.kill("SIGTERM");
    await new Promise((resolve) => { const timeout = setTimeout(() => { child.kill("SIGKILL"); resolve(); }, 5000); child.once("exit", () => { clearTimeout(timeout); resolve(); }); });
  }
  await rm(temporary, { recursive: true, force: true });
}
