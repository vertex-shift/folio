import { useEffect, useRef, useState } from "react";
import { FileKey2, FolderOpen, LockKeyhole } from "lucide-react";
import Dialog from "../../components/Dialog";
import type { DocumentInfo } from "../../lib/types";
import { native } from "../../lib/bridge";
import { errorMessage } from "../../lib/geometry";
import { signatureBridge } from "./bridge";
import { validateIdentityFile } from "./types";
import type { SigningIdentity } from "./types";

interface Props {
  info: DocumentInfo;
  busy: boolean;
  error: string | null;
  onClose(): void;
  onExport(identity: SigningIdentity, password: string): void;
}

export default function SigningDialog({ info, busy, error, onClose, onExport }: Props) {
  const [identity, setIdentity] = useState<SigningIdentity | null>(null);
  const [password, setPassword] = useState("");
  const [reviewed, setReviewed] = useState(false);
  const [picking, setPicking] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const passwordInput = useRef<HTMLInputElement>(null);
  const errorElement = useRef<HTMLParagraphElement>(null);
  const blocked = busy || picking;
  const message = failure ?? error;

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
    const secret = password;
    setPassword("");
    setFailure(null);
    onExport(identity, secret);
  }

  return <Dialog title="Export a cryptographically signed copy" className="signature-dialog" busy={blocked} onClose={onClose}
    actions={<><button className="button secondary" disabled={blocked} onClick={onClose}>Cancel</button>
      <button className="button primary" disabled={blocked || !identity || !reviewed} onClick={submit}>
        <FileKey2 size={15} />{busy ? "Signing copy..." : "Sign and export copy"}
      </button></>}>
    <p className="signature-document-name">Sign <strong>{info.name}</strong> with a local certificate and private key.
      {info.dirty ? " Current unsaved edits are included." : ""} This creates an invisible, detached CMS signature, not a drawn signature image.</p>
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
      The original remains unsigned and editable. Identity trust, revocation, and trusted signing time are not checked; self-signed certificates are not shown as trusted.</p>
    <p className="muted">Encrypted, already-signed, certified, incremental/hybrid PDFs, XFA, and existing signature fields are not supported for signing. No TSA/OCSP requests or external uploads.</p>
    {!native && <p className="muted">Development preview sends the selected identity only to its loopback Rust engine and downloads a new PDF. It does not write to an existing document path.</p>}
  </Dialog>;
}
