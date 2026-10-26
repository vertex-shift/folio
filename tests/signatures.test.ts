import test from "node:test";
import assert from "node:assert/strict";
import {
  certificateValidityLabel, chainLabel, signatureSummary, signedCopyName, signingField, validateAnchorFile, validateIdentityFile,
} from "../src/features/signatures/types.ts";
import type { SignatureInfo, SignatureReport } from "../src/features/signatures/types.ts";
import { createSignatureTrustState } from "../src/features/signatures/trustState.ts";

function signature(overrides: Partial<SignatureInfo> = {}): SignatureInfo {
  return {
    id: "1:0", integrity: "intact", coverage: "whole_document", message: "",
    digest: "SHA-256", algorithm: "RSA PKCS#1 v1.5", signedBytes: 100, currentBytes: 100,
    certificate: {
      subject: "CN=Test", issuer: "CN=Test", serial: "01", sha256Fingerprint: "00",
      notBefore: "", notAfter: "", validity: "valid_now", selfSigned: true, identityTrust: "not_checked",
    },
    chain: { status: "not_checked", message: "", anchorFingerprint: null, chainFingerprints: [] },
    claimedSigningTime: null, revocation: "not_checked", trustedTime: "not_checked",
    ...overrides,
  };
}

test("signature integrity labels never promote a self-signed identity to trusted", () => {
  assert.deepEqual(signatureSummary(signature()), { title: "Signed bytes intact", tone: "intact" });
  assert.ok(!signatureSummary(signature()).title.toLowerCase().includes("trusted"));
  assert.deepEqual(signatureSummary(signature({ integrity: "invalid" })), { title: "Signature check failed", tone: "invalid" });
  assert.deepEqual(signatureSummary(signature({ integrity: "unsupported" })), { title: "Verification unsupported", tone: "warning" });
});

test("later or unknown coverage never receives an intact-whole-file label", () => {
  assert.deepEqual(signatureSummary(signature({ coverage: "later_changes" })), { title: "Later bytes are not signed", tone: "warning" });
  assert.deepEqual(signatureSummary(signature({ coverage: "unknown" })), { title: "Signature coverage unknown", tone: "warning" });
});

test("certificate validity is described as a current date check, not trust or historical signing validity", () => {
  assert.equal(certificateValidityLabel("valid_now"), "Within certificate dates now");
  assert.equal(certificateValidityLabel("expired"), "Certificate expired");
  assert.equal(certificateValidityLabel("not_yet_valid"), "Certificate not yet valid");
});

test("identity selection enforces PKCS#12 names and exact nonempty 4 MiB bounds", () => {
  for (const name of ["identity.p12", "Identity.PFX"]) {
    validateIdentityFile({ name, size: 1 });
    validateIdentityFile({ name, size: 4 * 1024 * 1024 });
  }
  for (const input of [
    { name: "private-key.pem", size: 42 },
    { name: "identity.pfx", size: 0 },
    { name: "identity.p12", size: 4 * 1024 * 1024 + 1 },
  ]) {
    assert.throws(() => validateIdentityFile(input));
  }
});

test("signed downloads name a separate copy", () => {
  assert.equal(signedCopyName("Reviewed.PDF"), "Reviewed-signed.pdf");
  assert.equal(signedCopyName("untitled"), "untitled-signed.pdf");
});

test("custom CA selection requires a public certificate filename and exact nonempty 64 KiB bounds", () => {
  for (const name of ["Root.PEM", "root.der", "root.cer", "root.CRT"]) {
    validateAnchorFile({ name, size: 1 });
    validateAnchorFile({ name, size: 64 * 1024 });
  }
  for (const file of [
    { name: "private.key", size: 1 }, { name: "identity.pfx", size: 1 },
    { name: "root.pem", size: 0 }, { name: "root.pem", size: 64 * 1024 + 1 },
    { name: "root.pem", size: Number.NaN }, { name: "root.pem", size: 1.5 },
  ]) assert.throws(() => validateAnchorFile(file));
});

test("custom anchoring labels do not claim Adobe, OS, revocation, time, or mathematical trust", () => {
  const chain = signature().chain;
  assert.equal(chainLabel(chain), "Certificate-chain trust not checked");
  assert.equal(chainLabel({ ...chain, status: "untrusted" }), "No valid chain to your custom CA store");
  assert.equal(chainLabel({ ...chain, status: "anchored_custom_store" }), "Chain anchored in your custom CA store");
  const anchored = signature({ chain: { ...chain, status: "anchored_custom_store" }, integrity: "invalid" });
  assert.equal(signatureSummary(anchored).tone, "invalid");
  assert.equal(signatureSummary(signature({ ...anchored, integrity: "intact", coverage: "later_changes" })).tone, "warning");
});

test("selection binds canonical logical field IDs, not names, widget IDs, or a stale slot", () => {
  const report: SignatureReport = {
    documentId: 1, revision: 4, signatures: [], warnings: [], signingBlockedReason: null,
    trustStore: { revision: 0, anchors: [], persistent: false },
    emptyFields: [{ id: "50:0", name: "Approval", blockedReason: null,
      widgets: [{ id: "51:0", page: 1, rect: [40, 130, 300, 200], visible: true }] }],
  };
  assert.equal(signingField(report, null), null);
  assert.equal(signingField(report, "50:0"), report.emptyFields[0]);
  for (const id of ["51:0", "Approval", "", "050:0", "50:1", "50:0/../../identity"]) {
    assert.throws(() => signingField(report, id));
  }
  assert.throws(() => signingField({ ...report, signingBlockedReason: "Unsigned changes" }, null), /Unsigned changes/);
  assert.throws(() => signingField({ ...report, emptyFields: [{ ...report.emptyFields[0]!, blockedReason: "Locked" }] }, "50:0"), /Locked/);
});

test("anchor changes invalidate all panels including inactive tabs and reject late stale reports", () => {
  const state = createSignatureTrustState();
  const panels = [{ active: true, cached: "anchored" }, { active: false, cached: "anchored" }];
  const unsubscribe = panels.map((panel) => state.subscribe(() => { panel.cached = ""; }));
  assert.equal(state.accept(0), true);
  assert.equal(state.accept(1), true);
  assert.deepEqual(panels.map((panel) => panel.cached), ["", ""]);
  for (const panel of panels) panel.cached = "anchored";
  assert.equal(state.accept(2), true); // Removal changes the store even when a PDF revision is unchanged.
  assert.deepEqual(panels.map((panel) => panel.cached), ["", ""]);
  assert.equal(state.accept(1), false); // An inspection started before removal cannot restore trust.
  assert.equal(state.accept(2), true);
  for (const value of [-1, 1.5, Number.NaN, Number.POSITIVE_INFINITY]) assert.equal(state.accept(value), false);
  unsubscribe[0]!();
  panels[0]!.cached = "unmounted";
  state.accept(3);
  assert.equal(panels[0]!.cached, "unmounted");
});
