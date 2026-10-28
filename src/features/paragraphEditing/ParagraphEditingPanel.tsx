import { Check, LoaderCircle, MapPin, RefreshCw, SquarePen } from "lucide-react";
import { paragraphDraftPending, paragraphPath } from "./model";
import ParagraphPreviewCanvas from "./ParagraphPreviewCanvas";
import type { ParagraphBlock, ParagraphDraft, ParagraphInspection, ParagraphPreviewState } from "./types";

export default function ParagraphEditingPanel(props: {
  active: boolean; page: number; inspection: ParagraphInspection; draft: ParagraphDraft | null;
  preview: ParagraphPreviewState; disabled: boolean; stale: boolean; error: string | null; notice: string | null;
  onSelect(page: number, block: ParagraphBlock): void; onChange(draft: ParagraphDraft): void;
  onApply(): void; onDiscard(): void; onLocate(page: number): void; onTarget(): void; retry(): void;
}) {
  const { draft, inspection, preview } = props;
  const pending = paragraphDraftPending(draft);
  const locked = props.disabled || props.stale || !draft?.source.editable;
  const editor = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!props.active || !draft) return;
    editor.current?.scrollIntoView({ block: "start" });
    if (draft.source.editable) editor.current?.querySelector("textarea")?.focus({ preventScroll: true });
  }, [props.active, draft?.page, draft?.source.id, draft?.source.path.join(".")]);
  return <div className="paragraph-editing-panel">
    <div className="paragraph-intro">
      <p>Edit a source paragraph and wrap its words inside an adjustable box. Apply replaces real PDF text, not a cover or annotation.</p>
      <button className="button secondary" disabled={props.disabled} onClick={props.onTarget}><MapPin size={15} />Select paragraph on page</button>
    </div>
    <div className="panel-scroll paragraph-settings">
      <h3>Paragraphs on page {props.page + 1}</h3>
      {inspection.loading && <p role="status"><LoaderCircle size={14} className="spin" />Inspecting source text objects...</p>}
      {inspection.error && <p className="paragraph-error" role="alert">{inspection.error}</p>}
      {inspection.error && <button className="text-button" disabled={props.disabled} onClick={props.retry}><RefreshCw size={14} />Retry inspection</button>}
      {inspection.result && !inspection.result.blocks.length && <div className="paragraph-empty">
        <SquarePen size={24} /><h3>No paragraph source objects</h3>
        <p>This page has no supported BT/ET page-content blocks. Scans, outlined letters and annotation appearances are not paragraph text.</p>
      </div>}
      <div className="paragraph-source-list" aria-label={`Source paragraphs on page ${props.page + 1}`}>
        {inspection.result?.blocks.map((block, index) => {
          const selected = draft?.page === props.page && paragraphPath(draft.source.path) === paragraphPath(block.path);
          return <button type="button" className={`paragraph-source${selected ? " selected" : ""}`} key={`${block.id}:${paragraphPath(block.path)}`}
            aria-pressed={selected} disabled={props.disabled || pending && !selected}
            aria-label={`Paragraph ${index + 1}: ${block.text?.trim().slice(0, 80) || "Unmapped or empty source"}`}
            onClick={() => props.onSelect(props.page, block)}>
            <strong>Paragraph {index + 1}<span>{block.sourceLines.length} source {block.sourceLines.length === 1 ? "line" : "lines"}</span></strong>
            <span className="paragraph-source-excerpt">{block.text?.trim() || (block.text === null ? "Unmapped text" : "Empty logical paragraph")}</span>
            <span>{block.font.replace(/^[A-Z]{6}\+/, "")} · {block.editable ? "Editable source block" : "Read-only"}</span>
            {!block.editable && <span className="paragraph-source-reason">{block.reason}</span>}
          </button>;
        })}
      </div>
      {draft && <div className="paragraph-editor" ref={editor}>
        <h3>Selected paragraph · page {draft.page + 1}</h3>
        {draft.page !== props.page && <button className="text-button" onClick={() => props.onLocate(draft.page)}>Go to selected paragraph</button>}
        {props.stale && <p className="paragraph-warning" role="alert">The source revision or epoch changed. Your entire draft is kept; discard it before editing this revision.</p>}
        {!draft.source.editable && <p className="paragraph-warning" role="status">{draft.source.reason}</p>}
        <label className="setting-field paragraph-text-field">Paragraph text
          <textarea value={draft.text} rows={7} maxLength={32_000} disabled={locked}
            spellCheck={false} onChange={(event) => props.onChange({ ...draft, text: event.target.value })} />
        </label>
        <p>Source visual line endings become wrapping spaces. Enter preserves an explicit break; a blank line preserves a paragraph break. Spaces are retained, never silently trimmed.</p>
        <label className="setting-field">Paragraph font
          <select value={draft.font} disabled={locked} onChange={(event) => props.onChange({ ...draft, font: event.target.value === "original" ? "original" : "noto" })}>
            <option value="original" disabled={!draft.source.originalFontAvailable}>Original font{draft.source.originalFontAvailable ? "" : " - unavailable metrics"}</option>
            <option value="noto">Convert to Noto Sans Regular</option>
          </select>
        </label>
        {draft.font === "noto" ? <p className="paragraph-font-note">Explicit Noto Sans conversion: Latin, Greek and Cyrillic. Original weight, tracking and kerning are not preserved. Combining marks and complex shaping are refused; no remote fonts.</p>
          : <p className="paragraph-font-note">Uses the original embedded font and exact encoded glyph metrics. Reflow resets source tracking and kerning. New characters must exist in its encoding; otherwise choose Noto Sans.</p>}
        {draft.source.fontReason && <details className="paragraph-font-detail"><summary>Original-font limitation</summary><p>{draft.source.fontReason}</p></details>}
        <fieldset className="paragraph-fields" disabled={locked}>
          <legend>Physical box and line layout</legend>
          <p>From the rotated visible page&apos;s top-left. 72 pt = 1 inch. Source orientation: {draft.source.rotation} degrees.</p>
          <div className="paragraph-field-grid">
            {(["x", "y", "width", "height", "size", "leading"] as const).map((field) => <label className="setting-field" key={field}>
              {({ x: "X (pt)", y: "Y (pt)", width: "Width (pt)", height: "Height (pt)", size: "Text size (pt)", leading: "Leading (pt)" })[field]}
              <input type="number" step="0.1" value={draft[field]}
                min={field === "x" || field === "y" ? 0 : field === "size" ? 2 : field === "leading" ? 1 : 0.1}
                max={field === "size" ? 144 : field === "leading" ? 720 : undefined}
                onChange={(event) => props.onChange({ ...draft, [field]: event.target.value })} />
            </label>)}
          </div>
          <label className="setting-field">Line alignment
            <select value={draft.alignment} onChange={(event) => props.onChange({ ...draft,
              alignment: event.target.value === "center" ? "center" : event.target.value === "right" ? "right" : "left" })}>
              <option value="left">Left</option><option value="center">Center</option><option value="right">Right</option>
            </select>
          </label>
          <p>Drag the selected box or resize its corner. Arrow keys move 1 pt; Shift+Arrow moves 10 pt. The page shows temporary box and ink bounds; source text remains until Apply.</p>
        </fieldset>
        <div className="paragraph-layout-review" aria-label="Paragraph line layout">
          <h3>Actual PDF line preview</h3>
          {preview.loading && <p role="status"><LoaderCircle size={14} className="spin" />Measuring glyphs and laying out words...</p>}
          {preview.error && <p className="paragraph-error" role="alert">{preview.error}</p>}
          {preview.result && <>
            <p role="status" className={preview.result.layout.fits ? "paragraph-layout-ready" : "paragraph-error"}>
              {preview.result.layout.lines.length} {preview.result.layout.lines.length === 1 ? "line" : "lines"} · {preview.result.layout.usedWidth.toFixed(1)} × {preview.result.layout.usedHeight.toFixed(1)} pt
              {preview.result.layout.fits ? " · Fits box" : " · Overflow - cannot apply"}
            </p>
            {preview.result.layout.overflow && <p className="paragraph-error" role="alert">{preview.result.layout.overflow}</p>}
            {preview.result.renderPdf ? props.active && <ParagraphPreviewCanvas bytes={preview.result.renderPdf} />
              : <p>The complete preview exceeds 20,000 pt on one side. Widen the box or shorten the text; no source content has changed.</p>}
            <p>The blue outline is the target box. This isolated PDF preview uses actual saved-font glyphs and ink bounds, without neighboring page graphics.</p>
          </>}
        </div>
      </div>}
      <details className="paragraph-support"><summary>Supported source blocks and limits</summary>
        {inspection.result?.warnings.map((warning) => <p key={warning}>{warning}</p>)}
        <p>Uniform single-column BT/ET objects support Tm, Td, TD, T*, TJ and quote operators, including isolated repeated/nested Forms. Ambiguous columns, mixed fonts/sizes/colors, clipped or stretched text, transparency, inline-image streams, tags, ActualText and optional content are read-only with explicit reasons.</p>
        <p>No automatic font shrinking, hyphen insertion, movement of neighboring text or graphics, or continuation onto another page. Maximum 8,000 characters, 500 lines and a current PDF within the 64 MiB undo budget. An overlong word is reported, not split.</p>
      </details>
      <p className="paragraph-privacy">Ordinary source editing, not secure redaction. Original files, old objects and undo history may retain previous text. Use Redact for permanent removal in a separate raster copy.</p>
    </div>
    <div className="paragraph-commit">
      {props.error && <p className="paragraph-error" role="alert">{props.error}</p>}
      {props.notice && !props.error && <p className="paragraph-success" role="status">{props.notice}</p>}
      <button className="button primary" disabled={locked || !pending || preview.loading || !preview.result?.layout.fits || Boolean(preview.error)}
        onClick={props.onApply}><Check size={15} />Apply paragraph reflow</button>
      <button className="text-button" disabled={props.disabled || !draft} onClick={props.onDiscard}>{pending ? "Discard paragraph draft" : "Deselect paragraph"}</button>
    </div>
  </div>;
}
import { useEffect, useRef } from "react";
