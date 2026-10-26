import { useEffect, useRef, useState } from "react";
import { LockKeyhole } from "lucide-react";
import Dialog from "./Dialog";
import type { PasswordChallenge } from "../lib/types";

export default function PasswordDialog({
  challenge, busy, error, onSubmit, onCancel,
}: {
  challenge: PasswordChallenge;
  busy: boolean;
  error: string | null;
  onSubmit(password: string): void;
  onCancel(): void;
}) {
  const [password, setPassword] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const message = error ?? (challenge.incorrect ? "That password did not unlock this PDF. Try again." : null);

  useEffect(() => {
    if (!busy) input.current?.focus();
  }, [busy]);

  return <Dialog title="Open a protected PDF" busy={busy} error={message} onClose={onCancel}
    actions={<><button className="button secondary" disabled={busy} onClick={onCancel}>Cancel</button>
      <button className="button primary" type="submit" form="pdf-password-form" disabled={busy}>
        <LockKeyhole size={15} />{busy ? "Unlocking..." : "Unlock PDF"}
      </button></>}>
    <p className="password-filename"><strong>{challenge.name}</strong></p>
    <p id="pdf-password-help">Enter this PDF's password. Leave it blank if no opening password is set.
      Use the owner password if you need to change restricted content.</p>
    <form id="pdf-password-form" className="password-form" onSubmit={(event) => {
      event.preventDefault();
      if (busy) return;
      const value = password;
      setPassword("");
      onSubmit(value);
    }}>
      <label className="note-label" htmlFor="pdf-password">PDF password</label>
      <input id="pdf-password" ref={input} data-autofocus type="password" value={password}
        autoComplete="off" autoCapitalize="none" spellCheck={false} maxLength={1024}
        aria-describedby="pdf-password-help" aria-invalid={Boolean(message)} disabled={busy}
        onChange={(event) => setPassword(event.target.value)} />
    </form>
    <p className="muted">The password stays in memory for this open document. It is not stored in settings or sent to an external service.</p>
    <p className="muted">{challenge.importing
      ? "Inserted pages use the destination PDF's password protection. An encrypted source cannot be inserted into an unencrypted destination."
      : "Saved PDF copies keep the existing password protection. Image/text exports require a separate unencrypted-output acknowledgment. Your original file stays unchanged."}</p>
  </Dialog>;
}
