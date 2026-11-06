import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, readdir, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { PDFDocument } from "pdf-lib";
import { chromium } from "playwright";
import { createServer as createViteServer } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));
const output = process.env.FOLIO_RECOVERY_EVIDENCE
  ? resolve(process.env.FOLIO_RECOVERY_EVIDENCE) : await mkdtemp(join(tmpdir(), "folio-recovery-"));
const storage = join(output, "storage");
await mkdir(storage, { recursive: true });
const target = resolve(process.env.CARGO_TARGET_DIR ?? join(root, "src-tauri/target"));
const children = [];
const evidence = { checks: [], errors: [], externalRequests: [], screenshots: [] };
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
async function stopEngine() {
  const stopped = new Promise((resolve) => engine.once("exit", resolve));
  engine.kill("SIGKILL");
  await stopped;
}
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
try {
  if (process.env.FOLIO_RECOVERY_SKIP_BUILD !== "1") {
    await complete(start("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml",
      "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--locked"], { CARGO_TARGET_DIR: target }));
  }
  const frontendPort = await freePort(), backendPort = await freePort();
  const origin = `http://127.0.0.1:${frontendPort}`;
  async function startEngine() {
    engine = start(join(target, `debug/folio-preview${process.platform === "win32" ? ".exe" : ""}`), [],
      { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort), FOLIO_PREVIEW_RECOVERY_DIR: storage });
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`Recovery-test engine did not start: ${engine.output}`)), 15000);
      engine.once("error", reject);
      engine.stdout.on("data", () => { if (engine.output.includes("ready at")) { clearTimeout(timer); resolve(); } });
    });
  }
  await startEngine();
  async function api(path, value) {
    const response = await fetch(`http://127.0.0.1:${backendPort}${path}`, {
      headers: { "X-Folio-Preview": "1", ...(value === undefined ? {} : { "Content-Type": "application/json" }) },
      ...(value === undefined ? {} : { method: "POST", body: JSON.stringify(value) }),
    });
    assert.equal(response.ok, true, `${path}: ${await response.clone().text()}`);
    return response;
  }
  const status = async () => (await api("/recovery")).json();
  const revision = (info) => ({ documentId: info.id, revision: info.revision });
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
    ...(process.env.FOLIO_RECOVERY_BROWSER ? { executablePath: process.env.FOLIO_RECOVERY_BROWSER } : {}) });
  const context = await browser.newContext({ reducedMotion: "reduce", viewport: { width: 1440, height: 1000 } });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url()); return route.abort();
  });
  page = await context.newPage();
  page.setDefaultTimeout(15000);
  page.on("dialog", (dialog) => dialog.accept());
  page.on("pageerror", (error) => evidence.errors.push(error.message));
  async function ready() {
    await page.waitForFunction(() => document.querySelector(".document-workspace:not([hidden]) canvas")
      && !document.querySelector(".document-busy, .paper-loading"));
    await page.waitForLoadState("networkidle");
  }
  await page.goto(origin);
  await ready();
  await page.getByRole("button", { name: "Local recovery", exact: true }).click();
  await page.getByRole("button", { name: "Enable local recovery", exact: true }).click();
  await page.getByText("Recovery is on", { exact: true }).waitFor();
  await page.getByRole("button", { name: "Done", exact: true }).click();
  assert.equal((await status()).enabled, true);
  evidence.checks.push("Explicit opt-in is off initially and enabled through real UI.");

  const pdf = await PDFDocument.create();
  pdf.addPage([420, 600]).drawText("Synthetic crash recovery source");
  const source = Buffer.from(await pdf.save({ useObjectStreams: false }));
  const sourcePath = join(output, "original-source.pdf");
  await writeFile(sourcePath, source);
  await page.getByLabel("Choose PDF files", { exact: true }).setInputFiles(sourcePath);
  await page.waitForFunction(() => document.querySelector(".document-tab.selected")?.textContent.includes("original-source.pdf"));
  await ready();
  const active = page.locator(".document-workspace:not([hidden])");
  await active.getByRole("button", { name: "Organize", exact: true }).click();
  await Promise.all([
    page.waitForResponse((response) => response.url().endsWith("/__folio/edit") && response.ok()),
    active.getByRole("button", { name: "Rotate page right", exact: true }).click(),
  ]);
  await page.waitForFunction(() => !document.querySelector(".document-workspace:not([hidden]) .document-busy"));
  const snapshot = await (await api("/bootstrap")).json();
  const first = snapshot.documents.find((doc) => doc.id === snapshot.activeId);
  assert.equal(first.pages[0].rotation, 90);
  const applied = await bytes(first);
  assert.equal((await status()).checkpointed, 1);
  await stopEngine();
  await startEngine();
  await page.reload();
  const dialog = page.getByRole("dialog", { name: "Local crash recovery", exact: true });
  await dialog.waitFor();
  await dialog.getByText("original-source.pdf", { exact: true }).waitFor();
  await page.screenshot({ path: join(output, "recovery-startup.png") });
  evidence.screenshots.push("recovery-startup.png");
  await dialog.getByRole("button", { name: "Recover in new tab", exact: true }).click();
  await dialog.getByText("No previous recovery copies.", { exact: true }).waitFor();
  await dialog.getByRole("button", { name: "Done", exact: true }).click();
  const recoveredState = await (await api("/bootstrap")).json();
  const recovered = recoveredState.documents.find((doc) => doc.id === recoveredState.activeId);
  assert.equal(recovered.dirty, true);
  assert.equal(recovered.canUndo, false);
  assert.equal(hash(await bytes(recovered)), hash(applied));
  assert.equal(hash(await readFile(sourcePath)), hash(source));
  evidence.checks.push("Hard process termination + fresh Rust process + startup UI recovers exact applied PDF bytes; source hash unchanged; no invented undo.");

  // A real UI redaction draft participates in the aggregate backend guard.
  await active.getByRole("button", { name: "Redact areas (R)", exact: true }).click();
  await active.getByText("Add an area by coordinates", { exact: true }).click();
  const panel = active.locator(".redaction-fields");
  const inputs = panel.locator("input");
  for (const [index, value] of ["30", "30", "100", "100"].entries()) await inputs.nth(index).fill(value);
  await Promise.all([
    page.waitForResponse((response) => response.url().endsWith("/__folio/draft") && response.ok()),
    active.getByRole("button", { name: "Add redaction area", exact: true }).click(),
  ]);
  assert.equal((await status()).checkpointed, 0);
  await stopEngine();
  await startEngine();
  assert.equal((await status()).entries.length, 0);
  await page.reload();
  await ready();
  await page.getByRole("button", { name: "Local recovery", exact: true }).click();
  await page.getByText("No previous recovery copies.", { exact: true }).waitFor();
  await page.getByRole("button", { name: "Done", exact: true }).click();
  evidence.checks.push("Real pending redaction removes checkpoint; restart never presents unapplied marks as recovered.");

  const formPdf = await PDFDocument.create();
  const formPage = formPdf.addPage([420, 600]);
  formPdf.getForm().createTextField("Recovery field").addToPage(formPage, { x: 30, y: 450, width: 250, height: 30 });
  await page.getByLabel("Choose PDF files", { exact: true }).setInputFiles({
    name: "form-source.pdf", mimeType: "application/pdf", buffer: Buffer.from(await formPdf.save()),
  });
  await page.waitForFunction(() => document.querySelector(".document-tab.selected")?.textContent.includes("form-source.pdf"));
  await ready();
  await active.getByRole("button", { name: "Organize", exact: true }).click();
  await Promise.all([
    page.waitForResponse((response) => response.url().endsWith("/__folio/edit") && response.ok()),
    active.getByRole("button", { name: "Rotate page right", exact: true }).click(),
  ]);
  assert.equal((await status()).checkpointed, 1);
  await active.getByRole("button", { name: "Read", exact: true }).click();
  const fill = active.getByRole("button", { name: "Fill form fields", exact: true });
  if (!await fill.isVisible()) await active.getByRole("button", { name: "All tools", exact: true }).click();
  await fill.click();
  await Promise.all([
    page.waitForResponse((response) => response.url().endsWith("/__folio/draft") && response.ok()),
    active.getByRole("textbox", { name: "Recovery field", exact: true }).fill("Unapplied synthetic value"),
  ]);
  assert.equal((await status()).checkpointed, 0);
  await stopEngine();
  await startEngine();
  assert.equal((await status()).entries.length, 0);
  evidence.checks.push("A real unapplied AcroForm value removes an existing applied-edit checkpoint; neither it nor a stale copy reappears after restart.");

  const clear = await (await api("/create", { width: 200, height: 200 })).json();
  const protectedResponse = await api("/protection/export", { ...revision(clear), authorizationPassword: null,
    protection: { mode: "protect", openingPassword: "synthetic-open", ownerPassword: "synthetic-owner",
      permissions: { print: "high", copy: true, edit: true, annotate: true, fillForms: true, assemble: true } } });
  const protectedBytes = Buffer.from(await protectedResponse.arrayBuffer());
  await api("/close", { ...revision(clear), discardChanges: true });
  const challengeResponse = await fetch(`http://127.0.0.1:${backendPort}/open`, {
    method: "POST", headers: { "X-Folio-Preview": "1", "X-File-Name": "protected.pdf" }, body: protectedBytes,
  });
  const challenge = await challengeResponse.json();
  const unlocked = await (await api("/unlock", { requestId: challenge.requestId, password: "synthetic-owner" })).json();
  const protectedInfo = await (await api("/edit", { ...revision(unlocked.document), edit: { type: "rotate", page: 0, degrees: 90 } })).json();
  assert.equal(protectedInfo.passwordProtected, true);
  assert.equal((await status()).checkpointed, 0);
  await stopEngine();
  await startEngine();
  assert.equal((await status()).entries.length, 0);
  for (const name of await readdir(storage)) {
    if (name === "writer.lock") {
      assert.equal((await stat(join(storage, name))).size, 0);
      continue;
    }
    const data = await readFile(join(storage, name));
    assert.equal(data.includes(Buffer.from("synthetic-owner")), false);
    assert.equal(data.includes(Buffer.from("Synthetic crash recovery source")), false);
  }
  evidence.checks.push("Owner-unlocked, edited AES-256 PDF leaves no recovery PDF/password after actual process restart.");

  const damaged = await (await api("/create", { width: 200, height: 200 })).json();
  assert.equal(damaged.dirty, true);
  await stopEngine();
  const file = (await readdir(storage)).find((name) => name.endsWith(".folio-recovery"));
  await writeFile(join(storage, file), Buffer.from("incomplete synthetic checkpoint"));
  await startEngine();
  await page.reload();
  await dialog.waitFor();
  await dialog.getByText("Unreadable recovery copy", { exact: true }).waitFor();
  assert.equal(await dialog.getByRole("button", { name: "Recover in new tab" }).isDisabled(), true);
  await dialog.getByRole("button", { name: "Discard copy", exact: true }).click();
  await dialog.getByRole("button", { name: "Confirm deletion", exact: true }).click();
  await dialog.getByText("No previous recovery copies.", { exact: true }).waitFor();
  evidence.checks.push("Corrupt checkpoint is shown with a disabled recovery action; confirmed discard actually deletes it.");

  await dialog.getByRole("button", { name: "Done", exact: true }).waitFor({ state: "visible" });
  await dialog.evaluate((node) => { node.scrollTop = 0; });
  for (const theme of ["light", "dark", "sepia", "contrast"]) {
    await page.evaluate((theme) => { document.documentElement.dataset.theme = theme; }, theme);
    await page.screenshot({ path: join(output, `recovery-${theme}.png`) });
    evidence.screenshots.push(`recovery-${theme}.png`);
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(output, "recovery-narrow.png") });
  evidence.screenshots.push("recovery-narrow.png");
  assert.equal(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth), true);
  await page.keyboard.press("Escape");
  await dialog.waitFor({ state: "detached" });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole("button", { name: "Local recovery", exact: true }).click();
  await page.getByRole("button", { name: "Turn off & delete recovery copies", exact: true }).click();
  await page.getByRole("button", { name: "Confirm deletion", exact: true }).click();
  await page.getByText("Recovery is off", { exact: true }).waitFor();
  assert.equal((await status()).enabled, false);
  assert.equal((await readdir(storage)).some((name) => name.endsWith(".folio-recovery")), false);
  assert.equal(hash(await readFile(sourcePath)), hash(source));
  assert.deepEqual(evidence.errors, []);
  assert.deepEqual(evidence.externalRequests, []);
  evidence.checks.push("Theme/narrow dialog checks, Escape dismissal, opt-out deletion, no external requests or page errors, unchanged original source.");
  console.log(JSON.stringify({ output, ...evidence }, null, 2));
} catch (error) {
  evidence.failure = error.stack ?? String(error);
  const diagnostics = [writeFile(join(output, "engine.log"), engine?.output ?? "")];
  if (page && !page.isClosed()) diagnostics.push(
    page.screenshot({ path: join(output, "failure.png"), timeout: 5000 }),
    page.content().then((html) => writeFile(join(output, "failure.html"), html)),
  );
  for (const result of await Promise.allSettled(diagnostics)) {
    if (result.status === "rejected") console.error("Could not capture failure evidence:", result.reason);
  }
  throw error;
} finally {
  await browser?.close();
  await ui?.close();
  for (const child of children) if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  await writeFile(join(output, "evidence.json"), JSON.stringify(evidence, null, 2));
}
