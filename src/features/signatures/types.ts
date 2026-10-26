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
  identityTrust: "not_checked" | "untrusted" | "anchored_custom_store";
}

export interface SignatureChain {
  status: SignatureCertificate["identityTrust"];
  message: string;
  anchorFingerprint: string | null;
  chainFingerprints: string[];
}

export interface SignatureTrustStore {
  revision: number;
  anchors: SignatureCertificate[];
  persistent: false;
}

export interface SignatureField {
  id: string;
  name: string;
  widgets: { id: string; page: number; rect: [number, number, number, number]; visible: boolean }[];
  blockedReason: string | null;
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
  chain: SignatureChain;
  claimedSigningTime: string | null;
  revocation: "not_checked";
  trustedTime: "not_checked";
}

export interface SignatureReport {
  documentId: number;
  revision: number;
  signatures: SignatureInfo[];
  signingBlockedReason: string | null;
  warnings: string[];
  emptyFields: SignatureField[];
  trustStore: SignatureTrustStore;
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
  if (!Number.isSafeInteger(file.size) || file.size <= 0 || file.size > 4 * 1024 * 1024) throw new Error("Choose a nonempty signing identity of at most 4 MiB.");
}

export function validateAnchorFile(file: Pick<File, "name" | "size">): void {
  if (!/\.(pem|der|cer|crt)$/i.test(file.name)) throw new Error("Choose a public .pem, .der, .cer, or .crt root CA certificate, not a private key.");
  if (!Number.isSafeInteger(file.size) || file.size <= 0 || file.size > 64 * 1024) {
    throw new Error("Choose one nonempty public CA certificate of at most 64 KiB.");
  }
}

export function signingField(report: SignatureReport, fieldId: string | null): SignatureField | null {
  if (report.signingBlockedReason) throw new Error(report.signingBlockedReason);
  if (fieldId === null) return null;
  const field = report.emptyFields.find((field) => field.id === fieldId);
  if (!field) throw new Error("That signature field no longer exists. Refresh and select a current empty field.");
  if (field.blockedReason) throw new Error(field.blockedReason);
  return field;
}

export function chainLabel(chain: SignatureChain): string {
  return {
    not_checked: "Certificate-chain trust not checked",
    untrusted: "No valid chain to your custom CA store",
    anchored_custom_store: "Chain anchored in your custom CA store",
  }[chain.status];
}

export function signedCopyName(name: string): string {
  return name.replace(/\.pdf$/i, "") + "-signed.pdf";
}
