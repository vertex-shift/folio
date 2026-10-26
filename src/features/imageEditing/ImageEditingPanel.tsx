import { Check, Image, ImagePlus, LoaderCircle, MapPin, RefreshCw, Trash2 } from "lucide-react";
import type { DocumentInfo } from "../../lib/types";
import { draftPlacement, imageDraftPending, imageGeometryChanged, imagePath } from "./model";
import type { ImageEditDraft, ImageInspection, ImageOccurrence } from "./types";

export default function ImageEditingPanel(props: {
  info: DocumentInfo; page: number; inspection: ImageInspection; draft: ImageEditDraft | null;
  disabled: boolean; stale: boolean; error: string | null; notice: string | null;
  onSelect(page: number, image: ImageOccurrence): void; onChange(draft: ImageEditDraft): void;
  onReplacement(): void; onApply(): void; onDiscard(): void; onLocate(page: number): void;
  onTarget(): void; retry(): void;
}) {
  const { draft, inspection } = props;
  const pending = imageDraftPending(draft);
  const locked = props.disabled || props.stale || !draft?.source.editable;
  let placementError: string | null = null;
  if (draft?.source.editable && imageGeometryChanged(draft) && !draft.remove && !props.stale) {
    try { draftPlacement(draft, props.info.pages[draft.page].geometry); }
    catch (error) { placementError = error instanceof Error ? error.message : "The image placement is invalid."; }
  }
  return <div className="image-editing-panel">
    <div className="image-edit-intro">
      <p>Select an existing page image, then replace, move, resize or remove that occurrence. This changes source content, not an overlay stamp.</p>
      <button className="button secondary" disabled={props.disabled} onClick={props.onTarget}><MapPin size={15} />Select on page</button>
    </div>
    <div className="panel-scroll image-edit-settings">
      <h3>Images on page {props.page + 1}</h3>
      {inspection.loading && <p role="status"><LoaderCircle size={14} className="spin" />Inspecting source images...</p>}
      {inspection.error && <p className="image-edit-error" role="alert">{inspection.error}</p>}
      {inspection.error && <button className="text-button" onClick={props.retry} disabled={props.disabled}><RefreshCw size={14} />Retry inspection</button>}
      {inspection.result?.warnings.map((warning) => <p className="image-edit-warning" key={warning}>{warning}</p>)}
      {inspection.result && !inspection.result.images.length && <p>No Image-XObject occurrences in this page&apos;s content. Inline images and annotation or widget appearances are not selectable here.</p>}
      <div className="image-occurrence-list" aria-label={`Source images on page ${props.page + 1}`}>
        {inspection.result?.images.map((image, index) => {
          const selected = draft?.page === props.page && imagePath(draft.source.path) === imagePath(image.path);
          return <button key={imagePath(image.path)} type="button" className={`image-occurrence${selected ? " selected" : ""}`}
            aria-pressed={selected} disabled={props.disabled || pending && !selected}
            onClick={() => props.onSelect(props.page, image)}>
            <Image size={16} />
            <span><strong>Image {index + 1} · {image.name}</strong>
              <span>{image.pixelWidth ?? "?"} × {image.pixelHeight ?? "?"} px · {image.editable ? "Editable occurrence" : "Read-only"}</span>
              {!image.editable && <span>{image.reason}</span>}</span>
          </button>;
        })}
      </div>
      {draft && <>
        <div className="image-edit-context">
          <h3>Selected occurrence · page {draft.page + 1}</h3>
          <p>{draft.source.name} · operation {imagePath(draft.source.path)}</p>
          {draft.page !== props.page && <button className="text-button" onClick={() => props.onLocate(draft.page)}>Go to selected image</button>}
        </div>
        {props.stale && <p className="image-edit-warning" role="alert">The PDF changed. Your draft is kept; discard it before editing this revision.</p>}
        {!draft.source.editable && <p className="image-edit-warning" role="status">{draft.source.reason}</p>}
        {draft.remove ? <div className="image-edit-warning" role="status">
          <p>Applying removes only this image occurrence from page content. Shared occurrences stay unchanged.</p>
          <button className="text-button" disabled={locked} onClick={() => props.onChange({ ...draft, remove: false })}>Keep image</button>
        </div> : <>
          <button className="button secondary" disabled={locked} onClick={props.onReplacement}><ImagePlus size={15} />{draft.replacement ? "Choose another replacement" : "Replace with PNG or JPEG"}</button>
          {draft.replacement && <div className="image-replacement">
            <img src={draft.replacement.previewUrl} alt={`Replacement preview: ${draft.replacement.name}`} />
            <p><strong>{draft.replacement.name}</strong><br />{draft.replacement.width} × {draft.replacement.height} px · unapplied</p>
          </div>}
          <p>The selected footprint and orientation are preserved unless you edit placement. A different aspect ratio will stretch to fit.</p>
          <fieldset className="image-placement-fields" disabled={locked}>
            <legend>Placement in physical points</legend>
            <p>From the rotated visible page&apos;s top-left. 72 pt = 1 inch.</p>
            <div className="image-placement-grid">
              {(["x", "y", "width", "height"] as const).map((field) => <label className="setting-field" key={field}>
                {field === "x" ? "X (pt)" : field === "y" ? "Y (pt)" : field === "width" ? "Width (pt)" : "Height (pt)"}
                <input type="number" min={field === "x" || field === "y" ? 0 : 0.1} step="0.1"
                  value={draft[field]} onChange={(event) => props.onChange({ ...draft, [field]: event.target.value })} />
              </label>)}
            </div>
            <p>Drag the selected outline to move it; drag its corner to resize. Arrow keys move 1 pt, Shift+Arrow 10 pt. The outline previews bounds only; original pixels remain until Apply. Enclosing Form bounds are checked then.</p>
          </fieldset>
          <button className="text-button image-remove-action" disabled={locked} onClick={() => props.onChange({ ...draft, remove: true })}><Trash2 size={14} />Mark selected occurrence for removal</button>
        </>}
      </>}
      <details className="image-edit-support"><summary>Supported images and limits</summary>
        <p>Only actual raster Image-XObjects in page content, including repeated nested Forms. Fully visible, axis-aligned placements support right-angle turns and reflections. No inline images, stencil masks, optional-content layers, skew/oblique placements, source clipping paths, or page/Form transparency groups. Other unsupported graphics states are read-only with reasons.</p>
        <p>Replacement files: still PNG or 8-bit RGB/grayscale JPEG, up to 16 MiB, 24 megapixels and 8,192 px per side. Alpha and EXIF orientation are applied; source file metadata is not embedded. Image inspection is bounded to 2,000 occurrences, 200,000 operations, 16 nested Forms and 16 MiB decoded content per page.</p>
      </details>
      <p className="image-edit-privacy">Ordinary editing, not secure redaction. Old image objects, original files and undo history may still contain previous pixels or private data. Use Redact for permanent pixel removal in a separate rasterized copy.</p>
    </div>
    <div className="image-edit-commit">
      {(props.error ?? placementError) && <p className="image-edit-error" role="alert">{props.error ?? placementError}</p>}
      {props.notice && !props.error && <p className="image-edit-success" role="status"><Check size={14} />{props.notice}</p>}
      <button className={`button ${draft?.remove ? "danger" : "primary"}`} disabled={locked || !pending || Boolean(placementError)} onClick={props.onApply}>
        {draft?.remove ? <Trash2 size={15} /> : <Check size={15} />}{draft?.remove ? "Remove image occurrence" : "Apply image changes"}
      </button>
      <button className="text-button" disabled={props.disabled || !draft} onClick={props.onDiscard}>{pending ? "Discard image draft" : "Deselect image"}</button>
    </div>
  </div>;
}
