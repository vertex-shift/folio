import { Crop, Hash, ImagePlus, RotateCcw, Stamp, Trash2, Upload, X } from "lucide-react";
import IconButton from "./IconButton";
import type { AuthoringAction, AuthoringDraft, AuthoringKind, NumberPosition, PageGeometry } from "../lib/authoring";
import { authoringPending, fittedImage, imageDimension } from "../lib/authoring";

interface Props {
  draft: AuthoringDraft;
  geometry: PageGeometry;
  currentPage: number;
  pageCount: number;
  disabled: boolean;
  error: string | null;
  notice: string | null;
  onDismissError(): void;
  onChange(draft: AuthoringDraft): void;
  onKind(kind: AuthoringKind): void;
  onImage(): void;
  onApply(action: AuthoringAction): void;
  onDiscard(): void;
  onDraw(): void;
}

function NumberField(props: {
  label: string; value: string; min?: number; max?: number; step?: number | "any";
  onChange(value: string): void;
}) {
  return <label className="authoring-field">{props.label}<input type="number"
    min={props.min} max={props.max} step={props.step ?? "any"} value={props.value}
    onChange={(event) => props.onChange(event.target.value)} /></label>;
}

const sections = [
  { kind: "image", label: "Image", icon: ImagePlus },
  { kind: "crop", label: "Crop", icon: Crop },
  { kind: "watermark", label: "Watermark", icon: Stamp },
  { kind: "numbers", label: "Page numbers", icon: Hash },
] as const;

const positions: { value: NumberPosition; label: string }[] = [
  { value: "top_left", label: "Top left" }, { value: "top_center", label: "Top center" },
  { value: "top_right", label: "Top right" }, { value: "bottom_left", label: "Bottom left" },
  { value: "bottom_center", label: "Bottom center" }, { value: "bottom_right", label: "Bottom right" },
];

