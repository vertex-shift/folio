import assert from "node:assert/strict";
import { readFile, readdir, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { runInNewContext } from "node:vm";
import { downloads, pageUrl, platformDownload } from "../site/app.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));
const site = path.join(root, "site");
const html = await readFile(path.join(site, "index.html"), "utf8");
const css = await readFile(path.join(site, "styles.css"), "utf8");
const appearanceScript = await readFile(path.join(site, "appearance.js"), "utf8");

function meta(attribute: "name" | "property", value: string) {
  const matches = [...html.matchAll(new RegExp(`<meta ${attribute}="${value}" content="([^"]+)"`, "g"))];
  assert.equal(matches.length, 1, `Expected one ${value} declaration.`);
  return matches[0][1].replaceAll("&amp;", "&");
}

interface AppearanceState {
  preference: string;
  message: string | null;
}
interface AppearanceController {
  snapshot(): AppearanceState;
  choose(value: string): AppearanceState;
}
function appearance(storage: { getItem(key: string): string | null; setItem(key: string, value: string): void }) {
  const window: { folioSiteAppearance?: AppearanceController } = {};
  const dataset: Record<string, string> = {};
  const document = { documentElement: { dataset } };
  runInNewContext(appearanceScript, { window, document, localStorage: storage });
  assert.ok(window.folioSiteAppearance);
  return { controller: window.folioSiteAppearance, dataset: document.documentElement.dataset };
}

test("website appearance defaults to System and persists only its own preference before CSS", () => {
  const values = new Map([["folio.appearance", "sepia"]]);
  const requested: string[] = [];
  const storage = {
    getItem(key: string) { requested.push(key); return values.get(key) ?? null; },
    setItem(key: string, value: string) { requested.push(key); values.set(key, value); },
  };
  const initial = appearance(storage);
  assert.equal(initial.controller.snapshot().preference, "system");
  assert.equal(initial.controller.snapshot().message, null);
  assert.equal(initial.dataset.appearance, "system");
  for (const value of ["dark", "light", "system"]) {
    assert.equal(initial.controller.choose(value).preference, value);
    assert.equal(initial.dataset.appearance, value);
    assert.equal(appearance(storage).controller.snapshot().preference, value);
  }
  assert.ok(requested.every((key) => key === "folio.site.appearance"));
  assert.equal(values.get("folio.appearance"), "sepia");
  assert.throws(() => initial.controller.choose("sepia"), /Choose System, Light or Dark/);
  assert.equal(initial.controller.snapshot().preference, "system");
  assert.ok(html.indexOf('src="./appearance.js"') < html.indexOf('rel="stylesheet"'));
  assert.match(css, /prefers-color-scheme: dark/);
  assert.match(html, /for="website-appearance">Website appearance/);
});

test("blocked or corrupt appearance storage stays visible and never prevents visit-only choices", () => {
  const bad = appearance({ getItem: () => "unrecognized", setItem: () => {} });
  assert.equal(bad.controller.snapshot().preference, "system");
  assert.match(bad.controller.snapshot().message ?? "", /not recognized/);
  const blocked = appearance({
    getItem() { throw new Error("Blocked"); },
    setItem() { throw new Error("Blocked"); },
  });
  assert.equal(blocked.controller.snapshot().preference, "system");
  assert.match(blocked.controller.snapshot().message ?? "", /storage is unavailable/);
  assert.equal(blocked.controller.choose("dark").preference, "dark");
  assert.equal(blocked.dataset.appearance, "dark");
  assert.match(blocked.controller.snapshot().message ?? "", /could not be saved/);
});

test("the introduction names the actual independent PDF product and its release/source boundary", () => {
  assert.match(html, /An independent, open-source alternative to Adobe Acrobat/);
  assert.match(html, /local-first desktop PDF workspace/);
  assert.equal((html.match(/<h1(?:\s|>)/g) ?? []).length, 1);
  assert.match(html, /This feature wave is in the v0\.1\.2 installers/);
  assert.doesNotMatch(html, /fewer features|not in the v0\.1\.2 installers|v0\.1\.1/);
  assert.match(html, /not a complete Acrobat replacement/);
  assert.match(html, /not notarized/);
  assert.match(html, /Windows and Linux packages are unsigned/);
  assert.match(html, /Not affiliated with Adobe/);
  assert.doesNotMatch(html, /aggregateRating|reviewCount|Trusted by \d|Join \d|limited.time offer/i);
});

test("all local links, media, tabs and anchors resolve under the project Pages subpath", async () => {
  const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map((match) => match[1]);
  assert.equal(new Set(ids).size, ids.length);
  for (const [, value] of html.matchAll(/\b(?:href|src|srcset)="([^"]+)"/g)) {
    if (/^https:/.test(value)) continue;
    if (value.startsWith("#")) { assert.ok(ids.includes(value.slice(1)), value); continue; }
    assert.ok(value.startsWith("./"), `A local asset must preserve /folio/: ${value}`);
    if (value === "./") continue;
    const resolved = path.resolve(site, value);
    assert.ok(resolved.startsWith(site + path.sep));
    assert.ok((await stat(resolved)).isFile(), value);
    assert.equal(new URL(value, pageUrl).origin, new URL(pageUrl).origin);
    assert.ok(new URL(value, pageUrl).pathname.startsWith("/folio/"));
  }
  for (const view of ["read", "comment", "organize"]) {
    for (const suffix of ["", "-detail"]) assert.ok((await stat(path.join(site, `assets/workspace-${view}${suffix}.webp`))).size > 0);
    assert.ok(ids.includes(`preview-${view}`));
  }
  assert.match(html, /role="tablist"/);
  assert.match(html, /role="tabpanel"/);
  assert.match(html, /aria-live="polite"/);
  assert.match(css, /prefers-reduced-motion/);
  assert.match(css, /focus-visible/);
});

