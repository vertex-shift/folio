import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { BadgeCheck, CircleAlert, Clock3, FileKey2, FolderOpen, LoaderCircle, RefreshCw, Trash2 } from "lucide-react";
import type { DocumentInfo } from "../../lib/types";
import { errorMessage } from "../../lib/geometry";
import { native } from "../../lib/bridge";
import { signatureBridge } from "./bridge";
import {
  certificateValidityLabel, chainLabel, evidenceTarget, revocationLabel, signatureReportFresh,
  signatureSummary, trustedTimeLabel, validateAnchorFile, validateEvidenceFile,
} from "./types";
import type { SignatureEvidence, SignatureEvidenceKind, SignatureReport, SignatureRevocation, SignatureTimestamp } from "./types";
import { signatureTrustState } from "./trustState";
import "./signatures.css";

interface Props {
  info: DocumentInfo;
  active: boolean;
  disabled: boolean;
  pendingDraft: boolean;
  onSign(report: SignatureReport): void;
}

interface PickerRequest {
  info: DocumentInfo;
  report: SignatureReport;
  kind: "anchor" | SignatureEvidenceKind;
  signatureId: string | null;
  generation: number;
  lifetime: number;
}

function evidenceClass(status: string): string {
  if (status === "revoked" || status === "invalid") return "signature-evidence-failed";
  if (status === "good_current_evidence" || status === "verified_custom_store") return "signature-evidence-checked";
  return "signature-evidence-warning";
}

function RevocationDetails({ result, title }: { result: SignatureRevocation; title: string }) {
  return <details className="signature-certificate">
    <summary>{title} ({result.certificates.length} non-anchor certificates)</summary>
    {result.checkedAt && <p className="signature-check-time">Checked at {result.checkedAt}</p>}
    {!result.certificates.length && <p>No verified issuer path was available for CRL coverage checks.</p>}
    {result.certificates.map((certificate) => <section className="signature-coverage-row" key={certificate.certificateFingerprint}>
      <h5 className={evidenceClass(certificate.status)}>{revocationLabel(certificate.status)}</h5>
      <p>{certificate.message}</p>
      <dl className="signature-facts">
        <div><dt>Certificate serial</dt><dd>{certificate.serial}</dd></div>
        <div><dt>Certificate fingerprint</dt><dd className="signature-fingerprint">{certificate.certificateFingerprint}</dd></div>
        <div><dt>Actual issuing CA fingerprint</dt><dd className="signature-fingerprint">{certificate.issuerFingerprint}</dd></div>
        {certificate.evidenceFingerprint && <div><dt>Imported CRL fingerprint</dt><dd className="signature-fingerprint">{certificate.evidenceFingerprint}</dd></div>}
        {certificate.thisUpdate && <div><dt>CRL thisUpdate</dt><dd>{certificate.thisUpdate}</dd></div>}
        {certificate.nextUpdate && <div><dt>CRL nextUpdate</dt><dd>{certificate.nextUpdate}</dd></div>}
        {certificate.revocationTime && <div><dt>Listed revocation time</dt><dd>{certificate.revocationTime} (not a historical clearance)</dd></div>}
      </dl>
    </section>)}
  </details>;
}

