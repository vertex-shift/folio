import { useState } from "react";
import { ArrowDown, ArrowUp, Bookmark, ChevronRight, IndentDecrease, IndentIncrease, LoaderCircle, Pencil, Plus, Trash2 } from "lucide-react";
import { documentUtilities } from "../lib/documentUtilitiesBridge";
import { bookmarkMove } from "../lib/documentUtilitiesModel";
import type { BookmarkInfo } from "../lib/documentUtilitiesModel";
import { useDocumentUtility } from "../lib/useDocumentUtility";
import type { DocumentInfo } from "../lib/types";
import IconButton from "./IconButton";

export default function BookmarksPanel({ info, page, disabled, editDisabled, onNavigate, onAdd, onRename, onRemove, onMove }: {
  info: DocumentInfo | null;
  page: number;
  disabled: boolean;
  editDisabled: boolean;
  onNavigate(destination: unknown, url: string | null): void;
  onAdd(entries: BookmarkInfo[], parent: string | null): void;
  onRename(entry: BookmarkInfo): void;
  onRemove(entry: BookmarkInfo): void;
  onMove(entry: BookmarkInfo, parent: string | null, at: number): void;
}) {
  const resource = useDocumentUtility(info, documentUtilities.bookmarks);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const entries = resource.value ?? [];
  const selected = entries.find((entry) => entry.id === selectedId);
  return <div className="document-utility-panel">
    <div className="utility-panel-intro">
      <button className="button secondary" disabled={editDisabled || resource.loading || Boolean(resource.error)}
        onClick={() => onAdd(entries, null)}><Plus size={15} />Bookmark page {page + 1}</button>
      <p>Create real PDF destinations. Select a bookmark to rename or organize it.</p>
    </div>
    {selected && <div className="bookmark-editor-controls">
      <p className="bookmark-selection">{selected.title || "Untitled bookmark"}</p>
      <div className="utility-actions">
        <button className="text-button" disabled={editDisabled} onClick={() => onRename(selected)}><Pencil size={13} />Rename</button>
        <button className="text-button" disabled={editDisabled} onClick={() => onAdd(entries, selected.id)}><Plus size={13} />Add child</button>
        <IconButton label="Delete selected bookmark and descendants" disabled={editDisabled} onClick={() => onRemove(selected)}><Trash2 size={14} /></IconButton>
      </div>
      <div className="utility-actions" role="group" aria-label="Organize selected bookmark">
        {([
          ["up", "Move bookmark up", ArrowUp], ["down", "Move bookmark down", ArrowDown],
          ["in", "Nest under previous sibling", IndentIncrease], ["out", "Move out one level", IndentDecrease],
        ] as const).map(([direction, label, Icon]) => {
          const move = bookmarkMove(entries, selected, direction);
          return <IconButton key={direction} label={label} disabled={editDisabled || !move} onClick={() => {
            if (move) onMove(selected, move.parent, move.at);
          }}><Icon size={15} /></IconButton>;
        })}
      </div>
    </div>}
    <div className="panel-scroll bookmarks-panel" role="list" aria-label="PDF bookmarks">
      {resource.loading && <p className="utility-progress" role="status"><LoaderCircle size={14} className="spin" />Reading bookmarks...</p>}
      {resource.error && <div className="utility-error" role="alert"><p>{resource.error}</p><button className="text-button" onClick={resource.retry}>Retry</button></div>}
      {!resource.loading && !entries.length && !resource.error && <div className="panel-empty"><Bookmark size={28} /><h3>No bookmarks yet</h3><p>Bookmark the current page to create an outline for this PDF.</p></div>}
      {entries.map((entry) => <div role="listitem" key={entry.id}>
        <button className={`bookmark-entry ${selectedId === entry.id ? "selected" : ""}`}
          style={{ paddingLeft: 16 + Math.min(entry.depth, 8) * 12 }} disabled={disabled}
          aria-pressed={selectedId === entry.id}
          onClick={() => {
            setSelectedId(entry.id);
            if (entry.url) onNavigate(null, entry.url);
            else if (entry.page !== null) onNavigate([entry.page, { name: "Fit" }], null);
            else if (entry.namedDestination !== null) onNavigate(entry.namedDestination, null);
          }}>
          <Bookmark size={14} /><span>{entry.title || "Untitled bookmark"}<small>{entry.targetDescription}</small></span><ChevronRight size={13} />
        </button>
      </div>)}
    </div>
    <div className="utility-footer"><p>Other bookmark actions, destination details, styling, and open/closed groups are preserved. Undo restores removed groups.</p></div>
  </div>;
}