test("search and social previews describe the actual desktop PDF editor consistently", () => {
  const titles = [...html.matchAll(/<title>([^<]+)<\/title>/g)];
  assert.equal(titles.length, 1);
  const title = titles[0][1].replaceAll("&amp;", "&");
  assert.match(title, /^Folio \| Free Open-Source PDF Editor & Acrobat Alternative$/);
  assert.ok(title.length <= 65);
  assert.equal(meta("property", "og:title"), title);
  assert.equal(meta("name", "twitter:title"), title);
  const description = meta("name", "description");
  assert.ok(description.length >= 120 && description.length <= 160);
  assert.match(description, /macOS, Windows and Linux/);
  assert.match(description, /edit supported text/);
  assert.equal(meta("property", "og:description"), description);
  assert.equal(meta("name", "twitter:description"), description);
  assert.equal(meta("name", "robots"), "index,follow,max-image-preview:large");
  assert.equal(meta("property", "og:url"), pageUrl);
  assert.equal(meta("property", "og:type"), "website");
  assert.equal(meta("property", "og:locale"), "en_US");
  assert.equal(meta("property", "og:image:type"), "image/png");
  assert.equal(meta("name", "twitter:card"), "summary_large_image");
  assert.equal(meta("property", "og:image"), meta("name", "twitter:image"));
  assert.equal(meta("property", "og:image:alt"), meta("name", "twitter:image:alt"));
  assert.doesNotMatch(html, /name="keywords"|noindex|FAQPage|SearchAction/);
  assert.ok(html.includes(`<link rel="canonical" href="${pageUrl}">`));
  assert.equal((html.match(/rel="canonical"/g) ?? []).length, 1);
});

