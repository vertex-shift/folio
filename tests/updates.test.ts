import test from "node:test";
import assert from "node:assert/strict";
import { canCheckForUpdates, downloadProgress, updateSummary } from "../src/lib/updates.ts";
import type { UpdateStatus } from "../src/lib/updates.ts";

const initial: UpdateStatus = { phase: "idle", version: null, received: 0, total: null, message: null };

test("update messages distinguish download, verified readiness, installation and failures", () => {
  assert.match(updateSummary({ ...initial, phase: "downloading", version: "0.2.0" }), /Downloading Folio 0.2.0/);
  assert.match(updateSummary({ ...initial, phase: "ready", version: "0.2.0" }), /install when you quit/);
  assert.match(updateSummary({ ...initial, phase: "installing" }), /close when it is finished/);
  assert.equal(updateSummary({ ...initial, phase: "error", message: "Invalid signature" }), "Invalid signature");
  assert.equal(updateSummary({ ...initial, phase: "unavailable", message: "Use your package manager" }), "Use your package manager");
});

test("only idle and failed update checks can be retried", () => {
  for (const phase of ["unavailable", "checking", "downloading", "ready", "installing"] as const) {
    assert.equal(canCheckForUpdates({ ...initial, phase }), false);
  }
  assert.equal(canCheckForUpdates(initial), true);
  assert.equal(canCheckForUpdates({ ...initial, phase: "error" }), true);
});

test("unknown and invalid update sizes stay indeterminate rather than inventing progress", () => {
  assert.equal(downloadProgress(initial), undefined);
  assert.equal(downloadProgress({ ...initial, total: 0 }), undefined);
  assert.equal(downloadProgress({ ...initial, total: NaN }), undefined);
  assert.equal(downloadProgress({ ...initial, total: 20, received: -1 }), undefined);
  assert.equal(downloadProgress({ ...initial, total: 20, received: 5 }), 0.25);
  assert.equal(downloadProgress({ ...initial, total: 20, received: 30 }), 1);
});
