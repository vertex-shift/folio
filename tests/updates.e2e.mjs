import assert from "node:assert/strict";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const output = process.env.FOLIO_UPDATES_EVIDENCE
  ? resolve(process.env.FOLIO_UPDATES_EVIDENCE) : await mkdtemp(join(tmpdir(), "folio-update-ui-"));
await mkdir(output, { recursive: true });
const evidence = { checks: [], errors: [], externalRequests: [] };
let server, browser, page;
try {
  server = await createServer({ root, logLevel: "error", server: { host: "127.0.0.1", port: 0 },
    plugins: [{
      name: "update-ui-fixture",
      configureServer(server) {
        server.middlewares.use("/__updates-test", async (_request, response) => {
          response.setHeader("Content-Type", "text/html");
          response.end(await server.transformIndexHtml("/__updates-test", `<!doctype html><html lang="en"><head><title>Updater UI test</title></head><body>
            <script type="module">
              import React from "react";
              import {createRoot} from "react-dom/client";
              import UpdateControls from "/src/components/UpdateControls.tsx";
              import "/src/themes.css"; import "/src/styles.css";
              function Harness() {
                const [open, setOpen] = React.useState(false);
                return React.createElement("header", {className:"titlebar"},
                  React.createElement("div", {className:"titlebar-space"}),
                  React.createElement(UpdateControls, {open, onOpenChange:setOpen}));
              }
              createRoot(document.body).render(React.createElement(Harness));
            </script></body></html>`));
        });
      },
    }] });
  await server.listen();
  const origin = server.resolvedUrls.local[0].replace(/\/$/, "");
  browser = await chromium.launch({ headless: true,
    ...(process.env.FOLIO_UPDATES_BROWSER ? { executablePath: process.env.FOLIO_UPDATES_BROWSER }
      : process.platform === "win32" ? { channel: "msedge" } : {}) });
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, reducedMotion: "reduce" });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    evidence.externalRequests.push(route.request().url()); return route.abort();
  });
  await context.addInitScript(() => {
    window.isTauri = true;
    const callbacks = new Map(), listeners = new Map();
    let next = 0;
    window.updateState = { phase: "idle", version: null, received: 0, total: null, message: null };
    window.updateChecks = 0;
    window.publishUpdate = (patch) => {
      window.updateState = { ...window.updateState, ...patch };
      for (const [id, handler] of listeners) callbacks.get(handler)({ event: "folio-update", id, payload: window.updateState });
    };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (id) => listeners.delete(id) };
    window.__TAURI_INTERNALS__ = {
      transformCallback(callback) { const id = ++next; callbacks.set(id, callback); return id; },
      async invoke(command, args) {
        if (command === "plugin:app|identifier") return "app.folio.desktop";
        if (command === "update_status") return window.updateState;
        if (command === "check_for_updates") { window.updateChecks++; window.publishUpdate({ phase: "checking" }); return; }
        if (command === "plugin:event|listen") { const id = ++next; listeners.set(id, args.handler); return id; }
        if (command === "plugin:event|unlisten") { listeners.delete(args.eventId); return; }
        throw new Error("Unexpected native command: " + command);
      },
    };
  });
  page = await context.newPage();
  page.setDefaultTimeout(15000);
  page.on("pageerror", (error) => evidence.errors.push(error.message));
  await page.goto(`${origin}/__updates-test`);
  const trigger = page.getByRole("button", { name: "App updates", exact: true });
  await trigger.click();
  const panel = page.getByRole("region", { name: "App updates" });
  await panel.waitFor();
  await panel.getByRole("button", { name: "Check for updates", exact: true }).click();
  assert.equal(await page.evaluate(() => window.updateChecks), 1);
  await page.getByText("Checking for updates...", { exact: true }).waitFor();
  assert.equal(await panel.getByRole("button", { name: "Check for updates", exact: true }).isDisabled(), true);
  evidence.checks.push("manual check starts once and pending state disables duplicate checks");
  await page.evaluate(() => window.publishUpdate({ phase: "downloading", version: "0.2.0", received: 5, total: 20 }));
  await page.getByRole("progressbar", { name: "Update download" }).waitFor();
  assert.equal(await page.getByRole("progressbar").getAttribute("value"), "0.25");
  await page.evaluate(() => window.publishUpdate({ phase: "ready" }));
  await page.getByText("Folio 0.2.0 is ready. It will install when you quit.", { exact: true }).waitFor();
  assert.equal(await panel.getByRole("button", { name: "Check for updates", exact: true }).isDisabled(), true);
  evidence.checks.push("verified-ready state explains update-on-quit without offering immediate installation");
  for (const theme of ["light", "dark", "sepia", "contrast"]) {
    await page.evaluate((theme) => { document.documentElement.dataset.theme = theme; }, theme);
    await page.screenshot({ path: join(output, `ready-${theme}.png`) });
  }
  await page.setViewportSize({ width: 820, height: 620 });
  assert.equal(await panel.evaluate((element) => element.getBoundingClientRect().right <= innerWidth), true);
  await page.keyboard.press("Escape");
  assert.equal(await panel.count(), 0);
  assert.equal(await trigger.evaluate((element) => document.activeElement === element), true);
  await trigger.click();
  await page.evaluate(() => window.publishUpdate({ phase: "error", message: "Invalid update signature. Your app is unchanged." }));
  await page.getByText("Invalid update signature. Your app is unchanged.", { exact: true }).waitFor();
  assert.equal(await panel.getByRole("button", { name: "Check for updates", exact: true }).isEnabled(), true);
  await panel.getByRole("button", { name: "Check for updates", exact: true }).click();
  assert.equal(await page.evaluate(() => window.updateChecks), 2);
  await page.evaluate(() => window.publishUpdate({ phase: "unavailable", message: "Use your package manager." }));
  await page.getByText("Use your package manager.", { exact: true }).waitFor();
  assert.equal(await panel.getByRole("button", { name: "Check for updates", exact: true }).isDisabled(), true);
  evidence.checks.push("failure retries, unsupported installations, narrow layout and Escape focus restoration");
  assert.deepEqual(evidence.errors, []);
  assert.deepEqual(evidence.externalRequests, []);
  console.log("Updater UI browser checks passed; this fixture mocks native transport, not installation.");
} catch (error) {
  evidence.failure = error.stack ?? String(error);
  if (page) await page.screenshot({ path: join(output, "failure.png") }).catch(() => {});
  throw error;
} finally {
  await writeFile(join(output, "evidence.json"), JSON.stringify(evidence, null, 2));
  if (browser) await browser.close();
  if (server) await server.close();
}
