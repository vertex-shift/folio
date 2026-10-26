import test from "node:test";
import assert from "node:assert/strict";
import {
  certificateValidityLabel, signatureSummary, signedCopyName, validateIdentityFile,
} from "../src/features/signatures/types.ts";
import type { SignatureInfo } from "../src/features/signatures/types.ts";

function signature(overrides: Partial<SignatureInfo> = {}): SignatureInfo {
  return {
    id: "1:0", integrity: "intact", coverage: "whole_document", message: "",
    digest: "SHA-256", algorithm: "RSA PKCS#1 v1.5", signedBytes: 100, currentBytes: 100,
    certificate: {
      subject: "CN=Test", issuer: "CN=Test", serial: "01", sha256Fingerprint: "00",
      notBefore: "", notAfter: "", validity: "valid_now", selfSigned: true, identityTrust: "not_checked",
    },
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
