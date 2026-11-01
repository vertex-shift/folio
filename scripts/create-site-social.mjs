import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const directory = resolve(root, "site/assets");
const font = (await readFile(resolve(directory, "fonts/commissioner-latin.woff2"))).toString("base64");
const logo = (await readFile(resolve(directory, "folio.svg"))).toString("base64");
const workspace = (await readFile(resolve(directory, "workspace-read.webp"))).toString("base64");
const browser = await chromium.launch({ headless: true,
  ...(process.env.FOLIO_SITE_BROWSER ? { executablePath: process.env.FOLIO_SITE_BROWSER } : {}) });
try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, reducedMotion: "reduce" });
  await page.setContent(`<!doctype html><html lang="en"><head><style>
    @font-face{font-family:Commissioner;src:url(data:font/woff2;base64,${font}) format("woff2");font-weight:400 700}
    *{box-sizing:border-box}body{margin:0;width:1200px;height:630px;background:#225ae6;color:white;font-family:Commissioner,sans-serif}
    .brand{position:absolute;left:48px;top:37px;display:flex;align-items:center;gap:12px;font-size:34px;font-weight:700;letter-spacing:-.03em}
    .brand img{width:42px;height:42px}.title{position:absolute;left:48px;top:177px;margin:0;font-size:76px;line-height:1.02;font-weight:600;letter-spacing:-.035em}
    .copy{position:absolute;left:51px;top:361px;width:445px;font-size:24px;line-height:1.45}
    .workspace{position:absolute;left:544px;top:153px;width:618px;border:1px solid #d3dce9;border-radius:13px;box-shadow:0 19px 40px #0d33913d}
    .footer{position:absolute;left:51px;bottom:38px;font-size:17px;line-height:1.5}.caption{position:absolute;left:546px;top:599px;font-size:12px}
  </style></head><body>
    <div class="brand"><img src="data:image/svg+xml;base64,${logo}" alt="">folio</div>
    <h1 class="title">Your PDFs.<br>Your rules.</h1>
    <p class="copy">An independent, open-source<br>alternative to Adobe Acrobat.</p>
    <img class="workspace" src="data:image/webp;base64,${workspace}" alt="Actual Folio source-build desktop workspace with an original sample PDF.">
    <div class="footer">No subscription. No account.<br>vertex-shift.github.io/folio</div>
    <p class="caption">Current source build. Stable release downloads on the site.</p>
  </body></html>`);
  await page.evaluate(() => document.fonts.ready);
  await page.locator(".workspace").evaluate((image) => image.decode());
  const data = await page.screenshot({ path: resolve(directory, "social.png") });
  const filename = resolve(directory, "provenance.json");
  const provenance = JSON.parse(await readFile(filename, "utf8"));
  provenance.assets = provenance.assets.filter((item) => item.file !== "social.png");
  provenance.assets.push({
    file: "social.png", width: 1200, height: 630,
    sha256: createHash("sha256").update(data).digest("hex"),
    origin: "Original Folio social card rendered by scripts/create-site-social.mjs with the locally licensed Commissioner font and the verified actual workspace-read.webp screenshot. No invented product UI, ratings or Adobe assets.",
  });
  await writeFile(filename, JSON.stringify(provenance, null, 2) + "\n");
  console.log("Created the original 1200x630 Folio social-preview card.");
} finally { await browser.close(); }