export default function AuthoringPanel(props: Props) {
  const draft = props.draft;
  const pending = authoringPending(draft);
  const change = (next: AuthoringDraft) => props.onChange({ ...next, dirty: true });
  const applyLabel = draft.kind === "image" ? "Insert image" : draft.kind === "crop" ? "Apply crop"
    : draft.kind === "watermark" ? "Apply watermark" : "Apply page numbers";
  return <div className="authoring-panel">
    <div className="authoring-sections" role="group" aria-label="Page authoring tool">
      {sections.map(({ kind, label, icon: Icon }) => <button key={kind} aria-pressed={draft.kind === kind}
        className={draft.kind === kind ? "selected" : ""} disabled={props.disabled || pending && draft.kind !== kind}
        onClick={() => props.onKind(kind)}><Icon size={16} />{label}</button>)}
    </div>
    {props.error && <div className="authoring-error" role="alert"><span>{props.error}</span>
      <IconButton label="Dismiss authoring error" onClick={props.onDismissError}><X size={14} /></IconButton></div>}
    <div className="panel-scroll authoring-settings">
      <p className="authoring-page-summary">Page {draft.page + 1} · {Math.round(props.geometry.widthPoints)} × {Math.round(props.geometry.heightPoints)} pt</p>
      <fieldset disabled={props.disabled}>
        {draft.kind !== "image" && <div className="authoring-range">
          <label className="setting-field">Apply to<select aria-label="Authoring page scope" value={draft.scope}
            onChange={(event) => {
              const scope = event.target.value;
              if (scope === "current" || scope === "all" || scope === "range") change({ ...draft, scope });
            }}>
            <option value="current">Page {draft.page + 1}</option><option value="all">All {props.pageCount} pages</option>
            <option value="range">Page range...</option>
          </select></label>
          {draft.scope === "range" && <label className="authoring-field">Pages, in this order<input type="text"
            aria-label="Authoring page range" value={draft.range} maxLength={4000} placeholder="1-3, 5"
            onChange={(event) => change({ ...draft, range: event.target.value })} /></label>}
        </div>}
        {props.currentPage !== draft.page && <button className="text-button authoring-retarget"
          onClick={() => change({ ...draft, page: props.currentPage })}>Use visible page {props.currentPage + 1}</button>}
        {draft.kind === "image" && <>
          <p>Add a new PNG or JPEG. Click a visible page to position it, or enter precise dimensions below.</p>
          <button className="button secondary authoring-wide" onClick={props.onImage}><Upload size={15} />{draft.asset ? "Choose another image" : "Choose PNG or JPEG"}</button>
          {draft.asset && <>
            <div className="authoring-image-preview">
              {draft.previewUrl && <img src={draft.previewUrl} alt="Selected image preview" />}
              <span>{draft.name}</span><span>{draft.asset.width} × {draft.asset.height} pixels</span>
            </div>
            <div className="authoring-fields">
              <NumberField label="Left (pt)" min={0} value={draft.x} onChange={(x) => change({ ...draft, x })} />
              <NumberField label="Top (pt)" min={0} value={draft.y} onChange={(y) => change({ ...draft, y })} />
              <NumberField label="Image width (pt)" min={0.1} value={draft.width} onChange={(value) => change(imageDimension(draft, "width", value))} />
              <NumberField label="Image height (pt)" min={0.1} value={draft.height} onChange={(value) => change(imageDimension(draft, "height", value))} />
            </div>
            <label className="checkbox-setting"><input type="checkbox" checked={draft.lockAspect}
              onChange={(event) => change({ ...draft, lockAspect: event.target.checked })} />Keep original proportions</label>
            <button className="text-button" onClick={() => change(fittedImage(draft, props.geometry))}>Fit and center image</button>
            <label className="setting-field">Opacity (%)<input aria-label="Image opacity (%)" type="number" min={1} max={100}
              value={draft.opacity} onChange={(event) => change({ ...draft, opacity: event.target.value })} /></label>
            <button className="button secondary authoring-on-page" onClick={props.onDraw}>Position on page</button>
          </>}
          <p className="authoring-note">16 MiB encoded, 24 megapixels, 8,192 px per side. Still PNG and 8-bit RGB/grayscale JPEG only. Photo orientation is applied; source EXIF/location metadata is not embedded.</p>
          <p className="authoring-note">This inserts a portable image stamp, not an editor for images already in the PDF. Remove inserted images in Comments or Undo.</p>
        </>}
        {draft.kind === "crop" && <>
          <p>Drag the area to keep, or trim these margins from each chosen page&apos;s current visible edges.</p>
          <div className="authoring-fields">
            {(["Top", "Right", "Bottom", "Left"] as const).map((label, index) =>
              <NumberField key={label} label={`${label} margin (pt)`} min={0} value={draft.margins[index]}
                onChange={(value) => {
                  const margins: [string, string, string, string] = [...draft.margins];
                  margins[index] = value; change({ ...draft, margins });
                }} />)}
          </div>
          <button className="button secondary authoring-on-page" onClick={props.onDraw}><Crop size={15} />Draw crop on page</button>
          <p className="authoring-warning"><strong>Crop hides; it does not delete.</strong> Content outside the CropBox remains recoverable. Use Redact a PDF for permanent removal.</p>
          <div className="authoring-secondary-actions">
            <button className="button secondary" onClick={() => props.onApply("remove_crop")}>Show full MediaBox</button>
            <button className="button secondary" onClick={() => props.onApply("reset_crop")}><RotateCcw size={14} />Reset original crop</button>
          </div>
          <p className="authoring-note">Reset restores the view from before Folio&apos;s first crop change, including after save/reopen. Page sizes, rotation, and physical scale are respected individually.</p>
        </>}
        {draft.kind === "watermark" && <>
          <p>Center a single line on each chosen page. Applying again replaces only Folio watermarks on those pages.</p>
          <label className="authoring-field">Watermark text<input type="text" maxLength={160} value={draft.text}
            placeholder="For review" onChange={(event) => change({ ...draft, text: event.target.value })} /></label>
          <div className="authoring-fields">
            <NumberField label="Font size (pt)" min={6} max={144} value={draft.size} onChange={(size) => change({ ...draft, size })} />
            <NumberField label="Angle (degrees)" min={-90} max={90} value={draft.angle} onChange={(angle) => change({ ...draft, angle })} />
            <NumberField label="Opacity (%)" min={1} max={100} value={draft.opacity} onChange={(opacity) => change({ ...draft, opacity })} />
            <label className="authoring-field">Text color<input type="color" value={draft.color}
              onChange={(event) => change({ ...draft, color: event.target.value })} /></label>
          </div>
          <p className="authoring-note">Embedded Noto Sans supports Latin, Greek, and Cyrillic. Reduce text or size if it does not fit. Positive angles rise from left to right.</p>
          <button className="button secondary authoring-wide" onClick={() => props.onApply("remove_stamps")}><Trash2 size={14} />Remove Folio watermarks</button>
        </>}
        {draft.kind === "numbers" && <>
          <p>Number chosen pages sequentially, in the entered range order. The first chosen page receives the starting number.</p>
          <div className="authoring-fields">
            <NumberField label="Starting number" min={0} max={999999} step={1} value={draft.start} onChange={(start) => change({ ...draft, start })} />
            <NumberField label="Font size (pt)" min={6} max={144} value={draft.size} onChange={(size) => change({ ...draft, size })} />
          </div>
          <label className="authoring-field">Prefix<input type="text" maxLength={80} value={draft.prefix}
            placeholder="Page " onChange={(event) => change({ ...draft, prefix: event.target.value })} /></label>
          <label className="authoring-field">Suffix<input type="text" maxLength={80} value={draft.suffix}
            placeholder=" of 10" onChange={(event) => change({ ...draft, suffix: event.target.value })} /></label>
          <label className="setting-field">Position<select value={draft.position} onChange={(event) => {
            const position = positions.find((entry) => entry.value === event.target.value)?.value;
            if (position) change({ ...draft, position });
          }}>{positions.map(({ value, label }) => <option value={value} key={value}>{label}</option>)}</select></label>
          <div className="authoring-fields">
            <NumberField label="Edge margin (pt)" min={0} max={720} value={draft.margin} onChange={(margin) => change({ ...draft, margin })} />
            <label className="authoring-field">Text color<input type="color" value={draft.color}
              onChange={(event) => change({ ...draft, color: event.target.value })} /></label>
          </div>
          <p className="authoring-note">Example: {draft.prefix}{draft.start}{draft.suffix}. Edge margin measures to the text box. Numbers stay fixed after page reordering; apply again to renumber. PDF page labels are unchanged.</p>
          <button className="button secondary authoring-wide" onClick={() => props.onApply("remove_stamps")}><Trash2 size={14} />Remove Folio page numbers</button>
        </>}
        {(draft.kind === "watermark" || draft.kind === "numbers") && <p className="authoring-warning">Ordinary PDF annotations: editable and removable, not tamper-proof security. No protection is added.</p>}
        <p className="authoring-note">Measurements use physical points (72 pt = 1 inch), from the rotated visible top-left. The PDF itself keeps its original coordinate system.</p>
      </fieldset>
    </div>
    <div className="authoring-commit">
      {props.notice && !pending && <p className="authoring-success" role="status">{props.notice}</p>}
      {pending && <p role="status">Unapplied draft. Apply or discard before saving or changing this PDF.</p>}
      <button className="button primary authoring-wide" disabled={props.disabled || draft.kind === "image" && !draft.asset}
        onClick={() => props.onApply("apply")}>{applyLabel}</button>
      {pending && <button className="text-button authoring-wide" disabled={props.disabled} onClick={props.onDiscard}>Discard authoring draft</button>}
    </div>
  </div>;
}