test("structured data separates the real release, source repository and original page image", async () => {
  const scripts = [...html.matchAll(/<script type="application\/ld\+json">([\s\S]+?)<\/script>/g)];
  assert.equal(scripts.length, 1);
  const structured: { "@context": string; "@graph": Record<string, unknown>[] } = JSON.parse(scripts[0][1]);
  assert.equal(structured["@context"], "https://schema.org");
  const graph = structured["@graph"];
  assert.equal(graph.length, 4);
  assert.equal(new Set(graph.map((item) => item["@id"])).size, graph.length);
  const webpage = graph.find((item) => item["@type"] === "WebPage");
  const application = graph.find((item) => item["@type"] === "SoftwareApplication");
  const source = graph.find((item) => item["@type"] === "SoftwareSourceCode");
  const image = graph.find((item) => item["@type"] === "ImageObject");
  assert.ok(webpage && application && source && image);
  assert.equal(webpage["@id"], `${pageUrl}#webpage`);
  assert.equal(webpage.url, pageUrl);
  assert.equal(webpage.name, meta("property", "og:title"));
  assert.equal(webpage.description, meta("name", "description"));
  assert.equal(webpage.inLanguage, "en");
  assert.deepEqual(webpage.mainEntity, { "@id": application["@id"] });
  assert.deepEqual(webpage.primaryImageOfPage, { "@id": image["@id"] });
  assert.equal(application["@id"], `${pageUrl}#application`);
  assert.equal(application.name, "Folio");
  assert.equal(application.softwareVersion, "0.1.2");
  assert.deepEqual(application.operatingSystem, ["macOS 13+ (Apple Silicon)", "Windows (x64)", "Linux (x64)"]);
  assert.equal(application.isAccessibleForFree, true);
  assert.equal(application.license, "https://github.com/vertex-shift/folio/blob/main/LICENSE");
  assert.deepEqual(application.offers, { "@type": "Offer", url: application.downloadUrl, price: "0", priceCurrency: "USD" });
  assert.equal(application.downloadUrl, "https://github.com/vertex-shift/folio/releases/tag/v0.1.2");
  assert.ok(html.includes(`href="${application.downloadUrl}"`));
  assert.equal(application.codeRepository, undefined);
  assert.equal(source["@id"], "https://github.com/vertex-shift/folio");
  assert.equal(source.codeRepository, source["@id"]);
  assert.equal(source.license, application.license);
  assert.equal(source.softwareVersion, undefined);
  assert.deepEqual(source.programmingLanguage, ["Rust", "TypeScript"]);
  assert.deepEqual(source.targetProduct, { "@id": application["@id"] });
  assert.equal(image["@id"], `${pageUrl}#social-image`);
  assert.equal(image.contentUrl, meta("property", "og:image"));
  assert.equal(image.caption, meta("property", "og:image:alt"));
  const png = await readFile(path.join(site, "assets/social.png"));
  assert.equal(png.subarray(1, 4).toString(), "PNG");
  assert.equal(image.width, png.readUInt32BE(16));
  assert.equal(image.height, png.readUInt32BE(20));
  assert.equal(meta("property", "og:image:width"), String(image.width));
  assert.equal(meta("property", "og:image:height"), String(image.height));
  for (const item of graph) {
    assert.equal(item.aggregateRating, undefined);
    assert.equal(item.review, undefined);
  }
});

test("the sitemap lists the canonical page and only genuine local workspace images", async () => {
  const sitemap = await readFile(path.join(site, "sitemap.xml"), "utf8");
  assert.match(sitemap, /xmlns="http:\/\/www\.sitemaps\.org\/schemas\/sitemap\/0\.9"/);
  assert.match(sitemap, /xmlns:image="http:\/\/www\.google\.com\/schemas\/sitemap-image\/1\.1"/);
  assert.deepEqual([...sitemap.matchAll(/<loc>([^<]+)<\/loc>/g)].map((match) => match[1]), [pageUrl]);
  const lastmod = sitemap.match(/<lastmod>(\d{4}-\d{2}-\d{2})<\/lastmod>/)?.[1];
  assert.ok(lastmod);
  const date = new Date(lastmod);
  assert.equal(date.toISOString().slice(0, 10), lastmod);
  assert.ok(date.getTime() <= Date.now(), "lastmod must describe a real past page change, not a future date.");
  assert.doesNotMatch(sitemap, /<priority>|<changefreq>|<image:caption>|<image:title>/);
  const images = [...sitemap.matchAll(/<image:loc>([^<]+)<\/image:loc>/g)].map((match) => match[1]);
  assert.deepEqual(images, ["read", "comment", "organize"].map((view) => `${pageUrl}assets/workspace-${view}.webp`));
  for (const image of images) {
    const local = path.join(site, new URL(image).pathname.slice(new URL(pageUrl).pathname.length));
    assert.ok((await stat(local)).isFile());
  }
  assert.equal((await readdir(site)).includes("robots.txt"), false, "A /folio/robots.txt file would not control the host.");
});

