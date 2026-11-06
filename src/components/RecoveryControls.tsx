import { useEffect, useRef, useState } from "react";
import { History } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { bridge, native } from "../lib/bridge";
import type { RecoveryStatus } from "../lib/bridge";
import type { DocumentInfo } from "../lib/types";
import { errorMessage } from "../lib/geometry";
import Dialog from "./Dialog";
import IconButton from "./IconButton";

export default function RecoveryControls({ blocked, open, onOpenChange, onRecovered }: {
  blocked: boolean;
  open: boolean;
  onOpenChange(open: boolean): void;
  onRecovered(info: DocumentInfo): void;
}) {
  const [status, setStatus] = useState<RecoveryStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [startupPending, setStartupPending] = useState(true);
  const locked = useRef(false);
  const refocus = useRef(false);
  const done = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!busy && refocus.current) {
      refocus.current = false;
      done.current?.focus();
    }
  }, [busy]);

  useEffect(() => {
    let canceled = false;
    const refresh = () => {
      void bridge.recoveryStatus().then((value) => {
        if (!canceled) setStatus(value);
      }).catch((error: unknown) => { if (!canceled) setError(errorMessage(error)); });
    };
    refresh();
    const timer = window.setInterval(refresh, 5000);
    const subscription = native ? listen<RecoveryStatus>("folio-recovery", (event) => {
      if (!canceled) setStatus(event.payload);
    }) : null;
    return () => {
      canceled = true;
      window.clearInterval(timer);
      if (subscription) void subscription.then((unlisten) => unlisten()).catch(console.error);
    };
  }, []);

  useEffect(() => {
    if (!status || blocked || !startupPending) return;
    setStartupPending(false);
    if (status.entries.length) onOpenChange(true);
  }, [status, blocked, startupPending, onOpenChange]);

  async function run(action: () => Promise<void>) {
    if (locked.current) return;
    locked.current = true;
    refocus.current = true;
    setBusy(true); setError(null);
    try {
      await action();
      setConfirm(null);
      setStatus(await bridge.recoveryStatus());
    } catch (error) { setError(errorMessage(error)); }
    finally { locked.current = false; setBusy(false); }
  }

  const problem = error ?? status?.error;
  return <>
    <IconButton label="Local recovery" disabled={blocked} onClick={() => onOpenChange(true)}>
      <History size={17} />
    </IconButton>
    {!open && (problem || Boolean(status?.enabled && status.excluded.length)) &&
      <div className="recovery-warning" role={problem ? "alert" : "status"}>
        <span>{problem ? `Recovery needs attention: ${problem}` : "Some open work is not covered by local recovery."}</span>
        <button className="button secondary" disabled={blocked} onClick={() => onOpenChange(true)}>Review recovery</button>
      </div>}
    {open && <Dialog title="Local crash recovery" className="recovery-dialog" busy={busy} error={problem}
      onClose={() => { setConfirm(null); onOpenChange(false); }}
      actions={<button ref={done} className="button secondary" disabled={busy}
        onClick={() => { setConfirm(null); onOpenChange(false); }}>Done</button>}>
      <p>Keep local recovery copies of applied edits in unprotected, editable PDFs. Originals are never changed by recovery.</p>
      <p><strong>These copies contain full PDF contents, including text underneath unapplied redaction marks.</strong> They are not encrypted by Folio. Your account, device backups, and other software may be able to read them.</p>
      <p>No passwords, decrypted protected PDFs, undo history, tab layout, or unapplied work is stored. Pending writing, forms, metadata, page/paragraph/image edits, semantic recipes, redactions, OCR, and prepared exports are not recovered. A tab with pending work has its checkpoint removed until that work is applied or discarded.</p>
      <p className="muted">Checkpoints follow completed document actions, not every keystroke. A crash during an action may leave an older copy or none. This is not a backup or autosave to your source file. Limit: 16 copies, 256 MiB total. Save important work normally.</p>
      {!status ? <p role="status">Inspecting recovery storage...</p>
        : !status.available ? <p>Recovery storage is unavailable. In browser preview it requires an explicitly configured test directory. Desktop storage errors require resolving the reported problem and restarting Folio.</p>
          : <>
            <p><strong>{status.enabled ? "Recovery is on" : "Recovery is off"}</strong>
              {status.enabled && ` · ${status.checkpointed} open PDF checkpoint${status.checkpointed === 1 ? "" : "s"}`}</p>
            <p className="recovery-location">Storage: {status.location}</p>
            <button className="button secondary" disabled={busy} onClick={() => {
              if (status.enabled) setConfirm("disable");
              else void run(() => bridge.setRecoveryEnabled(true));
            }}>{status.enabled ? "Turn off & delete recovery copies" : "Enable local recovery"}</button>
            {status.excluded.length > 0 && <div role="status"><h3>Not covered</h3>
              <ul>{status.excluded.map((name) => <li key={name}>{name}</li>)}</ul></div>}
            <h3>Copies from a previous session</h3>
            <p>Inspect the name, time, and size before recovering. Recovery opens a new unsaved tab with no undo history; save it to a new file. Unapplied work is always absent.</p>
            {!status.entries.length ? <p className="muted">No previous recovery copies.</p>
              : <ul className="recovery-list">{status.entries.map((entry) => <li key={entry.token}>
                <strong>{entry.name}</strong>
                <span>{new Date(entry.modified * 1000).toLocaleString()} · {(entry.byteLength / 1024).toFixed(1)} KiB</span>
                {entry.problem && <p className="inline-error">{entry.problem}</p>}
                <div className="recovery-actions">
                  <button className="button primary" disabled={busy || Boolean(entry.problem)}
                    onClick={() => { void run(async () => onRecovered(await bridge.recoverDocument(entry.token))); }}>Recover in new tab</button>
                  <button className="button text-danger" disabled={busy} onClick={() => setConfirm(entry.token)}>Discard copy</button>
                </div>
              </li>)}</ul>}
            {confirm && <div className="recovery-confirm" role="group" aria-label="Confirm recovery deletion">
              <p>{confirm === "disable" ? "Turn off recovery and delete all local recovery copies? Open PDFs and source files stay unchanged."
                : "Permanently discard this recovery copy? This cannot be undone. Source files stay unchanged."}</p>
              <button className="button secondary" disabled={busy}
                onClick={() => { setConfirm(null); done.current?.focus(); }}>Cancel deletion</button>
              <button className="button text-danger" disabled={busy} onClick={() => {
                void run(() => confirm === "disable" ? bridge.setRecoveryEnabled(false) : bridge.discardRecovery(confirm));
              }}>Confirm deletion</button>
            </div>}
            <p className="muted">Saving, closing, or explicitly discarding an open tab removes its checkpoint. Normal quit removes this session&apos;s checkpoints. Previous-session copies stay until recovered and saved/closed, discarded, or recovery is turned off. Deletion is not secure erasure; backups may retain copies.</p>
          </>}
    </Dialog>}
  </>;
}