function TimestampDetails({ timestamp }: { timestamp: SignatureTimestamp }) {
  return <details className="signature-certificate signature-timestamp">
    <summary>{timestamp.source === "embedded" ? "Embedded token" : "Imported token"}: {trustedTimeLabel(timestamp.status)}</summary>
    <p>{timestamp.message}</p>
    <dl className="signature-facts">
      <div><dt>TSA token cryptography</dt><dd>{{
        intact: "TSA CMS signature and signed TSTInfo digest verified",
        invalid: "TSA token cryptographic checks failed",
        unsupported: "TSA token format or algorithm unsupported",
      }[timestamp.cryptographicIntegrity]}</dd></div>
      <div><dt>Actual PDF signature-value imprint</dt><dd>{timestamp.imprintMatches === true
        ? "Matches the actual CMS signature value" : timestamp.imprintMatches === false ? "Imprint does not match" : "Not evaluated"}</dd></div>
      <div><dt>TSA time (genTime)</dt><dd>{timestamp.genTime ?? "Not parsed"} (not PDF /M or the signer&apos;s claimed date)</dd></div>
      <div><dt>Signed accuracy</dt><dd>{timestamp.accuracyMicros === null
        ? "Not supplied; no clock accuracy is inferred" : `Plus or minus ${timestamp.accuracyMicros.toLocaleString()} microseconds`}</dd></div>
      <div><dt>Imprint digest</dt><dd>{timestamp.imprintDigest ?? "Not parsed"}</dd></div>
      <div><dt>Policy OID</dt><dd>{timestamp.policyOid ?? "Not parsed"} (policy and TSA clock not independently audited)</dd></div>
      <div><dt>Token serial / nonce</dt><dd>{timestamp.serialNumber ?? "Not parsed"} / {timestamp.nonce ?? "Not supplied"} (no original request nonce check)</dd></div>
      <div><dt>Signature value SHA-256</dt><dd className="signature-fingerprint">{timestamp.boundSignatureValueSha256}</dd></div>
      <div><dt>Public token fingerprint</dt><dd className="signature-fingerprint">{timestamp.evidenceFingerprint}</dd></div>
      <div><dt>TSA path now</dt><dd>{chainLabel(timestamp.tsaCurrentChain)}</dd></div>
      <div><dt>TSA path at genTime</dt><dd>{chainLabel(timestamp.tsaChainAtGenTime)} (not historical revocation)</dd></div>
      {timestamp.tsaChainAtGenTime.anchorFingerprint && <div><dt>TSA custom CA anchor</dt><dd className="signature-fingerprint">{timestamp.tsaChainAtGenTime.anchorFingerprint}</dd></div>}
      {timestamp.tsaCertificate && <>
        <div><dt>Actual TSA certificate</dt><dd>{timestamp.tsaCertificate.subject}</dd></div>
        <div><dt>TSA certificate dates now</dt><dd>{certificateValidityLabel(timestamp.tsaCertificate.validity)}</dd></div>
        <div><dt>TSA fingerprint (ESSCertIDv2-bound)</dt><dd className="signature-fingerprint">{timestamp.tsaCertificate.sha256Fingerprint}</dd></div>
      </>}
    </dl>
    <p>{timestamp.tsaCurrentChain.message}</p>
    <p>{timestamp.tsaChainAtGenTime.message}</p>
    <h5 className={evidenceClass(timestamp.tsaRevocation.status)}>{revocationLabel(timestamp.tsaRevocation.status)}</h5>
    <p>{timestamp.tsaRevocation.message}</p>
    <RevocationDetails result={timestamp.tsaRevocation} title="Current TSA issuer CRL coverage" />
  </details>;
}

function evidenceUsage(report: SignatureReport, evidence: SignatureEvidence): string {
  const timestamps = report.signatures.flatMap((signature) => signature.trustedTime.timestamps);
  if (evidence.kind === "timestamp") {
    const result = timestamps.find((timestamp) => timestamp.source === "imported" && timestamp.evidenceFingerprint === evidence.fingerprint);
    return result ? trustedTimeLabel(result.status) : "Bound to another signature; not checked for this PDF";
  }
  const checks = [...report.signatures.flatMap((signature) => signature.revocation.certificates),
    ...timestamps.flatMap((timestamp) => timestamp.tsaRevocation.certificates)]
    .filter((certificate) => certificate.evidenceFingerprint === evidence.fingerprint);
  return checks.length ? [...new Set(checks.map((certificate) => revocationLabel(certificate.status)))].join("; ")
    : "Not used by a verified issuer path in this PDF";
}

