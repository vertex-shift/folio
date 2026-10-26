import { useEffect, useRef } from "react";
import { FileText, X } from "lucide-react";
import { adjacentTab } from "../lib/tabs";
import type { DocumentInfo } from "../lib/types";
import SelectionIndicator from "./SelectionIndicator";

export default function DocumentTabs({
  documents, activeId, pending, disabled, onSelect, onClose,
}: {
  documents: DocumentInfo[];
  activeId: number | null;
  pending: ReadonlySet<number>;
  disabled: boolean;
  onSelect(id: number): void;
  onClose(id: number): void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const focusTarget = useRef<number | null>(null);
  useEffect(() => {
    const container = ref.current;
    if (!container) return;
    const reveal = () => container.querySelector<HTMLElement>('[aria-selected="true"]')
      ?.parentElement?.scrollIntoView({ block: "nearest", inline: "nearest" });
    const selected = container.querySelector<HTMLElement>('[aria-selected="true"]');
    reveal();
    if (!disabled && focusTarget.current === activeId) {
      selected?.focus();
      focusTarget.current = null;
    }
    const observer = new ResizeObserver(reveal);
    observer.observe(container);
    return () => observer.disconnect();
  }, [activeId, documents.length, disabled]);

  return <div className="document-tabs" role="tablist" aria-label="Open PDFs" ref={ref}>
    <SelectionIndicator kind="tabs" selectionKey={`${activeId}:${documents.map((document) => document.id).join(",")}`} />
    {documents.map((document) => <div className={`document-tab ${activeId === document.id ? "selected" : ""}`}
      role="presentation" key={document.id}>
      <button className="document-tab-select" role="tab" id={`document-tab-${document.id}`}
        aria-selected={activeId === document.id} aria-controls={`document-panel-${document.id}`}
        aria-label={document.name} title={document.name} disabled={disabled}
        tabIndex={activeId === document.id ? 0 : -1} onClick={() => onSelect(document.id)}
        onKeyDown={(event) => {
          let target: number | null = null;
          const ids = documents.map((item) => item.id);
          if (event.key === "ArrowRight") target = adjacentTab(ids, document.id, 1);
          else if (event.key === "ArrowLeft") target = adjacentTab(ids, document.id, -1);
          else if (event.key === "Home") target = ids[0] ?? null;
          else if (event.key === "End") target = ids.at(-1) ?? null;
          else if (event.key === "Delete") {
            event.preventDefault(); onClose(document.id); return;
          }
          if (target !== null) {
            event.preventDefault();
            focusTarget.current = target;
            onSelect(target);
          }
        }}>
        <FileText size={15} className="tab-file-icon" />
        <span className="document-tab-name">{document.name}</span>
        {(document.dirty || pending.has(document.id)) && <span className="unsaved-dot" aria-label="Unsaved changes" />}
      </button>
      <button className="document-tab-close icon-button" aria-label={`Close ${document.name}`}
        title={`Close ${document.name}`} disabled={disabled} onClick={() => onClose(document.id)}><X size={13} /></button>
    </div>)}
  </div>;
}
