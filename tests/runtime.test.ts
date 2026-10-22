import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { FOLIO_IDENTIFIER, resolveRuntime } from "../src/lib/runtime.ts";

test("native runtime identity matches the desktop bundle configuration", () => {
  const config = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  assert.equal(FOLIO_IDENTIFIER, config.identifier);
});

test("a normal browser never probes native APIs", async () => {
  let called = false;
  const result = await resolveRuntime({
    hasNativeBridge: false,
    identify: async () => { called = true; return FOLIO_IDENTIFIER; },
    developmentPreview: true,
    reportPreview: () => assert.fail("No host diagnostic was needed"),
  });
  assert.deepEqual(result, { native: false, issue: null });
  assert.equal(called, false);
});

test("only Folio's own bridge enables desktop operations", async () => {
  for (const [identifier, expected] of [[FOLIO_IDENTIFIER, true], ["another.desktop.host", false]] as const) {
    const result = await resolveRuntime({
      hasNativeBridge: true, identify: async () => identifier,
      developmentPreview: true, reportPreview: () => assert.fail("The identity was readable"),
    });
    assert.deepEqual(result, { native: expected, issue: null });
  }
});

test("an embedded host with restricted native APIs uses the explicit development preview", async () => {
  const diagnostics: string[] = [];
  const result = await resolveRuntime({
    hasNativeBridge: true,
    identify: async () => { throw new Error("Host API not allowed by ACL"); },
    developmentPreview: true,
    reportPreview: (message) => diagnostics.push(message),
  });
  assert.deepEqual(result, { native: false, issue: null });
  assert.equal(diagnostics.length, 1);
  assert.match(diagnostics[0], /without host native APIs.*not allowed/);
});

test("production identity failures remain explicit instead of enabling a foreign bridge", async () => {
  const result = await resolveRuntime({
    hasNativeBridge: true,
    identify: async () => { throw new Error("Identity unavailable"); },
    developmentPreview: false,
    reportPreview: () => assert.fail("Production must not silently downgrade"),
  });
  assert.equal(result.native, false);
  assert.match(result.issue ?? "", /could not identify.*Restart.*Identity unavailable/);
});

test("production browser and foreign-host contexts report an unsupported runtime", async () => {
  for (const hasNativeBridge of [false, true]) {
    const result = await resolveRuntime({
      hasNativeBridge, identify: async () => "another.desktop.host",
      developmentPreview: false,
      reportPreview: () => assert.fail("Production must not enable preview mode"),
    });
    assert.equal(result.native, false);
    assert.match(result.issue ?? "", /Open the Folio desktop application/);
  }
});
