import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { PDFDocument, StandardFonts } from "pdf-lib";
import { chromium } from "playwright";
import { createServer as createViteServer } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));
const target = resolve(process.env.CARGO_TARGET_DIR ?? join(root, "src-tauri/target"));
const output = process.env.FOLIO_MOTION_EVIDENCE_DIR
  ? resolve(process.env.FOLIO_MOTION_EVIDENCE_DIR) : await mkdtemp(join(tmpdir(), "folio-motion-"));
await mkdir(output, { recursive: true });
const evidence = { checks: [], screenshots: [], measurements: [], errors: [], externalRequests: [] };
const children = [];
let browser, ui, page;
function start(command, args, extra = {}) {
  const child = spawn(command, args, { cwd: root, env: { ...process.env, ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  child.output = "";
  for (const stream of [child.stdout, child.stderr]) {
    stream.on("data", (chunk) => { child.output = (child.output + chunk).slice(-20000); });
  }
  children.push(child);
  return child;
}
async function finished(child) {
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
function motionSnapshot() {
  return document.getAnimations().filter((animation) => animation.id.startsWith("folio-selection-")
    || ["context-reveal", "dialog-enter", "appearance-enter", "brand-unfold"].includes(animation.animationName))
    .map((animation) => ({
      name: animation.id || animation.animationName,
      duration: animation.effect.getTiming().duration,
      frames: animation.effect.getKeyframes().map((frame) => ({
        transform: frame.transform, clipPath: frame.clipPath, opacity: frame.opacity,
      })),
    }));
}
try {
  if (process.env.FOLIO_MOTION_SKIP_BUILD !== "1") {
    await finished(start("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml",
      "--no-default-features", "--features", "preview", "--bin", "folio-preview", "--locked"],
    { CARGO_TARGET_DIR: target }));
  }
  const frontendPort = await freePort(), backendPort = await freePort();
  const engine = start(join(target, `debug/folio-preview${process.platform === "win32" ? ".exe" : ""}`), [],
    { FOLIO_DEV_PORT: String(frontendPort), FOLIO_PREVIEW_PORT: String(backendPort) });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`Preview startup timed out: ${engine.output}`)), 15000);
    engine.once("error", (error) => { clearTimeout(timer); reject(error); });
    engine.once("exit", () => { clearTimeout(timer); reject(new Error(engine.output)); });
    engine.stdout.on("data", () => {
      if (engine.output.includes("Folio Rust preview engine ready")) { clearTimeout(timer); resolve(); }
    });
  });
  assert.equal((await fetch(`http://127.0.0.1:${backendPort}/health`)).ok, true);
  const initialResponse = await fetch(`http://127.0.0.1:${backendPort}/bootstrap`, { headers: { "X-Folio-Preview": "1" } });
  assert.equal(initialResponse.ok, true);
  const initial = await initialResponse.json();
  for (const document of initial.documents) {
    assert.equal(document.dirty, false);
    const closed = await fetch(`http://127.0.0.1:${backendPort}/close`, {
      method: "POST", headers: { "X-Folio-Preview": "1", "Content-Type": "application/json" },
      body: JSON.stringify({ documentId: document.id, revision: document.revision, discardChanges: false }),
    });
    assert.equal(closed.ok, true);
  }
  const origin = `http://127.0.0.1:${frontendPort}`;
  ui = await createViteServer({ root, logLevel: "error", server: {
    host: "127.0.0.1", port: frontendPort, strictPort: true,
    proxy: { "/__folio": { target: `http://127.0.0.1:${backendPort}`, rewrite: (path) => path.replace(/^\/__folio/, "") } },
  } });
  await ui.listen();
  browser = await chromium.launch({ headless: true,
    ...(process.env.FOLIO_MOTION_BROWSER ? { executablePath: process.env.FOLIO_MOTION_BROWSER } : {}) });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "no-preference" });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url());
    return route.abort();
  });
  await context.addInitScript((origin) => {
    if (location.origin !== origin) return;
    window.pauseCssMotion = true;
    window.pauseSelectionMotion = true;
    localStorage.setItem("folio.appearance", "light");
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (...args) {
      const animation = animate.apply(this, args);
      if (window.pauseSelectionMotion && this.classList.contains("selection-indicator")) {
        animation.pause(); animation.currentTime = 100;
      }
      return animation;
    };
    document.addEventListener("animationstart", (event) => {
      if (!window.pauseCssMotion || !["context-reveal", "dialog-enter", "appearance-enter", "brand-unfold"].includes(event.animationName)) return;
      for (const animation of event.target.getAnimations()) {
        if (animation.animationName === event.animationName) { animation.pause(); animation.currentTime = 100; }
      }
    });
  }, origin);
  page = await context.newPage();
  page.setDefaultTimeout(15000);
  page.on("pageerror", (error) => evidence.errors.push(error.message));
  await page.goto(origin);
  const workspace = () => page.locator(".document-workspace:not([hidden])");
  await page.getByRole("heading", { name: "A little less paperwork.", exact: true }).waitFor();
  const welcome = await page.evaluate(motionSnapshot);
  assert.ok(welcome.some((animation) => animation.name === "brand-unfold" && animation.duration === 560));
  evidence.measurements.push({ state: "welcome", animations: welcome });
  const release = async () => {
    await page.evaluate(() => {
      window.pauseCssMotion = false; window.pauseSelectionMotion = false;
      for (const animation of document.getAnimations()) if (animation.playState === "paused") animation.play();
    });
    await page.waitForFunction(() => document.getAnimations().every((animation) => animation.effect.getTiming().iterations === Infinity
      || animation.playState === "finished"));
  };
  await release();

  const pdf = await PDFDocument.create();
  const font = await pdf.embedFont(StandardFonts.Helvetica);
  for (let index = 0; index < 2; index++) {
    const sheet = pdf.addPage([612, 792]);
    sheet.drawText(`Folio motion review ${index + 1}`, { x: 54, y: 700, size: 23, font });
    sheet.drawText("Real PDF pixels stay still while workspace controls move.", { x: 54, y: 654, size: 13, font });
  }
  const source = Buffer.from(await pdf.save());
  await page.getByLabel("Choose PDF files", { exact: true }).setInputFiles([
    { name: "Motion first.pdf", mimeType: "application/pdf", buffer: source },
    { name: "Motion second.pdf", mimeType: "application/pdf", buffer: source },
  ]);
  const ready = async () => {
    await page.waitForFunction(() => {
      const region = document.querySelector(".document-workspace:not([hidden])");
      return region?.querySelector('canvas[aria-label="Rendered PDF page 1"]')
        && !region.querySelector(".document-busy, .paper-loading")
        && !document.querySelector(".tab-action-status");
    });
  };
  await ready();
  const canvasDigest = () => workspace().locator('canvas[aria-label="Rendered PDF page 1"]').evaluate((canvas) => canvas.toDataURL());
  const beforePixels = await canvasDigest();
  const beforeSize = await workspace().locator('canvas[aria-label="Rendered PDF page 1"]')
    .evaluate((canvas) => ({ width: canvas.width, height: canvas.height }));
  const pause = () => page.evaluate(() => { window.pauseCssMotion = true; window.pauseSelectionMotion = true; });
  await pause();
  const nav = () => workspace().getByRole("navigation", { name: "Document workspace", exact: true });
  await nav().getByRole("button", { name: "Comment", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".document-workspace:not([hidden]) .selection-modes[data-moving]"));
  const modeMotion = await page.evaluate(motionSnapshot);
  assert.ok(modeMotion.some((animation) => animation.name === "folio-selection-modes" && animation.duration === 380));
  const panelMotion = modeMotion.find((animation) => animation.name === "context-reveal");
  assert.ok(panelMotion);
  const panelDistance = await page.evaluate((transform) => Math.abs(new DOMMatrix(transform).m41), panelMotion.frames[0].transform);
  assert.equal(panelDistance, 36);
  assert.equal(await nav().getByRole("button", { name: "Comment", exact: true }).getAttribute("aria-current"), "page");
  const marker = workspace().locator(".selection-modes");
  const previous = await marker.boundingBox();
  await nav().getByRole("button", { name: "Write", exact: true }).click();
  await marker.evaluate((node) => { node.getAnimations().find((animation) => animation.id === "folio-selection-modes").currentTime = 0; });
  const interrupted = await marker.boundingBox();
  assert.ok(Math.abs(previous.x - interrupted.x) < 1 && Math.abs(previous.y - interrupted.y) < 1);
  await marker.evaluate((node) => { node.getAnimations().find((animation) => animation.id === "folio-selection-modes").currentTime = 100; });
  await page.waitForFunction(() => {
    const node = document.querySelector(".document-workspace:not([hidden]) .selection-modes");
    return node && Number(getComputedStyle(node, "::before").opacity) > .65;
  });
  const capture = async (name) => {
    const filename = join(output, name);
    await page.screenshot({ path: filename, animations: "allow" });
    evidence.screenshots.push(filename);
  };
  await capture("native-motion-light-selection.png");
  evidence.measurements.push({ state: "selection and panel", animations: modeMotion, interruptedFromVisiblePosition: true });
  await release();
  await page.waitForFunction(() => !document.querySelector(".selection-indicator[data-moving]")
    && [...document.querySelectorAll(".selection-indicator")].every((node) => !node.style.willChange));
  await pause();
  await nav().getByRole("button", { name: "Read", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".selection-indicator[data-moving]"));
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  assert.equal(await marker.getAttribute("data-moving"), null);
  assert.equal(await marker.evaluate((node) => node.style.willChange), "");
  await page.evaluate(() => {
    delete document.visibilityState;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await release();
  const about = page.getByRole("button", { name: "About Folio", exact: true });
  await about.focus();
  await about.click();
  await page.getByRole("dialog").waitFor();
  await page.getByRole("button", { name: "Close dialog", exact: true }).waitFor();
  await page.getByRole("dialog").press("Escape");
  await pause();
  await about.focus();
  await about.click();
  await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "Close dialog");
  const dialogMotion = (await page.evaluate(motionSnapshot)).find((animation) => animation.name === "dialog-enter");
  assert.ok(dialogMotion);
  const dialogGeometry = await page.evaluate((transform) => {
    const matrix = new DOMMatrix(transform); return { distance: matrix.m42, scale: matrix.m11 };
  }, dialogMotion.frames[0].transform);
  assert.equal(dialogGeometry.distance, 32);
  assert.ok(Math.abs(dialogGeometry.scale - .94) < .000001);
  assert.equal(await page.getByRole("dialog").evaluate((node) => getComputedStyle(node).opacity), "1");
  await capture("native-motion-light-dialog.png");
  await page.getByRole("dialog").press("Escape");
  await page.waitForFunction(() => !document.querySelector('[aria-modal="true"]'));
  assert.equal(await about.isEnabled(), true);
  await page.getByRole("button", { name: "Appearance", exact: true }).click();
  const appearanceMotion = (await page.evaluate(motionSnapshot)).find((animation) => animation.name === "appearance-enter");
  assert.ok(appearanceMotion);
  assert.equal(await page.evaluate((transform) => new DOMMatrix(transform).m42, appearanceMotion.frames[0].transform), -20);
  evidence.measurements.push({ state: "dialogs and appearance", dialog: dialogMotion, appearance: appearanceMotion });
  await page.getByRole("radio", { name: "Dark", exact: true }).check();
  await page.setViewportSize({ width: 980, height: 800 });
  await capture("native-motion-dark-compact.png");
  await page.getByRole("dialog", { name: "Appearance", exact: true }).press("Escape");
  assert.equal(await page.getByRole("button", { name: "Appearance", exact: true }).evaluate((node) => node === document.activeElement), true);
  await nav().getByRole("button", { name: "Comment", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".selection-indicator[data-moving]"));
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.waitForFunction(() => !document.querySelector(".selection-indicator[data-moving]"));
  assert.equal(await page.evaluate(() => document.getAnimations().some((animation) => animation.id.startsWith("folio-selection-"))), false);
  assert.equal(await workspace().locator(".tools-panel").evaluate((node) => getComputedStyle(node).animationName), "none");
  await page.getByRole("button", { name: "Appearance", exact: true }).click();
  await page.getByRole("radio", { name: "High Contrast", exact: true }).check();
  await page.setViewportSize({ width: 820, height: 700 });
  await capture("native-motion-contrast-compact.png");
  await page.getByRole("radio", { name: "Sepia", exact: true }).check();
  await page.getByRole("dialog", { name: "Appearance", exact: true }).press("Escape");
  await nav().getByRole("button", { name: "Read", exact: true }).click();
  await page.getByRole("button", { name: "All tools", exact: true }).click();
  if (await workspace().locator(".tools-panel").count() === 0) await page.getByRole("button", { name: "All tools", exact: true }).click();
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.waitForFunction(({ width, height }) => {
    const canvas = document.querySelector('.document-workspace:not([hidden]) canvas[aria-label="Rendered PDF page 1"]');
    return canvas?.width === width && canvas?.height === height;
  }, beforeSize);
  await ready();
  assert.equal(createHash("sha256").update(await canvasDigest()).digest("hex"),
    createHash("sha256").update(beforePixels).digest("hex"), "Settled PDF pixels must remain identical.");
  const sourceResponse = await fetch(`http://127.0.0.1:${backendPort}/bootstrap`, { headers: { "X-Folio-Preview": "1" } });
  const snapshot = await sourceResponse.json();
  assert.equal(snapshot.documents.length, 2);
  for (const document of snapshot.documents) {
    assert.equal(document.dirty, false); assert.equal(document.revision, 0);
    const bytes = await fetch(`http://127.0.0.1:${backendPort}/bytes`, { headers: {
      "X-Folio-Preview": "1", "X-Document-Id": String(document.id), "X-Revision": String(document.revision),
    } });
    assert.equal(bytes.ok, true);
    assert.equal(createHash("sha256").update(Buffer.from(await bytes.arrayBuffer())).digest("hex"),
      createHash("sha256").update(source).digest("hex"));
  }
  assert.deepEqual(evidence.errors, []);
  assert.deepEqual(evidence.externalRequests, []);
  evidence.checks.push("Noticeable 380ms selection/panel travel, 36px panel and 32px dialog geometry, 20px appearance opening and opaque dialog text.",
    "Interrupted selection continues from its visible position; finishing, hiding and reduced motion clean transient paint/compositing hints.",
    "Keyboard dismissal and appearance-focus return work; reduced motion removes travel; four palettes and native-supported compact widths remain usable.",
    "Real PDF pixels, exact source bytes, both document revisions and clean states remain unchanged.");
  await writeFile(join(output, "motion-evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
  console.log(JSON.stringify({ passed: true, checks: evidence.checks, output }, null, 2));
} catch (error) {
  evidence.failure = error.stack ?? String(error);
  if (page && !page.isClosed()) evidence.lastPage = await page.locator("body").innerText();
  await writeFile(join(output, "motion-failure.json"), JSON.stringify(evidence, null, 2) + "\n");
  console.error(JSON.stringify({ errors: evidence.errors, lastPage: evidence.lastPage?.slice(0, 1800) }, null, 2));
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
