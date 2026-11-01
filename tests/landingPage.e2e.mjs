import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const site = path.join(root, "site");
const output = path.resolve(process.env.FOLIO_SITE_REVIEW_DIR ?? path.join(root, ".impeccable/review/site"));
await mkdir(output, { recursive: true });
const mime = { ".html": "text/html; charset=utf-8", ".css": "text/css", ".mjs": "text/javascript",
  ".svg": "image/svg+xml", ".woff2": "font/woff2", ".webp": "image/webp",
  ".png": "image/png", ".pdf": "application/pdf", ".json": "application/json", ".xml": "application/xml" };
const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url, "http://127.0.0.1");
    if (!url.pathname.startsWith("/folio/")) { response.writeHead(404); response.end("Not found."); return; }
    const relative = decodeURIComponent(url.pathname.slice("/folio/".length)) || "index.html";
    const filename = path.resolve(site, relative);
    if (!filename.startsWith(site + path.sep)) { response.writeHead(403); response.end("Forbidden."); return; }
    const data = await readFile(filename);
    response.writeHead(200, { "Content-Type": mime[path.extname(filename)] ?? "application/octet-stream",
      "Content-Length": data.length, "Cache-Control": "no-store" });
    response.end(data);
  } catch (error) {
    if (error.code === "ENOENT") { response.writeHead(404); response.end("Not found."); }
    else { console.error("The owned site preview failed:", error); response.writeHead(500); response.end("Preview failed."); }
  }
});
await new Promise((resolve, reject) => {
  server.once("error", reject);
  server.listen(0, "127.0.0.1", resolve);
});
const address = server.address(), origin = `http://127.0.0.1:${address.port}`, url = origin + "/folio/";
let browser;
const errors = [], externalRequests = [], evidence = { url, viewports: [], behavior: [], screenshots: [] };
const cases = [
  { name: "desktop", width: 1440, height: 1000 },
  { name: "user-1280", width: 1280, height: 720, deviceScaleFactor: 2 },
  { name: "mobile", width: 390, height: 844, isMobile: true, hasTouch: true,
    userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1" },
  { name: "narrow", width: 320, height: 760, isMobile: true, hasTouch: true,
    userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1" },
];
async function context(settings = {}) {
  const value = await browser.newContext({ reducedMotion: "reduce", acceptDownloads: true, ...settings });
  await value.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    externalRequests.push(route.request().url()); return route.abort();
  });
  return value;
}
try {
  browser = await chromium.launch({ headless: true,
    ...(process.env.FOLIO_SITE_BROWSER ? { executablePath: process.env.FOLIO_SITE_BROWSER } : {}) });
  for (const item of cases) {
    const ctx = await context({ viewport: { width: item.width, height: item.height },
      deviceScaleFactor: item.deviceScaleFactor ?? 1, isMobile: item.isMobile ?? false,
      hasTouch: item.hasTouch ?? false, ...(item.userAgent ? { userAgent: item.userAgent } : {}) });
    const page = await ctx.newPage();
    page.on("pageerror", (error) => errors.push(error.message));
    await page.addInitScript(() => {
      window.folioLayoutShift = 0;
      new PerformanceObserver((list) => {
        for (const value of list.getEntries()) if (!value.hadRecentInput) window.folioLayoutShift += value.value;
      }).observe({ type: "layout-shift", buffered: true });
    });
    await page.goto(url);
    await page.evaluate(() => document.fonts.ready);
    await page.locator("#preview-image").evaluate((image) => image.decode());
    const measurements = await page.evaluate(() => {
      const rect = (id) => {
        const value = document.getElementById(id).getBoundingClientRect();
        return { top: value.top, bottom: value.bottom, width: value.width, height: value.height };
      };
      const color = (value) => {
        const parts = value.match(/[\d.]+/g)?.map(Number);
        return parts?.length >= 3 ? [...parts.slice(0, 3), parts[3] ?? 1] : [0, 0, 0, 0];
      };
      const blend = (front, back) => front.slice(0, 3).map((value, index) => value * front[3] + back[index] * (1 - front[3]));
      const luminance = (rgb) => rgb.map((value) => {
        const channel = value / 255;
        return channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4;
      }).reduce((sum, value, index) => sum + value * [.2126, .7152, .0722][index], 0);
      const background = (element) => {
        const layers = [];
        for (let node = element; node; node = node.parentElement) layers.unshift(color(getComputedStyle(node).backgroundColor));
        return layers.reduce((result, layer) => blend(layer, result), [255, 255, 255]);
      };
      const failures = [], walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
      while (walker.nextNode()) {
        const node = walker.currentNode, parent = node.parentElement, text = node.textContent.trim();
        if (!text || !parent || ["SCRIPT", "STYLE", "NOSCRIPT"].includes(parent.tagName)) continue;
        const range = document.createRange(); range.selectNodeContents(node);
        if (!range.getBoundingClientRect().width || !range.getBoundingClientRect().height) continue;
        const style = getComputedStyle(parent), bg = background(parent), foreground = blend(color(style.color), bg);
        const levels = [luminance(foreground), luminance(bg)].sort((a, b) => b - a);
        const ratio = (levels[0] + .05) / (levels[1] + .05), size = parseFloat(style.fontSize);
        const threshold = size >= 24 || (Number(style.fontWeight) >= 700 && size >= 18.67) ? 3 : 4.5;
        if (ratio + .01 < threshold) failures.push({ text: text.slice(0, 90), ratio, threshold });
      }
      return { width: innerWidth, height: innerHeight,
        overflow: Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) > innerWidth,
        primaryDownload: rect("primary-download"), screenshot: rect("preview-image"),
        sourceNotice: document.querySelector(".preview-build-note").getBoundingClientRect().top,
        displaySize: getComputedStyle(document.querySelector("h1")).fontSize,
        fontLoaded: document.fonts.check('600 60px "Commissioner"'),
        contrastFailures: failures, layoutShift: window.folioLayoutShift,
        initialResourceBytes: performance.getEntriesByType("resource").reduce((sum, value) => sum + value.encodedBodySize, 0),
        imageSource: document.getElementById("preview-image").currentSrc,
      };
    });
    assert.equal(measurements.overflow, false, JSON.stringify(measurements));
    assert.equal(measurements.fontLoaded, true);
    assert.deepEqual(measurements.contrastFailures, []);
    assert.ok(measurements.primaryDownload.bottom <= item.height, "The main action must be available in the first viewport.");
    const detail = measurements.imageSource.endsWith("-detail.webp");
    const documentHeadingBottom = measurements.screenshot.top
      + (276 - (detail ? 95 : 0)) * measurements.screenshot.width / (detail ? 900 : 1440);
    assert.ok(documentHeadingBottom < item.height - 8,
      "The real sample document heading, not just toolbar chrome, must be visible in the first viewport.");
    measurements.documentHeadingBottom = documentHeadingBottom;
    assert.ok(measurements.initialResourceBytes < 250_000, "Initial resources must remain below 250 kB.");
    const filename = path.join(output, `${item.name}.png`);
    await page.screenshot({ path: filename, fullPage: true });
    await page.screenshot({ path: path.join(output, `${item.name}-first-viewport.png`) });
    evidence.screenshots.push(filename, path.join(output, `${item.name}-first-viewport.png`));
    evidence.viewports.push({ name: item.name, ...measurements });
    await ctx.close();
  }

  const ctx = await context({ viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url);
  await page.keyboard.press("Tab");
  assert.equal(await page.locator(":focus").innerText(), "Skip to content");
  await page.getByRole("tab", { name: "Read & review", exact: true }).focus();
  await page.keyboard.press("ArrowRight");
  assert.equal(await page.getByRole("tab", { name: "Leave a note", exact: true }).getAttribute("aria-selected"), "true");
  assert.match(await page.locator("#preview-image").getAttribute("src"), /workspace-comment\.webp$/);
  await page.keyboard.press("End");
  assert.equal(await page.getByRole("tab", { name: "Organize pages", exact: true }).getAttribute("aria-selected"), "true");
  await page.keyboard.press("Home");
  assert.equal(await page.getByRole("tab", { name: "Read & review", exact: true }).getAttribute("aria-selected"), "true");
  evidence.behavior.push("Keyboard skip link, roving proof tabs and matching actual-image captions.");
  const downloadEvent = page.waitForEvent("download");
  await page.getByRole("link", { name: "Try this sample PDF", exact: true }).click();
  const download = await downloadEvent, saved = path.join(output, "Downloaded-original-sample.pdf");
  await download.saveAs(saved);
  const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
  assert.equal(hash(await readFile(saved)), hash(await readFile(path.join(site, "assets/try-folio.pdf"))));
  evidence.behavior.push("Real downloadable original sample PDF matches the published bytes.");
  await ctx.grantPermissions(["clipboard-read", "clipboard-write"], { origin });
  await page.evaluate(() => Object.defineProperty(navigator, "share", { configurable: true, value: undefined }));
  await page.getByRole("button", { name: "Pass Folio on", exact: true }).click();
  await page.getByRole("status").filter({ hasText: "Link copied" }).waitFor();
  assert.equal(await page.evaluate(() => navigator.clipboard.readText()), "https://vertex-shift.github.io/folio/");
  evidence.behavior.push("Actual desktop clipboard sharing writes the canonical public page URL.");
  await page.evaluate(() => Object.defineProperty(navigator, "share", { configurable: true,
    value: () => Promise.reject(new DOMException("Canceled", "AbortError")) }));
  await page.getByRole("button", { name: "Pass Folio on", exact: true }).click();
  await page.getByRole("status").filter({ hasText: "Share canceled." }).waitFor();
  assert.equal(await page.locator("#share-fallback").isHidden(), true);
  await page.evaluate(() => Object.defineProperty(navigator, "share", { configurable: true,
    value: () => Promise.reject(new Error("Permission denied")) }));
  await page.getByRole("button", { name: "Pass Folio on", exact: true }).click();
  await page.locator("#share-fallback").waitFor();
  assert.equal(await page.locator(":focus").getAttribute("id"), "share-address");
  assert.equal(await page.getByRole("button", { name: "Pass Folio on", exact: true }).isEnabled(), true);
  evidence.behavior.push("Simulated native-share cancel/denial preserves control state and exposes a focused manual-copy recovery.");
  await page.locator(".share-note").screenshot({ path: path.join(output, "share-fallback.png") });
  evidence.screenshots.push(path.join(output, "share-fallback.png"));
  await ctx.close();

  const noJs = await context({ javaScriptEnabled: false, viewport: { width: 390, height: 844 } });
  const plain = await noJs.newPage();
  await plain.goto(url);
  assert.equal(await plain.locator(".preview-tabs").isHidden(), true);
  assert.equal(await plain.locator("#share-button").isHidden(), true);
  assert.ok(await plain.locator("[data-platform]").count() === 3);
  await plain.getByText("Is Folio really free?", { exact: true }).click();
  assert.equal(await plain.locator("details").first().getAttribute("open"), "");
  evidence.behavior.push("No-JavaScript users keep real installer links, the sample PDF, native FAQ disclosure and no dead enhancement controls.");
  await noJs.close();
  assert.deepEqual(errors, []); assert.deepEqual(externalRequests, []);
  evidence.browserErrors = errors; evidence.externalRequests = externalRequests;
  await writeFile(path.join(output, "landing-evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
  console.log(JSON.stringify({ passed: true, viewports: evidence.viewports, behavior: evidence.behavior,
    browserErrors: errors, externalRequests, screenshotDirectory: output }, null, 2));
} finally {
  if (browser) await browser.close();
  await new Promise((resolve) => server.close(resolve));
}
