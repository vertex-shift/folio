import { useEffect, useRef, useState } from "react";
import { FileKey2, FolderOpen, LockKeyhole } from "lucide-react";
import Dialog from "../../components/Dialog";
import type { DocumentInfo } from "../../lib/types";
import { native } from "../../lib/bridge";
import { errorMessage } from "../../lib/geometry";
import { signatureBridge } from "./bridge";
import { signingField, validateIdentityFile } from "./types";
import type { SignatureReport, SigningIdentity } from "./types";

interface Props {
  info: DocumentInfo;
  report: SignatureReport;
  busy: boolean;
  error: string | null;
  onClose(): void;
  onExport(identity: SigningIdentity, password: string, fieldId: string | null): void;
}

export default function SigningDialog({ info, report, busy, error, onClose, onExport }: Props) {
  const [identity, setIdentity] = useState<SigningIdentity | null>(null);
  const [password, setPassword] = useState("");
  const [reviewed, setReviewed] = useState(false);
  const [fieldId, setFieldId] = useState<string | null>(null);
  const [picking, setPicking] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const passwordInput = useRef<HTMLInputElement>(null);
  const errorElement = useRef<HTMLParagraphElement>(null);
  const blocked = busy || picking;
  const message = failure ?? error;
  const selected = report.emptyFields.find((field) => field.id === fieldId);
  const coSigning = report.signatures.length > 0;

  useEffect(() => {
    if (message && !blocked) {
      passwordInput.current?.focus();
      errorElement.current?.scrollIntoView({ block: "nearest" });
    }
  }, [message, blocked]);

  async function choose() {
    if (!native) { input.current?.click(); return; }
    setPicking(true); setFailure(null);
    try {
      const file = await signatureBridge.chooseIdentity(info);
      if (file) { setIdentity({ kind: "native", file }); setPassword(""); }
    } catch (error) { setFailure(errorMessage(error)); }
    finally { setPicking(false); }
  }

  function submit() {
    if (!identity || !reviewed || blocked) return;
    try { signingField(report, fieldId); }
    catch (error) { setFailure(errorMessage(error)); return; }
    const secret = password;
    setPassword("");
    setFailure(null);
    onExport(identity, secret, fieldId);
  }

  return <Dialog title={coSigning ? "Add an approval signature to a separate copy" : "Export a cryptographically signed copy"} className="signature-dialog" busy={blocked} onClose={onClose}
    actions={<><button className="button secondary" disabled={blocked} onClick={onClose}>Cancel</button>
      <button className="button primary" disabled={blocked || !identity || !reviewed} onClick={submit}>
        <FileKey2 size={15} />{busy ? "Signing copy..." : "Sign and export copy"}
      </button></>}>
    <p className="signature-document-name">Sign <strong>{info.name}</strong> with a local certificate and private key.
      {info.dirty ? " Current unsaved edits are included." : ""} This adds a detached CMS approval signature, not a drawn signature image.
      {coSigning ? " Earlier signatures and every source byte are retained; earlier signatures cover their own revisions, not this added approval." : ""}</p>
    <label className="signature-field-picker">Signature location
      <select value={fieldId ?? ""} disabled={blocked} onChange={(event) => { setFieldId(event.target.value || null); setReviewed(false); }}>
        <option value="">New invisible approval field (separate copy)</option>
        {report.emptyFields.map((field) => <option key={field.id} value={field.id} disabled={Boolean(field.blockedReason)}>
          {field.name} · {field.id}{field.widgets.length ? ` · page ${field.widgets[0]!.page + 1}` : ""}{field.blockedReason ? " (unavailable)" : ""}
        </option>)}
      </select>
    </label>
    <p className="muted">{selected
      ? `${selected.widgets.some((widget) => widget.visible) ? "Visible widgets get a persistent signed appearance with a certificate fingerprint and an explicitly unverified local time. " : "This selected widget is invisible. "}Field identity, page geometry, and other fields are preserved. The appearance is not proof of validation or trusted time.`
      : "The new approval field is invisible. This preserves the existing signing default; select an available blank field above to use its page widgets."}</p>
    <input ref={input} className="file-input" type="file" accept=".p12,.pfx,application/x-pkcs12" aria-label="Choose PKCS#12 signing identity"
      onChange={(event) => {
        const file = event.currentTarget.files?.[0];
        event.currentTarget.value = "";
        if (!file) return;
        try {
          validateIdentityFile(file);
          setIdentity({ kind: "browser", file }); setPassword(""); setFailure(null);
        } catch (error) { setFailure(errorMessage(error)); }
      }} />
    <div className="signature-identity-picker">
      <button className="button secondary" data-autofocus disabled={blocked} onClick={() => void choose()}><FolderOpen size={15} />Choose PKCS#12 identity</button>
      <span>{identity?.file.name ?? "Local .p12 or .pfx file, up to 4 MiB"}</span>
    </div>
    <label className="signature-password">Identity password
      <input ref={passwordInput} type="password" autoComplete="off" spellCheck={false} maxLength={1024} value={password} disabled={blocked || !identity}
        aria-describedby={message ? `signature-error-${info.id}` : undefined}
        onChange={(event) => setPassword(event.target.value)}
        onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); submit(); } }} />
    </label>
    {message && <p ref={errorElement} id={`signature-error-${info.id}`} className="inline-error" role="alert">{message}</p>}
    <p className="signature-password-hint"><LockKeyhole size={13} />The password is cleared after submission. Identity keys and passwords stay in memory and are never saved in preferences.</p>
    <label className="checkbox-setting signature-acknowledgment"><input type="checkbox" checked={reviewed} disabled={blocked}
      onChange={(event) => setReviewed(event.target.checked)} />I have reviewed the current PDF and want to sign a separate copy.</label>
    <p className="muted">Uses SHA-256 with a currently valid RSA or supported ECDSA certificate.
      The source, its saved state, and history stay unchanged. Open the exported copy to check integrity and, optionally, its chain against your custom CAs. No revocation or trusted-time checks.</p>
    <p className="muted">Encrypted, repaired/hybrid, certified, XFA, field-locking/seed-value policies, ambiguous signatures, and unsigned changes after a signature are refused. Ordinary signed-file editing stays read-only. No TSA/OCSP requests or external uploads.</p>
    {!native && <p className="muted">Development preview sends the selected identity only to its loopback Rust engine and downloads a new PDF. It does not write to an existing document path.</p>}
  </Dialog>;
}