test("real platform download URLs match the visible release and never guess unsupported architectures", () => {
  assert.equal(platformDownload("MacIntel"), downloads.macos);
  assert.equal(platformDownload("Win32"), downloads.windows);
  assert.equal(platformDownload("Linux x86_64"), downloads.linux);
  assert.equal(platformDownload("Android"), null);
  assert.equal(platformDownload("Linux armv8l"), null);
  assert.equal(platformDownload("Linux aarch64"), null);
  assert.equal(platformDownload("iPhone"), null);
  for (const value of Object.values(downloads)) {
    assert.ok(html.includes(value.href));
    assert.match(value.href, /^https:\/\/github\.com\/vertex-shift\/folio\/releases\/download\/v0\.1\.2\//);
    assert.match(value.detail, /v0\.1\.2/);
  }
  assert.match(downloads.macos.detail, /Apple Silicon/);
  assert.match(html, /Intel macOS and ARM Windows\/Linux installers are not provided/);
});

test("the introduction stays lightweight, locally hosted and licensed", async () => {
  assert.doesNotMatch(html + css, /https:\/\/(?:fonts\.googleapis|fonts\.gstatic)|google-analytics|googletagmanager|connect\.facebook|<iframe/i);
  assert.doesNotMatch(html, /<input[^>]+type="(?:file|email)"/);
  const script = await readFile(path.join(site, "app.mjs"), "utf8");
  assert.doesNotMatch(script, /\bfetch\s*\(|\bXMLHttpRequest\b|\blocalStorage\b|\bdocument\.cookie\b/);
  assert.doesNotMatch(appearanceScript, /\bfetch\s*\(|\bXMLHttpRequest\b|\bdocument\.cookie\b/);
  assert.ok(Buffer.byteLength(html) + Buffer.byteLength(css) + Buffer.byteLength(script) + Buffer.byteLength(appearanceScript) < 50_000);
  assert.ok((await stat(path.join(site, "assets/fonts/commissioner-latin.woff2"))).size < 50_000);
  assert.match(await readFile(path.join(site, "assets/fonts/OFL.txt"), "utf8"), /SIL OPEN FONT LICENSE Version 1\.1/);
  assert.equal((await readFile(path.join(site, "assets/try-folio.pdf"))).subarray(0, 5).toString(), "%PDF-");
  const provenance = JSON.parse(await readFile(path.join(site, "assets/provenance.json"), "utf8"));
  assert.equal(provenance.assets.filter((item: { file: string }) => item.file.startsWith("workspace-")).length, 6);
  assert.ok(provenance.assets.every((item: { origin: string }) => item.origin.trim().length > 20));
  async function size(directory: string): Promise<number> {
    let total = 0;
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const name = path.join(directory, entry.name);
      total += entry.isDirectory() ? await size(name) : (await stat(name)).size;
    }
    return total;
  }
  assert.ok(await size(site) < 1_800_000, "The whole public site, including social imagery, must stay below 1.8 MiB.");
});

test("Pages publishes only the site on protected main and uses scoped OIDC permissions", async () => {
  const workflow = await readFile(path.join(root, ".github/workflows/pages.yml"), "utf8");
  assert.match(workflow, /branches: \[main\]/);
  assert.match(workflow, /pull_request:/);
  assert.match(workflow, /node --test tests\/landingPage\.test\.ts/);
  assert.match(workflow, /path: site/);
  assert.match(workflow, /if: github\.ref == 'refs\/heads\/main' && github\.repository == 'vertex-shift\/folio'/);
  assert.match(workflow, /pages: write/);
  assert.match(workflow, /id-token: write/);
  assert.doesNotMatch(workflow, /pull_request_target|contents: write|path: ['"]?\.['"]?\s*\n/);
  for (const action of ["checkout", "setup-node", "upload-pages-artifact", "configure-pages", "deploy-pages"]) {
    assert.match(workflow, new RegExp(`uses: actions/${action}@[0-9a-f]{40}`));
  }
});
