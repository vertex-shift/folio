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
const mime = { ".html": "text/html; charset=utf-8", ".css": "text/css", ".mjs": "text/javascript", ".js": "text/javascript",
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
const errors = [], externalRequests = [], evidence = { url, viewports: [], behavior: [], screenshots: [], motion: [], themes: [] };
const colorThemes = [
  { preference: "sepia", background: "rgb(247, 241, 231)", colorScheme: "light" },
  { preference: "forest", background: "rgb(243, 248, 242)", colorScheme: "light" },
  { preference: "midnight", background: "rgb(17, 27, 48)", colorScheme: "dark" },
];
const cases = [
  { name: "desktop", width: 1440, height: 1000 },
  { name: "user-1280", width: 1280, height: 720, deviceScaleFactor: 2 },
  { name: "mobile", width: 390, height: 844, isMobile: true, hasTouch: true,
    userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1" },
  { name: "narrow", width: 320, height: 760, isMobile: true, hasTouch: true,
    userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1" },
];
cases.push(...cases.map((item) => ({ ...item, name: `${item.name}-dark`, colorScheme: "dark" })));
for (const theme of colorThemes) {
  for (const viewport of [{ width: 1440, height: 1000 }, { width: 320, height: 760 }]) {
    cases.push({ ...viewport, name: `${theme.preference}-${viewport.width}`,
      appearance: theme.preference, expectedBackground: theme.background,
      colorScheme: theme.colorScheme === "light" ? "dark" : "light" });
  }
}
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
      colorScheme: item.colorScheme ?? "light",
      deviceScaleFactor: item.deviceScaleFactor ?? 1, isMobile: item.isMobile ?? false,
      hasTouch: item.hasTouch ?? false, ...(item.userAgent ? { userAgent: item.userAgent } : {}) });
    const page = await ctx.newPage();
    page.on("pageerror", (error) => errors.push(error.message));
    await page.addInitScript(({ preference, origin }) => {
      if (location.origin !== origin) return;
      localStorage.setItem("folio.appearance", "contrast");
      if (preference) localStorage.setItem("folio.site.appearance", preference);
    }, { preference: item.appearance, origin });
    await page.addInitScript(() => {
      window.folioLayoutShift = 0;
      new PerformanceObserver((list) => {
        for (const value of list.getEntries()) if (!value.hadRecentInput) window.folioLayoutShift += value.value;
      }).observe({ type: "layout-shift", buffered: true });
    });
    if (item.appearance && item.width === 1440) {
      let releaseStyles;
      const stylesReady = new Promise((resolve) => { releaseStyles = resolve; });
      await ctx.route("**/styles.css", async (route) => {
        await stylesReady;
        await route.continue();
      });
      try {
        await page.goto(url, { waitUntil: "commit" });
        await page.waitForFunction((preference) => document.documentElement.dataset.appearance === preference, item.appearance);
        assert.equal(await page.evaluate(() => window.folioSiteAppearance.snapshot().preference), item.appearance);
      } finally { releaseStyles(); }
      await page.waitForLoadState("load");
    } else await page.goto(url);
    await page.evaluate(() => document.fonts.ready);
    await page.locator("#preview-image").evaluate((image) => image.decode());
    assert.equal(await page.getByRole("combobox", { name: "Website appearance", exact: true }).inputValue(), item.appearance ?? "system");
    assert.equal(await page.evaluate(() => localStorage.getItem("folio.appearance")), "contrast");
    assert.equal(await page.evaluate(() => document.getAnimations().filter((value) => value.id.startsWith("folio-")).length), 0,
      "Reduced motion must skip the document-opening geometry.");
    const measurements = await page.evaluate(() => {
      const rect = (id) => {
        const value = document.getElementById(id).getBoundingClientRect();
        return { top: value.top, bottom: value.bottom, width: value.width, height: value.height };
      };
      const color = (value) => {
        if (/^#[\da-f]{6}$/i.test(value)) return [1, 3, 5].map((index) => parseInt(value.slice(index, index + 2), 16)).concat(1);
        const parts = value.match(/[\d.]+/g)?.map(Number);
        return parts?.length >= 3 ? [...parts.slice(0, 3), parts[3] ?? 1] : [0, 0, 0, 0];
      };
      const blend = (front, back) => front.slice(0, 3).map((value, index) => value * front[3] + back[index] * (1 - front[3]));
      const luminance = (rgb) => rgb.slice(0, 3).map((value) => {
        const channel = value / 255;
        return channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4;
      }).reduce((sum, value, index) => sum + value * [.2126, .7152, .0722][index], 0);
      const background = (element) => {
        const layers = [];
        for (let node = element; node; node = node.parentElement) layers.unshift(color(getComputedStyle(node).backgroundColor));
        return layers.reduce((result, layer) => blend(layer, result), [255, 255, 255]);
      };
      const contrast = (foreground, background) => {
        const levels = [luminance(foreground), luminance(background)].sort((a, b) => b - a);
        return (levels[0] + .05) / (levels[1] + .05);
      };
      const failures = [], walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
      while (walker.nextNode()) {
        const node = walker.currentNode, parent = node.parentElement, text = node.textContent.trim();
        if (!text || !parent || ["SCRIPT", "STYLE", "NOSCRIPT"].includes(parent.tagName)) continue;
        const range = document.createRange(); range.selectNodeContents(node);
        if (!range.getBoundingClientRect().width || !range.getBoundingClientRect().height) continue;
        const style = getComputedStyle(parent), bg = background(parent), foreground = blend(color(style.color), bg);
        const ratio = contrast(foreground, bg), size = parseFloat(style.fontSize);
        const threshold = size >= 24 || (Number(style.fontWeight) >= 700 && size >= 18.67) ? 3 : 4.5;
        if (ratio + .01 < threshold) failures.push({ text: text.slice(0, 90), ratio, threshold });
      }
      const tokens = getComputedStyle(document.documentElement);
      const paletteContrast = ["sepia", "forest", "midnight"].includes(document.documentElement.dataset.appearance)
        ? [
          ["--ink", "--paper", 4.5], ["--muted", "--canvas", 4.5],
          ["--blue-ink", "--soft-blue", 4.5], ["--on-primary", "--blue", 4.5],
          ["--on-primary", "--primary-hover", 4.5], ["--ink", "--selection", 4.5],
          ["--control-border", "--paper", 3], ["--control-hover-border", "--paper", 3],
          ["--selected-border", "--soft-blue", 3], ["--focus", "--paper", 3],
          ["--focus", "--canvas", 3], ["--scrollbar", "--canvas", 3],
        ].map(([foreground, background, threshold]) => ({
          foreground, background, threshold,
          ratio: contrast(color(tokens.getPropertyValue(foreground).trim()), color(tokens.getPropertyValue(background).trim())),
        })) : [];
      return { width: innerWidth, height: innerHeight,
        overflow: Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) > innerWidth,
        primaryDownload: rect("primary-download"), screenshot: rect("preview-image"),
        sourceNotice: document.querySelector(".preview-build-note").getBoundingClientRect().top,
        displaySize: getComputedStyle(document.querySelector("h1")).fontSize,
        fontLoaded: document.fonts.check('600 60px "Commissioner"'),
        contrastFailures: failures, layoutShift: window.folioLayoutShift,
        initialResourceBytes: performance.getEntriesByType("resource").reduce((sum, value) => sum + value.encodedBodySize, 0),
        imageSource: document.getElementById("preview-image").currentSrc,
        appearance: document.documentElement.dataset.appearance,
        background: getComputedStyle(document.body).backgroundColor, paletteContrast,
        imageFilter: getComputedStyle(document.getElementById("preview-image")).filter,
        sourceBackground: getComputedStyle(document.getElementById("source-chapter")).backgroundColor,
      };
    });
    assert.equal(measurements.overflow, false, JSON.stringify(measurements));
    assert.equal(measurements.fontLoaded, true);
    assert.deepEqual(measurements.contrastFailures, []);
    assert.ok(measurements.paletteContrast.every((item) => item.ratio >= item.threshold),
      JSON.stringify(measurements.paletteContrast));
    assert.equal(measurements.imageFilter, "none");
    assert.equal(measurements.sourceBackground, "rgb(34, 90, 230)");
    if (item.expectedBackground) assert.equal(measurements.background, item.expectedBackground);
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
    if (item.appearance) {
      const select = page.getByRole("combobox", { name: "Website appearance", exact: true });
      await select.focus();
      const focus = await select.evaluate((node) => {
        const style = getComputedStyle(node);
        const measure = document.createElement("span");
        measure.style.font = style.font;
        measure.textContent = node.selectedOptions[0].textContent;
        document.body.append(measure);
        const labelWidth = measure.getBoundingClientRect().width;
        measure.remove();
        return { outline: style.outlineStyle, color: style.outlineColor,
          labelFits: labelWidth <= node.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight) - 18 };
      });
      assert.equal(focus.outline, "solid");
      assert.equal(focus.labelFits, true, `${item.appearance} must fit in the native selector at ${item.width}px.`);
      await page.locator("#primary-download").hover();
      const hover = await page.locator("#primary-download").evaluate((node) => ({
        background: getComputedStyle(node).backgroundColor,
        expected: getComputedStyle(document.documentElement).getPropertyValue("--primary-hover").trim(),
      }));
      const rgb = hover.expected.match(/[\da-f]{2}/gi).map((value) => parseInt(value, 16));
      assert.equal(hover.background, `rgb(${rgb.join(", ")})`);
      await page.emulateMedia({ media: "print" });
      assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(255, 255, 255)");
      assert.equal(await page.evaluate(() => getComputedStyle(document.documentElement).colorScheme), "light");
    }
    await ctx.close();
  }

  for (const item of [
    { name: "motion-desktop", width: 1280, height: 720, colorScheme: "light" },
    { name: "motion-mobile-dark", width: 390, height: 844, colorScheme: "dark" },
  ]) {
    const motionContext = await context({ reducedMotion: "no-preference", colorScheme: item.colorScheme,
      viewport: { width: item.width, height: item.height } });
    const moving = await motionContext.newPage();
    moving.on("pageerror", (error) => errors.push(error.message));
    await moving.addInitScript(() => {
      window.folioPauseMotion = true;
      window.folioMotionCalls = [];
      window.folioMotionLayoutShift = 0;
      new PerformanceObserver((list) => {
        for (const value of list.getEntries()) if (!value.hadRecentInput) window.folioMotionLayoutShift += value.value;
      }).observe({ type: "layout-shift", buffered: true });
      const animate = Element.prototype.animate;
      Element.prototype.animate = function (frames, options) {
        const animation = animate.call(this, frames, options);
        window.folioMotionCalls.push({ target: this.id || this.tagName, frames, duration: options.duration });
        if (window.folioPauseMotion) { animation.pause(); animation.currentTime = 90; }
        return animation;
      };
    });
    await moving.goto(url);
    await moving.waitForFunction(() => document.getAnimations().some((value) => value.id === "folio-document-open"));
    const entrance = await moving.evaluate(() => {
      const frame = document.getElementById("workspace-preview"), download = document.getElementById("primary-download");
      return {
        animations: document.getAnimations().filter((value) => value.id.startsWith("folio-"))
          .map((value) => ({ id: value.id, duration: value.effect.getTiming().duration, iterations: value.effect.getTiming().iterations })),
        transform: getComputedStyle(frame).transform, clipPath: getComputedStyle(frame).clipPath,
        downloadBottom: download.getBoundingClientRect().bottom,
        overflow: document.documentElement.scrollWidth > innerWidth,
      };
    });
    assert.deepEqual(entrance.animations.map((value) => value.id).sort(), ["folio-document-open", "folio-ownership-reveal"]);
    assert.ok(entrance.animations.every((value) => value.iterations === 1 && value.duration <= 800));
    assert.notEqual(entrance.transform, "none");
    assert.notEqual(entrance.clipPath, "none");
    assert.ok(entrance.downloadBottom <= item.height);
    assert.equal(entrance.overflow, false);
    const entranceFile = path.join(output, `${item.name}-opening.png`);
    await moving.screenshot({ path: entranceFile, animations: "allow" });
    evidence.screenshots.push(entranceFile);
    const timing = await moving.evaluate(async () => {
      window.folioPauseMotion = false;
      for (const animation of document.getAnimations()) animation.play();
      const deltas = [];
      let last = performance.now();
      for (let index = 0; index < 48; index++) {
        await new Promise(requestAnimationFrame);
        const now = performance.now(); deltas.push(now - last); last = now;
      }
      deltas.sort((a, b) => a - b);
      return { frameP95: deltas[Math.floor(deltas.length * .95)], layoutShift: window.folioMotionLayoutShift };
    });
    assert.ok(timing.frameP95 < 50, JSON.stringify(timing));
    assert.ok(timing.layoutShift < .01, JSON.stringify(timing));
    await moving.waitForFunction(() => document.getAnimations().filter((value) => value.id.startsWith("folio-")).length === 0);
    assert.equal(await moving.locator("#workspace-preview").evaluate((node) => getComputedStyle(node).transform), "none");
    assert.equal(await moving.locator("#workspace-preview").evaluate((node) => node.style.willChange), "");
    const note = moving.getByRole("tab", { name: "Leave a note", exact: true });
    const organize = moving.getByRole("tab", { name: "Organize pages", exact: true });
    await moving.evaluate(() => { window.folioPauseMotion = true; });
    await note.click();
    await moving.waitForFunction(() => document.getAnimations().some((value) => value.id === "folio-preview-sheet"));
    await organize.click();
    await moving.waitForFunction(() => document.getElementById("preview-image").complete
      && document.getAnimations().some((value) => value.id === "folio-preview-sheet"));
    assert.match(await moving.locator("#preview-image").getAttribute("src"), /workspace-organize\.webp$/);
    assert.equal(await organize.getAttribute("aria-selected"), "true");
    assert.equal(await note.getAttribute("aria-selected"), "false");
    assert.equal(await moving.evaluate(() => document.getAnimations().filter((value) => value.id === "folio-preview-sheet").length), 1);
    await moving.emulateMedia({ reducedMotion: "reduce" });
    await moving.waitForFunction(() => document.getAnimations().filter((value) => value.id.startsWith("folio-")).length === 0);
    await note.focus();
    await moving.keyboard.press("Enter");
    await moving.waitForFunction(() => document.getAnimations().some((value) => value.id === "folio-preview-sheet"));
    const reduced = await moving.evaluate(() => document.getAnimations().filter((value) => value.id.startsWith("folio-"))
      .map((value) => ({ duration: value.effect.getTiming().duration, frames: value.effect.getKeyframes() })));
    assert.ok(reduced.every((value) => value.duration === 120 && value.frames.every((frame) => !frame.transform && !frame.clipPath)));
    await moving.emulateMedia({ media: "print" });
    await moving.evaluate(() => dispatchEvent(new Event("beforeprint")));
    await moving.waitForFunction(() => document.getAnimations().filter((value) => value.id.startsWith("folio-")).length === 0);
    await moving.emulateMedia({ media: "screen", reducedMotion: "no-preference" });
    await organize.click();
    await moving.waitForFunction(() => document.getAnimations().some((value) => value.id === "folio-preview-sheet"));
    await moving.evaluate(() => {
      Object.defineProperty(document, "hidden", { configurable: true, value: true });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    assert.equal(await moving.evaluate(() => document.getAnimations().filter((value) => value.id.startsWith("folio-")).length), 0);
    await moving.evaluate(() => delete document.hidden);
    await moving.evaluate(() => dispatchEvent(new Event("pagehide")));
    await moving.evaluate(() => new Promise(requestAnimationFrame));
    assert.equal(await moving.locator("#workspace-preview").evaluate((node) => node.style.willChange), "");
    evidence.motion.push({ name: item.name, entrance, timing, reducedMotion: "crossfade only", interruption: "latest preview wins",
      hiddenPage: "canceled", print: "canceled", noLoops: true });
    await motionContext.close();
  }
  evidence.behavior.push("Prominent document-opening and preview-sheet motion passes on desktop/mobile; bounded frame timing, no layout shift, interruption, reduced-motion crossfades and hidden/print cancellation.");

  const delayed = await context({ reducedMotion: "no-preference", viewport: { width: 1280, height: 900 } });
  let releaseInitialImage;
  const initialImage = new Promise((resolve) => { releaseInitialImage = resolve; });
  await delayed.route("**/assets/workspace-read.webp", async (route) => {
    await initialImage;
    await route.fulfill({ status: 200, contentType: "image/webp",
      body: await readFile(path.join(site, "assets/workspace-read.webp")) });
  });
  const latest = await delayed.newPage();
  latest.on("pageerror", (error) => errors.push(error.message));
  await latest.goto(url, { waitUntil: "domcontentloaded" });
  await latest.getByRole("tab", { name: "Organize pages", exact: true }).click();
  await latest.waitForFunction(() => document.getElementById("preview-image").complete
    && document.getElementById("preview-image").currentSrc.endsWith("/workspace-organize.webp"));
  releaseInitialImage();
  await latest.waitForLoadState("networkidle");
  assert.equal(await latest.getByRole("tab", { name: "Organize pages", exact: true }).getAttribute("aria-selected"), "true");
  assert.match(await latest.locator("#preview-caption").innerText(), /Every page in its place/);
  assert.equal(await latest.evaluate(() => document.getAnimations().some((value) => value.id === "folio-document-open")), false);
  await latest.setViewportSize({ width: 390, height: 844 });
  await latest.waitForFunction(() => document.getElementById("preview-image").complete
    && document.getElementById("preview-image").currentSrc.endsWith("/workspace-organize-detail.webp"));
  assert.match(await latest.locator("#preview-caption").innerText(), /Every page in its place/);
  await delayed.close();
  evidence.behavior.push("A delayed/canceled initial image cannot replay the entrance or replace the latest preview; responsive source changes preserve the selected view.");

  const ctx = await context({ viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url);
  const discovery = await page.evaluate(async () => {
    const title = document.title;
    const meta = (name) => document.querySelector(`meta[name="${name}"], meta[property="${name}"]`)?.content;
    const graph = JSON.parse(document.querySelector('script[type="application/ld+json"]').textContent)["@graph"];
    const response = await fetch(new URL("./sitemap.xml", location.href));
    const sitemap = new DOMParser().parseFromString(await response.text(), "application/xml");
    return {
      title, description: meta("description"), socialTitle: meta("og:title"), twitterTitle: meta("twitter:title"),
      robots: meta("robots"), canonical: document.querySelector('link[rel="canonical"]').href,
      graphTypes: graph.map((item) => item["@type"]),
      sitemapStatus: response.status, xmlErrors: sitemap.querySelectorAll("parsererror").length,
      sitemapNamespace: sitemap.documentElement.namespaceURI,
      sitemapPages: Array.from(sitemap.getElementsByTagNameNS("http://www.sitemaps.org/schemas/sitemap/0.9", "loc"), (node) => node.textContent),
      sitemapImages: Array.from(sitemap.getElementsByTagNameNS("http://www.google.com/schemas/sitemap-image/1.1", "loc"), (node) => node.textContent),
    };
  });
  assert.equal(discovery.title, "Folio | Free Open-Source PDF Editor & Acrobat Alternative");
  assert.equal(discovery.socialTitle, discovery.title);
  assert.equal(discovery.twitterTitle, discovery.title);
  assert.match(discovery.description, /edit supported text/);
  assert.equal(discovery.robots, "index,follow,max-image-preview:large");
  assert.equal(discovery.canonical, "https://vertex-shift.github.io/folio/");
  assert.deepEqual(discovery.graphTypes, ["WebPage", "SoftwareApplication", "SoftwareSourceCode", "ImageObject"]);
  assert.equal(discovery.sitemapStatus, 200);
  assert.equal(discovery.xmlErrors, 0);
  assert.equal(discovery.sitemapNamespace, "http://www.sitemaps.org/schemas/sitemap/0.9");
  assert.deepEqual(discovery.sitemapPages, [discovery.canonical]);
  assert.deepEqual(discovery.sitemapImages, ["read", "comment", "organize"].map((view) => `${discovery.canonical}assets/workspace-${view}.webp`));
  evidence.behavior.push("Real DOM search/social metadata and linked JSON-LD parse correctly; XML parser verifies the canonical/image sitemap.");
  evidence.discovery = discovery;
  await page.keyboard.press("Tab");
  assert.equal(await page.locator(":focus").innerText(), "Skip to content");
  const theme = page.getByRole("combobox", { name: "Website appearance", exact: true });
  const background = () => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  await page.emulateMedia({ colorScheme: "light" });
  const light = await background();
  await page.emulateMedia({ colorScheme: "dark" });
  const dark = await background();
  assert.notEqual(light, dark);
  assert.equal(await theme.inputValue(), "system");
  await theme.selectOption("light");
  assert.equal(await background(), light);
  await page.reload();
  assert.equal(await theme.inputValue(), "light");
  assert.equal(await background(), light);
  await theme.selectOption("dark");
  await page.emulateMedia({ colorScheme: "light" });
  assert.equal(await background(), dark);
  await page.reload();
  assert.equal(await theme.inputValue(), "dark");
  await theme.selectOption("system");
  assert.equal(await background(), light);
  await page.emulateMedia({ colorScheme: "dark" });
  assert.equal(await background(), dark);
  await page.emulateMedia({ colorScheme: "light" });
  assert.equal(await page.evaluate(() => localStorage.getItem("folio.site.appearance")), "system");
  evidence.behavior.push("System follows live OS changes; explicit Light/Dark overrides persist across reloads with isolated website storage.");
  await page.evaluate(() => localStorage.setItem("folio.appearance", "contrast"));
  for (const item of colorThemes) {
    await theme.selectOption(item.preference);
    for (const os of ["dark", "light"]) {
      await page.emulateMedia({ colorScheme: os });
      assert.equal(await background(), item.background);
      assert.equal(await page.evaluate(() => getComputedStyle(document.documentElement).colorScheme), item.colorScheme);
    }
    await page.reload();
    assert.equal(await theme.inputValue(), item.preference);
    assert.equal(await background(), item.background);
    assert.equal(await page.evaluate(() => localStorage.getItem("folio.appearance")), "contrast");
    const styles = await page.evaluate(() => ({
      image: document.getElementById("preview-image").currentSrc,
      sourceBackground: getComputedStyle(document.getElementById("source-chapter")).backgroundColor,
      selection: getComputedStyle(document.querySelector(".lead"), "::selection").backgroundColor,
    }));
    assert.match(styles.image, /workspace-read\.webp$/);
    assert.equal(styles.sourceBackground, "rgb(34, 90, 230)");
    evidence.themes.push({ preference: item.preference, background: item.background,
      colorScheme: item.colorScheme, persistence: true, followsOS: false, ...styles });
  }
  await theme.selectOption("system");
  await theme.focus();
  await page.keyboard.press("m");
  assert.equal(await theme.inputValue(), "midnight");
  assert.equal(await background(), colorThemes[2].background);
  await theme.selectOption("system");
  assert.equal(await theme.inputValue(), "system");
  assert.equal(await background(), light);
  await theme.blur();
  evidence.behavior.push("Sepia, Forest and Midnight restore before CSS, persist across reloads and stay fixed across OS changes; native keyboard choices and isolated desktop storage are preserved.");
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

  const noJs = await context({ javaScriptEnabled: false, colorScheme: "dark", viewport: { width: 390, height: 844 } });
  const plain = await noJs.newPage();
  await plain.goto(url);
  assert.equal(await plain.title(), discovery.title);
  assert.equal(await plain.locator('link[rel="canonical"]').getAttribute("href"), discovery.canonical);
  assert.equal(await plain.locator('script[type="application/ld+json"]').count(), 1);
  assert.match(await plain.locator("main").textContent(), /not a complete Acrobat replacement/);
  assert.equal(await plain.locator(".preview-tabs").isHidden(), true);
  assert.equal(await plain.locator("#share-button").isHidden(), true);
  assert.equal(await plain.locator("#appearance-control").isHidden(), true);
  assert.equal(await plain.evaluate(() => getComputedStyle(document.body).backgroundColor), dark);
  assert.ok(await plain.locator("[data-platform]").count() === 3);
  await plain.getByText("Is Folio really free?", { exact: true }).click();
  assert.equal(await plain.locator("details").first().getAttribute("open"), "");
  evidence.behavior.push("No-JavaScript users follow the OS dark theme and keep real installer links, the sample PDF, native FAQ disclosure and no dead enhancement controls.");
  await noJs.close();
  const blocked = await context({ viewport: { width: 390, height: 844 }, colorScheme: "dark" });
  await blocked.addInitScript(() => {
    Object.defineProperty(window, "localStorage", { get() { throw new Error("Storage blocked"); } });
  });
  const temporary = await blocked.newPage();
  await temporary.goto(url);
  await temporary.getByRole("status").filter({ hasText: "Browser storage is unavailable" }).waitFor();
  await temporary.getByRole("combobox", { name: "Website appearance", exact: true }).selectOption("light");
  assert.equal(await temporary.evaluate(() => getComputedStyle(document.body).backgroundColor), light);
  await temporary.getByRole("status").filter({ hasText: "could not be saved" }).waitFor();
  for (const item of colorThemes) {
    await temporary.getByRole("combobox", { name: "Website appearance", exact: true }).selectOption(item.preference);
    assert.equal(await temporary.evaluate(() => getComputedStyle(document.body).backgroundColor), item.background);
    await temporary.getByRole("status").filter({ hasText: "could not be saved" }).waitFor();
  }
  evidence.behavior.push("Blocked browser storage surfaces a visible notice while visit-only appearance selection still works.");
  await blocked.close();
  const invalid = await context({ viewport: { width: 390, height: 844 }, colorScheme: "dark" });
  await invalid.addInitScript((origin) => {
    if (location.origin === origin) localStorage.setItem("folio.site.appearance", "not-a-theme");
  }, origin);
  const recovered = await invalid.newPage();
  await recovered.goto(url);
  assert.equal(await recovered.getByRole("combobox", { name: "Website appearance", exact: true }).inputValue(), "system");
  assert.equal(await recovered.evaluate(() => getComputedStyle(document.body).backgroundColor), dark);
  await recovered.getByRole("status").filter({ hasText: "not recognized" }).waitFor();
  await recovered.getByRole("combobox", { name: "Website appearance", exact: true }).selectOption("forest");
  assert.equal(await recovered.locator("#appearance-status").isHidden(), true);
  assert.equal(await recovered.evaluate(() => localStorage.getItem("folio.site.appearance")), "forest");
  await invalid.close();
  evidence.behavior.push("An unknown saved theme explicitly recovers to System and a valid new selection replaces it.");
  assert.deepEqual(errors, []); assert.deepEqual(externalRequests, []);
  evidence.browserErrors = errors; evidence.externalRequests = externalRequests;
  await writeFile(path.join(output, "landing-evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
  console.log(JSON.stringify({ passed: true, viewports: evidence.viewports, behavior: evidence.behavior,
    browserErrors: errors, externalRequests, screenshotDirectory: output }, null, 2));
} finally {
  if (browser) await browser.close();
  await new Promise((resolve) => server.close(resolve));
}
