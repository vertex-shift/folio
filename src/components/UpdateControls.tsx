import { useEffect, useRef, useState } from "react";
import { ArrowDownToLine, LoaderCircle, X } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { native } from "../lib/bridge";
import { errorMessage } from "../lib/geometry";
import { canCheckForUpdates, downloadProgress, updateSummary } from "../lib/updates";
import type { UpdateStatus } from "../lib/updates";
import IconButton from "./IconButton";

export function UpdatePanel({ status, error, onCheck, onClose }: {
  status: UpdateStatus | null;
  error: string | null;
  onCheck(): void;
  onClose(): void;
}) {
  return <section className="update-panel" aria-label="App updates">
    <div className="update-panel-heading"><h2>App updates</h2>
      <IconButton label="Close app updates" onClick={onClose}><X size={16} /></IconButton></div>
    <p role="status">{status ? updateSummary(status) : "Reading update status..."}</p>
    {status?.phase === "downloading" && <progress aria-label="Update download" max={1} value={downloadProgress(status)} />}
    {error && <p className="inline-error" role="alert">{error}</p>}
    <p className="muted">Updates download while you work. Installation waits until you quit and finish reviewing unsaved PDFs. Folio won&apos;t restart itself.</p>
    <p className="muted">Checks contact GitHub for public release information. No PDFs, document names, passwords, or recovery copies are sent.</p>
    <button className="button secondary" disabled={!status || !canCheckForUpdates(status)} onClick={onCheck}>
      {status?.phase === "checking" && <LoaderCircle size={14} className="spin" />}Check for updates</button>
  </section>;
}

export default function UpdateControls({ open, onOpenChange, disabled = false }: {
  open: boolean;
  onOpenChange(open: boolean): void;
  disabled?: boolean;
}) {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const container = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (disabled && open) onOpenChange(false);
  }, [disabled, open, onOpenChange]);

  useEffect(() => {
    if (!native) return;
    let canceled = false;
    const refresh = () => {
      void invoke<UpdateStatus>("update_status").then((value) => {
        if (!canceled) { setStatus(value); setError(null); }
      }).catch((error: unknown) => { if (!canceled) setError(errorMessage(error)); });
    };
    const subscription = listen<UpdateStatus>("folio-update", (event) => {
      if (!canceled) setStatus(event.payload);
    });
    refresh();
    const timer = window.setInterval(refresh, 5000);
    return () => {
      canceled = true;
      window.clearInterval(timer);
      void subscription.then((dispose) => dispose()).catch(console.error);
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    const dismiss = (event: PointerEvent) => {
      if (event.target instanceof Node && !container.current?.contains(event.target)) onOpenChange(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") { onOpenChange(false); trigger.current?.focus(); }
    };
    window.addEventListener("pointerdown", dismiss);
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("pointerdown", dismiss);
      window.removeEventListener("keydown", escape);
    };
  }, [open, onOpenChange]);

  if (!native) return null;
  const attention = status?.phase === "ready" || status?.phase === "error" || Boolean(error);
  return <div ref={container} className="update-controls">
    <button ref={trigger} className={`icon-button${attention ? " active" : ""}`}
      title={status ? updateSummary(status) : "App updates"} aria-label="App updates"
      aria-expanded={open} disabled={disabled} onClick={() => onOpenChange(!open)}>
      <ArrowDownToLine size={17} />{attention && <span className="update-dot" />}
    </button>
    {open && <UpdatePanel status={status} error={error} onClose={() => {
      onOpenChange(false); trigger.current?.focus();
    }} onCheck={() => {
      setError(null);
      void invoke("check_for_updates").then(() => invoke<UpdateStatus>("update_status")).then(setStatus)
        .catch((error: unknown) => setError(errorMessage(error)));
    }} />}
  </div>;
}
