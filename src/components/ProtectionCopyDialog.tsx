import { useEffect, useRef, useState } from "react";
import { FileOutput, LockKeyhole } from "lucide-react";
import Dialog from "./Dialog";
import type { CopyPermissions, CopyProtection } from "../lib/documentUtilitiesModel";
import { validateCopyProtection } from "../lib/documentUtilitiesModel";
import { errorMessage } from "../lib/geometry";
import type { DocumentInfo } from "../lib/types";

export default function ProtectionCopyDialog({ info, busy, error, onClose, onSubmit }: {
  info: DocumentInfo;
  busy: boolean;
  error: string | null;
  onClose(): void;
  onSubmit(protection: CopyProtection, authorizationPassword: string | null): void;
}) {
  const [mode, setMode] = useState<"protect" | "unprotect">("protect");
  const [opening, setOpening] = useState("");
  const [openingConfirmation, setOpeningConfirmation] = useState("");
  const [owner, setOwner] = useState("");
  const [ownerConfirmation, setOwnerConfirmation] = useState("");
  const [authorization, setAuthorization] = useState("");
  const [acknowledged, setAcknowledged] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const [permissions, setPermissions] = useState<CopyPermissions>({
    print: "high", copy: true, edit: true, annotate: true, fillForms: true, assemble: true,
  });
  const needsOwner = info.passwordProtected && !info.permissions.ownerAuthenticated;
  const authorizationInput = useRef<HTMLInputElement>(null);
  const ownerInput = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!busy && error) (needsOwner ? authorizationInput.current : ownerInput.current)?.focus();
  }, [busy, error, needsOwner]);
  function submit() {
    setLocalError(null);
    const protection: CopyProtection = mode === "unprotect" ? { mode } : {
      mode, openingPassword: opening, ownerPassword: owner, permissions,
    };
    try {
      validateCopyProtection(protection);
      if (mode === "protect" && (opening !== openingConfirmation || owner !== ownerConfirmation)) {
        throw new Error("The password confirmation fields do not match.");
      }
      const password = needsOwner ? authorization : null;
      setOpening(""); setOpeningConfirmation(""); setOwner(""); setOwnerConfirmation(""); setAuthorization("");
      onSubmit(protection, password);
    } catch (error) { setLocalError(errorMessage(error)); }
  }
  return <Dialog title="Export a protection copy" busy={busy} error={localError ?? error} onClose={onClose}
    actions={<><button className="button secondary" disabled={busy} onClick={onClose}>Cancel</button>
      <button className="button primary" type="submit" form="protection-copy-form" disabled={busy || !acknowledged}><FileOutput size={15} />Export separate copy</button></>}>
    <p><strong>{info.name}</strong> stays unchanged, including its current protection and unsaved work. Only the separate output copy uses these settings.</p>
    <form id="protection-copy-form" className="protection-copy-form" onSubmit={(event) => { event.preventDefault(); if (!busy && acknowledged) submit(); }}>
      <label className="utility-field">Copy protection<select data-autofocus value={mode} disabled={busy}
        onChange={(event) => {
          if (event.target.value === "protect" || event.target.value === "unprotect") {
            setMode(event.target.value); setOpening(""); setOpeningConfirmation(""); setOwner(""); setOwnerConfirmation("");
          }
        }}><option value="protect">AES-256 · new passwords and permissions</option>
        <option value="unprotect">No password protection</option></select></label>
      {needsOwner && <label className="utility-field">Current owner password
        <input ref={authorizationInput} type="password" value={authorization} onChange={(event) => setAuthorization(event.target.value)}
          disabled={busy} autoComplete="off" autoCapitalize="none" spellCheck={false} maxLength={1024} />
        <span>The opening password is not owner authorization, even when it permits all edits.</span>
      </label>}
      {mode === "protect" ? <>
        <div className="protection-passwords">
          <label className="utility-field">New opening password (optional)<input type="password" value={opening}
            onChange={(event) => setOpening(event.target.value)} disabled={busy} autoComplete="off" autoCapitalize="none" spellCheck={false} maxLength={127} /></label>
          <label className="utility-field">Confirm opening password<input type="password" value={openingConfirmation}
            onChange={(event) => setOpeningConfirmation(event.target.value)} disabled={busy} autoComplete="off" autoCapitalize="none" spellCheck={false} maxLength={127} /></label>
          <label className="utility-field">New owner password<input ref={ownerInput} type="password" value={owner}
            onChange={(event) => setOwner(event.target.value)} disabled={busy} autoComplete="off" autoCapitalize="none" spellCheck={false} maxLength={127} /></label>
          <label className="utility-field">Confirm owner password<input type="password" value={ownerConfirmation}
            onChange={(event) => setOwnerConfirmation(event.target.value)} disabled={busy} autoComplete="off" autoCapitalize="none" spellCheck={false} maxLength={127} /></label>
        </div>
        <p className="utility-note">An empty opening password lets anyone open the copy. The owner password must be nonempty and different. Each password is limited to 127 UTF-8 bytes; store it securely yourself.</p>
        <fieldset className="protection-permissions" disabled={busy}>
          <legend>Opening-password permissions</legend>
          <label className="utility-field">Printing<select value={permissions.print} onChange={(event) => {
            const print = event.target.value;
            if (print === "none" || print === "low" || print === "high") setPermissions({ ...permissions, print });
          }}><option value="none">Not permitted</option><option value="low">Low resolution only</option><option value="high">High resolution</option></select></label>
          {([
            ["copy", "Copy text, graphics, and attachments"], ["edit", "Modify document content"],
            ["annotate", "Add annotations (also allows form filling)"], ["fillForms", "Fill existing forms"],
            ["assemble", "Organize pages and bookmarks"],
          ] as const).map(([key, label]) => <label className="checkbox-setting" key={key}>
            <input type="checkbox" checked={permissions[key] || key === "fillForms" && permissions.annotate}
              disabled={key === "fillForms" && permissions.annotate}
              onChange={(event) => setPermissions({ ...permissions, [key]: event.target.checked })} />{label}
          </label>)}
        </fieldset>
        <p className="utility-note">PDF permissions are cooperative restrictions, not absolute DRM. Owner access overrides them. Accessibility extraction remains enabled. Some readers do not honor these flags.</p>
      </> : <p className="utility-warning"><LockKeyhole size={15} />Anyone with the exported copy can open and extract its contents. This does not sanitize or redact the document.</p>}
      <label className="checkbox-setting"><input type="checkbox" checked={acknowledged} disabled={busy} onChange={(event) => setAcknowledged(event.target.checked)} />
        I understand this exports a separate copy, not a change to this tab.</label>
    </form>
    <p className="utility-note">AES-256 uses the supported standard PDF 2.0 revision-6 handler and encrypts metadata. Passwords remain in memory only and are cleared on submission or closing this dialog.</p>
  </Dialog>;
}