export default function SignaturesPanel({ info, active, disabled, pendingDraft, onSign }: Props) {
  const [inspection, setReport] = useState<SignatureReport | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(false);
  const [storeBusy, setStoreBusy] = useState(false);
  const [storeError, setStoreError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const anchorInput = useRef<HTMLInputElement>(null);
  const crlInput = useRef<HTMLInputElement>(null);
  const timestampInput = useRef<HTMLInputElement>(null);
  const picker = useRef<PickerRequest | null>(null);
  const generation = useRef(0);
  const live = useRef({ info, active, disabled });
  live.current = { info, active, disabled };
  const report = inspection && active && !disabled && inspection.documentId === info.id
    && inspection.revision === info.revision && inspection.trustStore.revision === signatureTrustState.currentRevision()
    && signatureReportFresh(inspection) ? inspection : null;

  useEffect(() => signatureTrustState.subscribe((documentId) => {
    if (documentId !== undefined && documentId !== info.id) return;
    generation.current++;
    picker.current = null;
    setReport(null);
    setRefresh((value) => value + 1);
  }), [info.id]);
  useLayoutEffect(() => {
    generation.current++;
    picker.current = null;
    setReport(null); setFailure(null); setStoreError(null); setNotice(null);
    signatureTrustState.invalidateDocument(info.id);
    return () => { generation.current++; picker.current = null; };
  }, [active, disabled, info.id, info.revision, pendingDraft]);
  useEffect(() => {
    if (!active || disabled || storeBusy) { setLoading(false); return; }
    let current = true;
    const requested = generation.current;
    setLoading(true); setFailure(null); setReport(null);
    void signatureBridge.inspect(info).then((next) => {
      if (current && requested === generation.current) setReport(next);
    }).catch((error: unknown) => {
      if (current && requested === generation.current) setFailure(errorMessage(error));
    }).finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
  }, [active, disabled, info.id, info.revision, pendingDraft, refresh, storeBusy]);
  useEffect(() => {
    if (!inspection) return;
    const invalidate = () => { signatureTrustState.invalidateDocument(info.id); };
    const clockChanged = () => { if (!signatureReportFresh(inspection)) invalidate(); };
    const timer = window.setTimeout(invalidate, Math.max(0, inspection.recheckAtUnixMs - Date.now()));
    window.addEventListener("focus", clockChanged);
    document.addEventListener("visibilitychange", clockChanged);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("focus", clockChanged);
      document.removeEventListener("visibilitychange", clockChanged);
    };
  }, [inspection, info.id]);
  useEffect(() => {
    const inputs = [anchorInput.current, crlInput.current, timestampInput.current];
    for (const input of inputs) input?.addEventListener("cancel", cancelPicker);
    return () => { for (const input of inputs) input?.removeEventListener("cancel", cancelPicker); };
  }, []);

  function sameSource(request: PickerRequest): boolean {
    return live.current.active && !live.current.disabled && live.current.info.id === request.info.id
      && live.current.info.revision === request.info.revision;
  }

  function pickerCurrent(request: PickerRequest): boolean {
    return sameSource(request) && request.generation === generation.current
      && signatureTrustState.documentCurrent(request.info.id, request.lifetime)
      && request.report.trustStore.revision === signatureTrustState.currentRevision()
      && signatureReportFresh(request.report);
  }

  function cancelPicker() {
    picker.current = null;
    setNotice("Public evidence selection canceled. The PDF, drafts and custom store are unchanged.");
  }

  async function importPublic(file?: File) {
    const request = picker.current;
    if (!request || !pickerCurrent(request)) {
      setStoreError("The PDF, its draft lifetime or the custom store changed while choosing evidence. Refresh and choose it again.");
      picker.current = null;
      return;
    }
    setStoreBusy(true); setStoreError(null); setNotice(null);
    try {
      if (file) {
        if (request.kind === "anchor") validateAnchorFile(file);
        else validateEvidenceFile(file, request.kind);
      }
      const store = request.kind === "anchor"
        ? await signatureBridge.addAnchor(request.info, request.report.trustStore.revision, file)
        : await signatureBridge.addEvidence(request.info, request.report, request.kind, request.signatureId, file);
      if (sameSource(request)) setNotice(store
        ? "Public evidence imported for this app run. Signature-specific checks are refreshed separately; the PDF is unchanged."
        : "Public evidence selection canceled. The PDF, drafts and custom store are unchanged.");
    } catch (error) { if (sameSource(request)) setStoreError(errorMessage(error)); }
    finally { setStoreBusy(false); picker.current = null; }
  }

  function choosePublic(kind: PickerRequest["kind"], signatureId: string | null = null) {
    if (!report || disabled || loading || storeBusy) return;
    try {
      if (kind !== "anchor") evidenceTarget(report, kind, signatureId);
      picker.current = { info, report, kind, signatureId, generation: generation.current,
        lifetime: signatureTrustState.captureDocument(info.id) };
      setStoreError(null); setNotice(null);
      if (native) void importPublic();
      else if (kind === "anchor") anchorInput.current?.click();
      else if (kind === "crl") crlInput.current?.click();
      else timestampInput.current?.click();
    } catch (error) { setStoreError(errorMessage(error)); }
  }

  async function removePublic(fingerprint: string, kind: "anchor" | "evidence") {
    if (!report || disabled || loading || storeBusy) return;
    setStoreBusy(true); setStoreError(null); setNotice(null);
    try {
      if (kind === "anchor") await signatureBridge.removeAnchor(info, report.trustStore.revision, fingerprint);
      else await signatureBridge.removeEvidence(info, report, fingerprint);
      if (live.current.active && live.current.info.id === info.id && live.current.info.revision === info.revision) {
        setNotice("Imported public evidence removed. The source PDF and any embedded timestamp remain unchanged.");
      }
    } catch (error) { setStoreError(errorMessage(error)); }
    finally { setStoreBusy(false); }
  }

  const blockedReason = pendingDraft
    ? "Apply or discard pending edits and clear redaction marks before signing. Drafts and marks are not part of the PDF."
    : report?.signingBlockedReason ?? null;
  const controlsDisabled = disabled || loading || storeBusy || !report;

  return <div className="signatures-panel">
    <div className="panel-scroll signature-review">
      <div className="signature-intro">
        <p>Check cryptographic signatures without changing this PDF.</p>
        <p><strong>Content integrity is not identity trust.</strong> Use only your explicitly imported public CA, full CRL and RFC3161 evidence. No downloads or TSA requests.</p>
        {pendingDraft && <p>Pending edits are not part of the inspected source. Your drafts remain protected.</p>}
        <button className="button secondary" disabled={disabled || loading || storeBusy}
          onClick={() => signatureTrustState.invalidateDocument(info.id)}>
          <RefreshCw size={14} />Refresh checks
        </button>
        {report && <p className="signature-check-time">Checked {new Date(report.checkedAtUnixMs).toLocaleTimeString()} · source epoch {report.epoch} · store revision {report.trustStore.revision}</p>}
      </div>
      {loading && <p className="signature-progress" role="status"><LoaderCircle size={15} className="spin" />Checking signed bytes and offline evidence...</p>}
      {storeBusy && <p className="signature-progress" role="status"><LoaderCircle size={15} className="spin" />Updating public evidence...</p>}
      {failure && <p className="inline-error signature-failure" role="alert">{failure}</p>}
      <input ref={anchorInput} className="file-input" type="file" accept=".pem,.der,.cer,.crt"
        aria-label="Choose public root CA certificate"
        onChange={(event) => {
          const file = event.currentTarget.files?.[0]; event.currentTarget.value = "";
          if (file) void importPublic(file); else cancelPicker();
        }} />
      <input ref={crlInput} className="file-input" type="file" accept=".crl,.pem,.der"
        aria-label="Choose public full CRL"
        onChange={(event) => {
          const file = event.currentTarget.files?.[0]; event.currentTarget.value = "";
          if (file) void importPublic(file); else cancelPicker();
        }} />
      <input ref={timestampInput} className="file-input" type="file" accept=".tsr,.tst,.der"
        aria-label="Choose RFC3161 timestamp for selected signature"
        onChange={(event) => {
          const file = event.currentTarget.files?.[0]; event.currentTarget.value = "";
          if (file) void importPublic(file); else cancelPicker();
        }} />
      {report && <details className="signature-anchors">
        <summary>Custom CA store ({report.trustStore.anchors.length})</summary>
        <p>Only add roots you independently trust. One public PEM/DER certificate up to 64 KiB, 32 anchors maximum.</p>
        <p>Anchors and evidence are <strong>in memory for this app run only</strong>, shared by tabs. No OS/keychain or Adobe AATL/EUTL trust.</p>
        {report.trustStore.anchors.map((anchor) => <div className="signature-anchor" key={anchor.sha256Fingerprint}>
          <strong>{anchor.subject}</strong><span>{certificateValidityLabel(anchor.validity)}</span>
          <code>{anchor.sha256Fingerprint}</code>
          <button className="button secondary" disabled={controlsDisabled}
            aria-label={`Remove custom CA ${anchor.subject}`} onClick={() => void removePublic(anchor.sha256Fingerprint, "anchor")}>
            <Trash2 size={13} />Remove anchor
          </button>
        </div>)}
        {!report.trustStore.anchors.length && <p>No anchors loaded. Embedded signer/TSA certificates are not trusted automatically.</p>}
        <button className="button secondary" disabled={controlsDisabled} onClick={() => choosePublic("anchor")}>
          <FolderOpen size={14} />Add public root CA
        </button>
      </details>}
      {report && <details className="signature-anchors signature-evidence-store">
        <summary>Offline public evidence ({report.trustStore.evidence.length})</summary>
        <p>One full direct CRL up to 1 MiB, or one DER RFC3161 response/token up to 256 KiB. Up to 32 items / 8 MiB; four imported tokens per signature.</p>
        <p>Loading is not a blanket trust verdict. CRLs must cover every non-anchor certificate in the actual path. Import timestamps with the signature-specific button below.</p>
        {report.trustStore.evidence.map((evidence) => <div className="signature-anchor" key={evidence.fingerprint}>
          <strong>{evidence.kind === "crl" ? "Full CRL" : "RFC3161 token"}: {evidence.issuer}</strong>
          <span>{evidenceUsage(report, evidence)}</span>
          {evidence.thisUpdate && <span>thisUpdate: {evidence.thisUpdate}</span>}
          {evidence.nextUpdate && <span>nextUpdate: {evidence.nextUpdate}</span>}
          {evidence.crlNumber && <span>CRL number (hex): {evidence.crlNumber}</span>}
          {evidence.genTime && <span>TSA genTime: {evidence.genTime}</span>}
          <code>{evidence.fingerprint}</code>
          {evidence.boundCertificateFingerprint && <span>Bound signer certificate: <code>{evidence.boundCertificateFingerprint}</code></span>}
          {evidence.boundSignatureValueSha256 && <span>Bound signature value: <code>{evidence.boundSignatureValueSha256}</code></span>}
          <span>{evidence.bytes.toLocaleString()} public DER bytes</span>
          <button className="button secondary" disabled={controlsDisabled}
            aria-label={`Remove imported ${evidence.kind} ${evidence.fingerprint}`} onClick={() => void removePublic(evidence.fingerprint, "evidence")}>
            <Trash2 size={13} />Remove imported evidence
          </button>
        </div>)}
        {!report.trustStore.evidence.length && <p>No public revocation or timestamp evidence imported.</p>}
        <p>{report.trustStore.evidenceBytes.toLocaleString()} of 8,388,608 DER bytes loaded. Embedded tokens stay in the PDF; only imported evidence can be removed here.</p>
        <button className="button secondary" disabled={controlsDisabled} onClick={() => choosePublic("crl")}>
          <FolderOpen size={14} />Import full CRL
        </button>
      </details>}
      {storeError && <p className="inline-error signature-failure" role="alert">{storeError}</p>}
      {notice && <p className="signature-notice" role="status">{notice}</p>}
      {report?.warnings.map((warning) => <p className="signature-warning" key={warning}>{warning}</p>)}
      {report && !report.signatures.length && !report.warnings.length && <div className="panel-empty">
        <FileKey2 size={28} /><h3>No signatures found</h3><p>This working PDF has no detected signature dictionaries. Export a separate copy to sign it.</p>
      </div>}
      {report?.signatures.map((signature, index) => {
        const summary = signatureSummary(signature);
        const certificate = signature.certificate;
        return <article className="signature-entry" key={signature.id}>
          <h3 className={`signature-status signature-${summary.tone}`}>
            {summary.tone === "intact" ? <BadgeCheck size={17} /> : <CircleAlert size={17} />}{summary.title}
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
            <p className={certificate.validity === "valid_now" ? "signature-date" : "signature-date signature-date-warning"}>{certificateValidityLabel(certificate.validity)}</p>
            <details className="signature-certificate">
              <summary>Certificate and algorithm details</summary>
              <dl className="signature-facts">
                <div><dt>Algorithm</dt><dd>{signature.algorithm} / {signature.digest}</dd></div>
                <div><dt>Issuer</dt><dd>{certificate.issuer || "No issuer name"}</dd></div>
                <div><dt>Valid from / until</dt><dd>{certificate.notBefore} / {certificate.notAfter}</dd></div>
                <div><dt>Serial number</dt><dd>{certificate.serial}</dd></div>
                <div><dt>SHA-256 fingerprint</dt><dd className="signature-fingerprint">{certificate.sha256Fingerprint}</dd></div>
                {signature.chain.anchorFingerprint && <div><dt>Custom CA anchor</dt><dd className="signature-fingerprint">{signature.chain.anchorFingerprint}</dd></div>}
                {signature.chain.chainFingerprints.map((fingerprint, index) => <div key={fingerprint}><dt>Chain certificate {index + 1}</dt><dd className="signature-fingerprint">{fingerprint}</dd></div>)}
                <div><dt>Claimed signing time</dt><dd>{signature.claimedSigningTime ?? "Not provided"} (unverified claim, not trusted time)</dd></div>
                <div><dt>Actual signature value SHA-256</dt><dd className="signature-fingerprint">{signature.signatureValueSha256 ?? "Exact-byte checks did not pass"}</dd></div>
              </dl>
            </details>
          </>}
          <section className="signature-evidence-result">
            <h4 className={evidenceClass(signature.revocation.status)}><CircleAlert size={14} />{revocationLabel(signature.revocation.status)}</h4>
            <p>{signature.revocation.message}</p>
            <RevocationDetails result={signature.revocation} title="Current issuer CRL coverage" />
          </section>
          <section className="signature-evidence-result">
            <h4 className={evidenceClass(signature.trustedTime.status)}><Clock3 size={14} />{trustedTimeLabel(signature.trustedTime.status)}</h4>
            <p>{signature.trustedTime.message}</p>
            {signature.trustedTime.timestamps.map((timestamp) => <TimestampDetails timestamp={timestamp} key={`${timestamp.source}:${timestamp.evidenceFingerprint}`} />)}
            <button className="button secondary" disabled={controlsDisabled || signature.integrity !== "intact" || !signature.signatureValueSha256}
              onClick={() => choosePublic("timestamp", signature.id)}>
              <FolderOpen size={14} />Import timestamp for signature {index + 1}
            </button>
            {signature.integrity !== "intact" && <p>Timestamp imports require passing supported exact-byte signature checks.</p>}
          </section>
        </article>;
      })}
      <p className="signature-limitations">Current imported CRLs are not historical revocation checks. TSA time never clears signer revocation, failed PDF integrity or current untrusted dates. No system/Adobe trust, OCSP/CRL downloads, TSA submissions, qualified-signature or PAdES-LTV assurance.</p>
    </div>
    <div className="signature-export-actions">
      <p>{blockedReason ?? "Sign the current working PDF, including unsaved edits. Your source and its history stay unchanged."}</p>
      <button className="button primary" disabled={controlsDisabled || Boolean(blockedReason)} onClick={() => { if (report) onSign(report); }}>
        <FileKey2 size={15} />{report?.signatures.length ? "Export co-signed copy" : "Export signed copy"}
      </button>
    </div>
  </div>;
}
