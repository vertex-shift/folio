import { Hash, Plus, Trash2 } from "lucide-react";
import Dialog from "./Dialog";
import { PAGE_LABEL_STYLES, pageLabelsPending } from "../lib/pageLabels";
import type { PageLabelDraft, PageLabelDraftRange } from "../lib/pageLabels";
import "./PageLabelsDialog.css";

export default function PageLabelsDialog({ draft, pageCount, issue, busy, error, onChange, onApply, onReset, onClose }: {
  draft: PageLabelDraft;
  pageCount: number;
  issue: string | null;
  busy: boolean;
  error: string | null;
  onChange(draft: PageLabelDraft): void;
  onApply(): void;
  onReset(): void;
  onClose(): void;
}) {
  const change = (index: number, update: Partial<PageLabelDraftRange>) => onChange({
    ...draft, ranges: draft.ranges.map((range, position) => position === index ? { ...range, ...update } : range),
  });
  return <Dialog title="PDF page labels" className="page-label-dialog" busy={busy} error={error} onClose={onClose}
    actions={<>
      <button className="button text-danger" disabled={busy || !draft.custom} onClick={onReset}>Reset to page positions</button>
      <button className="button secondary" disabled={busy} onClick={onClose}>Cancel</button>
      <button className="button primary" disabled={busy || !pageLabelsPending(draft)} onClick={onApply}><Hash size={15} />Apply labels</button>
    </>}>
    <p>Number front matter, restart sections, or add prefixes. These are navigation labels, not text stamped on the PDF pages.</p>
    <p className="muted">Start positions and all export/OCR page ranges use physical pages 1 through {pageCount}. Labels follow existing pages when reordered or extracted; duplicate labels require Page mode for unambiguous navigation.</p>
    {issue && <p className="inline-error" role="status">{issue} Applying a new definition replaces the old label tree.</p>}
    <div className="page-label-ranges">
      {draft.ranges.map((range, index) => <fieldset className="page-label-range" key={index}>
        <legend>Numbering section {index + 1}</legend>
        <div className="page-label-fields">
          <label>Start at physical page<input data-autofocus={index === 0 || undefined} type="text" inputMode="numeric"
            aria-label={`Section ${index + 1} page`} value={range.page} disabled={busy}
            onChange={(event) => change(index, { page: event.target.value })} /></label>
          <label>Numbering style<select aria-label={`Section ${index + 1} style`} value={range.style} disabled={busy}
            onChange={(event) => {
              const style = PAGE_LABEL_STYLES.find((style) => style.id === event.target.value);
              if (style) change(index, { style: style.id });
            }}>
            {PAGE_LABEL_STYLES.map((style) => <option key={style.id} value={style.id}>{style.name}</option>)}
          </select></label>
          <label>Prefix<input type="text" maxLength={512} aria-label={`Section ${index + 1} prefix`} value={range.prefix}
            disabled={busy} onChange={(event) => change(index, { prefix: event.target.value })} /></label>
          <label>Starting number<input type="text" inputMode="numeric" aria-label={`Section ${index + 1} starting number`}
            value={range.start} disabled={busy || range.style === "prefix"}
            onChange={(event) => change(index, { start: event.target.value })} /></label>
        </div>
        {index > 0 && <button className="button text-danger" disabled={busy} aria-label={`Remove section ${index + 1}`}
          onClick={() => onChange({ ...draft, ranges: draft.ranges.filter((_, position) => position !== index) })}>
          <Trash2 size={14} />Remove section
        </button>}
      </fieldset>)}
    </div>
    <button className="button secondary" disabled={busy || draft.ranges.length >= 256} onClick={() => onChange({
      ...draft, ranges: [...draft.ranges, { page: "", style: "decimal", prefix: "", start: "1" }],
    })}><Plus size={15} />Add numbering section</button>
    <p className="muted">Apply is one undoable change. Reset removes label metadata only; no page content is erased. Cancel discards this draft.</p>
  </Dialog>;
}
