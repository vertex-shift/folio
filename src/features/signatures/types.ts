export interface SigningIdentityFile {
  name: string;
  path: string;
}

export type SigningIdentity = { kind: "native"; file: SigningIdentityFile }
  | { kind: "browser"; file: File };

export interface SignatureCertificate {
  subject: string;
  issuer: string;
  serial: string;
  sha256Fingerprint: string;
  notBefore: string;
  notAfter: string;
  validity: "valid_now" | "expired" | "not_yet_valid";
  selfSigned: boolean;
  identityTrust: "not_checked";
}

export interface SignatureInfo {
  id: string;
  integrity: "intact" | "invalid" | "unsupported";
  coverage: "whole_document" | "later_changes" | "unknown";
  message: string;
  digest: string | null;
  algorithm: string | null;
  signedBytes: number | null;
  currentBytes: number;
  certificate: SignatureCertificate | null;
}

export interface SignatureReport {
  documentId: number;
  revision: number;
  signatures: SignatureInfo[];
  signingBlockedReason: string | null;
  warnings: string[];
}

export function signatureSummary(signature: SignatureInfo): {
  title: string; tone: "intact" | "warning" | "invalid";
} {
  if (signature.integrity === "unsupported") return { title: "Verification unsupported", tone: "warning" };
  if (signature.integrity === "invalid") return { title: "Signature check failed", tone: "invalid" };
  if (signature.coverage === "later_changes") return { title: "Later bytes are not signed", tone: "warning" };
  if (signature.coverage === "unknown") return { title: "Signature coverage unknown", tone: "warning" };
  return { title: "Signed bytes intact", tone: "intact" };
}

export function certificateValidityLabel(validity: SignatureCertificate["validity"]): string {
  return {
    valid_now: "Within certificate dates now",
    expired: "Certificate expired",
    not_yet_valid: "Certificate not yet valid",
  }[validity];
}

export function validateIdentityFile(file: Pick<File, "name" | "size">): void {
  if (!/\.(p12|pfx)$/i.test(file.name)) throw new Error("Choose a .p12 or .pfx PKCS#12 signing identity.");
  if (file.size <= 0 || file.size > 4 * 1024 * 1024) throw new Error("Choose a nonempty signing identity of at most 4 MiB.");
}

export function signedCopyName(name: string): string {
  return name.replace(/\.pdf$/i, "") + "-signed.pdf";
}
