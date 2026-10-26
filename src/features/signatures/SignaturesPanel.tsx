import { useEffect, useState } from "react";
import { BadgeCheck, CircleAlert, FileKey2, LoaderCircle, RefreshCw } from "lucide-react";
import type { DocumentInfo } from "../../lib/types";
import { errorMessage } from "../../lib/geometry";
import { signatureBridge } from "./bridge";
import { certificateValidityLabel, signatureSummary } from "./types";
import type { SignatureReport } from "./types";
import "./signatures.css";

interface Props {
  info: DocumentInfo;
  active: boolean;
  disabled: boolean;
  pendingDraft: boolean;
  onSign(): void;
}

export default function SignaturesPanel({ info, active, disabled, pendingDraft, onSign }: Props) {
  const [report, setReport] = useState<SignatureReport | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    if (!active) return;
    let current = true;
    setLoading(true); setFailure(null); setReport(null);
    void signatureBridge.inspect(info).then((next) => {
      if (next.documentId !== info.id || next.revision !== info.revision) {
        throw new Error("The document changed while checking its signatures. Refresh the checks.");
      }
      if (current) setReport(next);
    }).catch((error: unknown) => {
      if (current) setFailure(errorMessage(error));
    }).finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
  }, [active, info.id, info.revision, refresh]);

  const blockedReason = pendingDraft
    ? "Apply or cancel writing and clear pending redaction marks before signing. Pending marks are not part of the PDF."
    : report?.signingBlockedReason ?? null;

  return <div className="signatures-panel">
    <div className="panel-scroll signature-review">
      <div className="signature-intro">
        <p>Check cryptographic signatures without changing this PDF.</p>
        <p><strong>Content integrity is not identity trust.</strong> Folio does not use a trusted certificate store or check revocation.</p>
        <button className="button secondary" disabled={disabled || loading} onClick={() => setRefresh((value) => value + 1)}>
          <RefreshCw size={14} />Refresh checks
        </button>
      </div>
      {loading && <p className="signature-progress" role="status"><LoaderCircle size={15} className="spin" />Checking signed byte spans...</p>}
      {failure && <p className="inline-error signature-failure" role="alert">{failure}</p>}
      {report?.warnings.map((warning) => <p className="signature-warning" key={warning}>{warning}</p>)}
      {report && !report.signatures.length && !report.warnings.length && <div className="panel-empty">
        <FileKey2 size={28} /><h3>No signatures found</h3><p>This working PDF has no detected signature dictionaries. Export a separate copy to sign it.</p>
      </div>}
      {report?.signatures.map((signature, index) => {
        const summary = signatureSummary(signature);
        const certificate = signature.certificate;
        return <article className="signature-entry" key={signature.id}>
          <h3 className={`signature-status signature-${summary.tone}`}>
            {summary.tone === "intact" ? <BadgeCheck size={17} /> : <CircleAlert size={17} />}
            {summary.title}
          </h3>
          <p className="signature-number">Signature {index + 1} · Object {signature.id}</p>
          <p>{signature.message}</p>
          {signature.signedBytes !== null && <dl className="signature-facts">
            <div><dt>Coverage</dt><dd>{signature.coverage === "whole_document" ? "Entire current file" : "Earlier signed revision only"}</dd></div>
            <div><dt>Signed revision</dt><dd>{signature.signedBytes.toLocaleString()} of {signature.currentBytes.toLocaleString()} bytes</dd></div>
          </dl>}
          {certificate && <>
            <p className="signature-identity-warning"><CircleAlert size={14} />Identity not trusted by Folio{certificate.selfSigned ? " (self-signed)" : ""}</p>
            <p className="signature-subject">{certificate.subject || "Certificate has no subject name"}</p>
            <p className={certificate.validity === "valid_now" ? "signature-date" : "signature-date signature-date-warning"}>
              {certificateValidityLabel(certificate.validity)}
            </p>
            <details className="signature-certificate">
              <summary>Certificate and algorithm details</summary>
              <dl className="signature-facts">
                <div><dt>Algorithm</dt><dd>{signature.algorithm} / {signature.digest}</dd></div>
                <div><dt>Issuer</dt><dd>{certificate.issuer || "No issuer name"}</dd></div>
                <div><dt>Valid from</dt><dd>{certificate.notBefore}</dd></div>
                <div><dt>Valid until</dt><dd>{certificate.notAfter}</dd></div>
                <div><dt>Serial number</dt><dd>{certificate.serial}</dd></div>
                <div><dt>SHA-256 fingerprint</dt><dd className="signature-fingerprint">{certificate.sha256Fingerprint}</dd></div>
              </dl>
            </details>
          </>}
        </article>;
      })}
      <p className="signature-limitations">Checks are local and use this computer&apos;s current time. No certificate-chain trust, revocation, trusted timestamp, qualified-signature, or PAdES-LTV assurance. Later changes and certification policies are not approved.</p>
    </div>
    <div className="signature-export-actions">
      <p>{blockedReason ?? "Sign the current working PDF, including unsaved edits. Your source and its history stay unchanged."}</p>
      <button className="button primary" disabled={disabled || loading || !report || Boolean(blockedReason)} onClick={onSign}>
        <FileKey2 size={15} />Export signed copy
      </button>
    </div>
  </div>;
}
