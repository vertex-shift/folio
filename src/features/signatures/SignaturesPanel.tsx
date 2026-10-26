import { useEffect, useRef, useState } from "react";
import { BadgeCheck, CircleAlert, FileKey2, FolderOpen, LoaderCircle, RefreshCw, Trash2 } from "lucide-react";
import type { DocumentInfo } from "../../lib/types";
import { errorMessage } from "../../lib/geometry";
import { native } from "../../lib/bridge";
import { signatureBridge } from "./bridge";
import { certificateValidityLabel, chainLabel, signatureSummary, validateAnchorFile } from "./types";
import type { SignatureReport } from "./types";
import { signatureTrustState } from "./trustState";
import "./signatures.css";

interface Props {
  info: DocumentInfo;
  active: boolean;
  disabled: boolean;
  pendingDraft: boolean;
  onSign(report: SignatureReport): void;
}

export default function SignaturesPanel({ info, active, disabled, pendingDraft, onSign }: Props) {
  const [report, setReport] = useState<SignatureReport | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(false);
  const [anchorBusy, setAnchorBusy] = useState(false);
  const [anchorError, setAnchorError] = useState<string | null>(null);
  const anchorInput = useRef<HTMLInputElement>(null);
  const pickerRevision = useRef<{ documentId: number; revision: number; trustRevision: number } | null>(null);
  useEffect(() => signatureTrustState.subscribe(() => {
    setReport(null);
    setRefresh((value) => value + 1);
  }), []);
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

  async function addAnchor(file?: File) {
    const requested = pickerRevision.current;
    if (!requested || requested.documentId !== info.id || requested.revision !== info.revision) {
      setAnchorError("The document changed while choosing a CA certificate. Refresh and choose it again.");
      return;
    }
    setAnchorBusy(true); setAnchorError(null);
    try {
      if (file) validateAnchorFile(file);
      await signatureBridge.addAnchor(info, requested.trustRevision, file);
    } catch (error) { setAnchorError(errorMessage(error)); }
    finally { setAnchorBusy(false); pickerRevision.current = null; }
  }

  function chooseAnchor() {
    if (!report || disabled || loading || anchorBusy) return;
    pickerRevision.current = { documentId: info.id, revision: info.revision, trustRevision: report.trustStore.revision };
    if (native) void addAnchor();
    else anchorInput.current?.click();
  }

  async function removeAnchor(fingerprint: string) {
    if (!report || disabled || loading || anchorBusy) return;
    setAnchorBusy(true); setAnchorError(null);
    try { await signatureBridge.removeAnchor(info, report.trustStore.revision, fingerprint); }
    catch (error) { setAnchorError(errorMessage(error)); }
    finally { setAnchorBusy(false); }
  }

  const blockedReason = pendingDraft
    ? "Apply or discard pending edits and clear redaction marks before signing. Drafts and marks are not part of the PDF."
    : report?.signingBlockedReason ?? null;

  return <div className="signatures-panel">
    <div className="panel-scroll signature-review">
      <div className="signature-intro">
        <p>Check cryptographic signatures without changing this PDF.</p>
        <p><strong>Content integrity is not identity trust.</strong> Chain checks use only public CA certificates you explicitly add below. Revocation is not checked.</p>
        <button className="button secondary" disabled={disabled || loading || anchorBusy} onClick={() => setRefresh((value) => value + 1)}>
          <RefreshCw size={14} />Refresh checks
        </button>
      </div>
      {loading && <p className="signature-progress" role="status"><LoaderCircle size={15} className="spin" />Checking signed byte spans...</p>}
      {failure && <p className="inline-error signature-failure" role="alert">{failure}</p>}
      <input ref={anchorInput} className="file-input" type="file" accept=".pem,.der,.cer,.crt"
        aria-label="Choose public root CA certificate"
        onChange={(event) => {
          const file = event.currentTarget.files?.[0];
          event.currentTarget.value = "";
          if (file) void addAnchor(file);
        }} />
      {report && <details className="signature-anchors">
        <summary>Custom CA store ({report.trustStore.anchors.length})</summary>
        <p>Only add root CAs you independently trust for document signing. Public certificates only; one PEM/DER certificate up to 64 KiB, 32 anchors maximum.</p>
        <p>This store is <strong>in memory for this app run only</strong>, shared by open tabs. It is not Adobe AATL/EUTL or OS trust. No system certificates or downloads are used.</p>
        {report.trustStore.anchors.map((anchor) => <div className="signature-anchor" key={anchor.sha256Fingerprint}>
          <strong>{anchor.subject}</strong>
          <span>{certificateValidityLabel(anchor.validity)}</span>
          <code>{anchor.sha256Fingerprint}</code>
          <button className="button secondary" disabled={disabled || loading || anchorBusy}
            aria-label={`Remove custom CA ${anchor.subject}`} onClick={() => void removeAnchor(anchor.sha256Fingerprint)}>
            <Trash2 size={13} />Remove anchor
          </button>
        </div>)}
        {!report.trustStore.anchors.length && <p>No anchors loaded. Certificate-chain trust is not checked by default.</p>}
        <button className="button secondary" disabled={disabled || loading || anchorBusy} onClick={chooseAnchor}>
          <FolderOpen size={14} />{anchorBusy ? "Updating custom CA store..." : "Add public root CA"}
        </button>
      </details>}
      {anchorError && <p className="inline-error signature-failure" role="alert">{anchorError}</p>}
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
            <p className={`signature-identity-warning${signature.chain.status === "anchored_custom_store" ? " signature-custom-anchor" : ""}`}>
              <CircleAlert size={14} />{chainLabel(signature.chain)}{certificate.selfSigned ? " (self-signed signer)" : ""}
            </p>
            <p>{signature.chain.message}</p>
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
                {signature.chain.anchorFingerprint && <div><dt>Custom CA anchor</dt><dd className="signature-fingerprint">{signature.chain.anchorFingerprint}</dd></div>}
                {signature.chain.chainFingerprints.map((fingerprint, index) => <div key={fingerprint}><dt>Chain certificate {index + 1}</dt><dd className="signature-fingerprint">{fingerprint}</dd></div>)}
                <div><dt>Claimed signing time</dt><dd>{signature.claimedSigningTime ?? "Not provided"} (unverified claim, not trusted time)</dd></div>
                <div><dt>Revocation / trusted timestamp</dt><dd>Not checked. No OCSP, CRL, or TSA requests.</dd></div>
              </dl>
            </details>
          </>}
        </article>;
      })}
      <p className="signature-limitations">Chain and certificate-date checks use this computer&apos;s current time, not historical signing time. No revocation, trusted timestamp, qualified-signature, or PAdES-LTV assurance. Later changes and certification policies are not approved.</p>
    </div>
    <div className="signature-export-actions">
      <p>{blockedReason ?? "Sign the current working PDF, including unsaved edits. Your source and its history stay unchanged."}</p>
      <button className="button primary" disabled={disabled || loading || anchorBusy || !report || Boolean(blockedReason)} onClick={() => { if (report) onSign(report); }}>
        <FileKey2 size={15} />{report?.signatures.length ? "Export co-signed copy" : "Export signed copy"}
      </button>
    </div>
  </div>;
}
