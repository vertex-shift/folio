import { useEffect, useRef } from "react";
import type { ReactNode } from "react";
import { X } from "lucide-react";

export default function Dialog({
  title, children, actions, onClose, busy, error, className,
}: {
  title: string;
  children: ReactNode;
  actions?: ReactNode;
  onClose(): void;
  busy?: boolean;
  error?: string | null;
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;

  useEffect(() => {
    const previous = document.activeElement;
    const first = ref.current?.querySelector<HTMLElement>("textarea, [data-autofocus]")
      ?? ref.current?.querySelector<HTMLElement>("button");
    first?.focus();
    return () => {
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, []);

  return <div className="modal-backdrop" onMouseDown={(event) => {
    if (event.target === event.currentTarget && !busy) onClose();
  }}>
    <div className={`dialog${className ? ` ${className}` : ""}`} ref={ref} role="dialog" aria-modal="true" aria-labelledby="dialog-title"
      onKeyDown={(event) => {
        if (event.key === "Escape" && !busy) { event.preventDefault(); close.current(); }
        if (event.key !== "Tab") return;
        const elements = Array.from(ref.current?.querySelectorAll<HTMLElement>(
          "button:not(:disabled), textarea:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex='0']",
        ) ?? []);
        const first = elements[0];
        const last = elements.at(-1);
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault(); last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault(); first?.focus();
        }
      }}
    >
      <header className="dialog-header"><h2 id="dialog-title">{title}</h2>
        <button className="icon-button" aria-label="Close dialog" title="Close dialog" disabled={busy} onClick={onClose}><X size={18} /></button>
      </header>
      <div className="dialog-body">{children}{error && <p className="inline-error" role="alert">{error}</p>}</div>
      {actions && <footer className="dialog-actions">{actions}</footer>}
    </div>
  </div>;
}
