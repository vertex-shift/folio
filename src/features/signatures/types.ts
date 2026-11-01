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
  evidence: SignatureEvidence[];
  evidenceBytes: number;
}

export type SignatureEvidenceKind = "crl" | "timestamp";

export interface SignatureEvidence {
  kind: SignatureEvidenceKind;
  fingerprint: string;
  bytes: number;
  issuer: string;
  thisUpdate: string | null;
  nextUpdate: string | null;
  crlNumber: string | null;
  genTime: string | null;
  boundCertificateFingerprint: string | null;
  boundSignatureValueSha256: string | null;
}

export type RevocationStatus = "not_checked" | "good_current_evidence" | "revoked" | "incomplete"
  | "missing" | "expired" | "future" | "invalid" | "unsupported" | "untrusted";

export interface CertificateRevocation {
  certificateFingerprint: string;
  serial: string;
  issuerFingerprint: string;
  status: RevocationStatus;
  message: string;
  evidenceFingerprint: string | null;
  thisUpdate: string | null;
  nextUpdate: string | null;
  revocationTime: string | null;
}

export interface SignatureRevocation {
  status: RevocationStatus;
  message: string;
  checkedAt: string | null;
  certificates: CertificateRevocation[];
}

export type TrustedTimeStatus = "not_checked" | "verified_custom_store" | "incomplete"
  | "revoked" | "future" | "untrusted" | "invalid" | "unsupported";

export interface SignatureTimestamp {
  status: TrustedTimeStatus;
  message: string;
  source: "embedded" | "imported";
  evidenceFingerprint: string;
  cryptographicIntegrity: SignatureInfo["integrity"];
  imprintMatches: boolean | null;
  imprintDigest: string | null;
  boundSignatureValueSha256: string;
  genTime: string | null;
  accuracyMicros: number | null;
  policyOid: string | null;
  serialNumber: string | null;
  nonce: string | null;
  ordering: boolean | null;
  tsaCertificate: SignatureCertificate | null;
  tsaCurrentChain: SignatureChain;
  tsaChainAtGenTime: SignatureChain;
  tsaRevocation: SignatureRevocation;
}

export interface SignatureTrustedTime {
  status: TrustedTimeStatus;
  message: string;
  timestamps: SignatureTimestamp[];
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
  signatureValueSha256: string | null;
  revocation: SignatureRevocation;
  trustedTime: SignatureTrustedTime;
}

export interface SignatureReport {
  documentId: number;
  revision: number;
  epoch: number;
  checkedAtUnixMs: number;
  recheckAtUnixMs: number;
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

export function signatureReportFresh(report: SignatureReport, now = Date.now()): boolean {
  return Number.isSafeInteger(report.checkedAtUnixMs) && Number.isSafeInteger(report.recheckAtUnixMs)
    && report.checkedAtUnixMs >= 0 && report.checkedAtUnixMs <= now
    && report.recheckAtUnixMs > now && report.recheckAtUnixMs <= report.checkedAtUnixMs + 60_000;
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

export function validateEvidenceFile(file: Pick<File, "name" | "size">, kind: SignatureEvidenceKind): void {
  const validName = kind === "crl" ? /\.(crl|pem|der)$/i : /\.(tsr|tst|der)$/i;
  if (!validName.test(file.name)) throw new Error(kind === "crl"
    ? "Choose one public .crl, .pem or .der CRL, not a private key or certificate bundle."
    : "Choose one DER .tsr response or .tst/.der RFC3161 timestamp token.");
  const limit = kind === "crl" ? 1024 * 1024 : 256 * 1024;
  if (!Number.isSafeInteger(file.size) || file.size <= 0 || file.size > limit) {
    throw new Error(`Choose nonempty public ${kind === "crl" ? "CRL evidence up to 1 MiB" : "timestamp evidence up to 256 KiB"}.`);
  }
}

export function evidenceTarget(report: SignatureReport, kind: SignatureEvidenceKind, signatureId: string | null): SignatureInfo | null {
  if (kind === "crl") {
    if (signatureId !== null) throw new Error("CRLs are issuer evidence, not signature-specific timestamps.");
    return null;
  }
  const signature = report.signatures.find((signature) => signature.id === signatureId);
  if (!signature || signature.integrity !== "intact" || !signature.signatureValueSha256 || !signature.certificate) {
    throw new Error("Refresh and select an actual PDF signature whose exact-byte cryptographic checks pass.");
  }
  return signature;
}

export function revocationLabel(status: RevocationStatus): string {
  return {
    not_checked: "Revocation not checked",
    good_current_evidence: "No revocation listed in current imported CRLs",
    revoked: "Certificate revoked in current imported evidence",
    incomplete: "Revocation evidence incomplete",
    missing: "Issuer CRL missing",
    expired: "Imported CRL expired",
    future: "Imported CRL future-dated",
    invalid: "Revocation evidence failed verification",
    unsupported: "Revocation evidence unsupported",
    untrusted: "Revocation path is not custom-anchored",
  }[status];
}

export function trustedTimeLabel(status: TrustedTimeStatus): string {
  return {
    not_checked: "Trusted time not checked",
    verified_custom_store: "Timestamp verified with custom evidence",
    incomplete: "Timestamp TSA evidence incomplete",
    revoked: "Timestamp TSA path revoked",
    future: "Timestamp time is in the future",
    untrusted: "Timestamp TSA path is not trusted",
    invalid: "Timestamp verification failed",
    unsupported: "Timestamp evidence unsupported",
  }[status];
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
