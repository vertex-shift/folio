import { useEffect, useRef, useState } from "react";
import type { PointerEvent } from "react";
import type { PageViewport } from "pdfjs-dist";
import type { PageGeometry } from "../../lib/authoring";
import { draftPlacement, gesturePlacement, imageDraftPending, imagePath } from "./model";
import type { ImageEditDraft, ImageOccurrence, ImagePlacement } from "./types";

export interface ImageEditingOverlayInput {
  images: ImageOccurrence[];
  draft: ImageEditDraft | null;
  geometry: PageGeometry;
  disabled: boolean;
  active: boolean;
  inspect(page: number): Promise<void>;
  onSelect(page: number, image: ImageOccurrence): void;
  onPlace(page: number, placement: ImagePlacement): void;
  onGesture(pending: boolean): void;
  onError(message: string): void;
}

export default function ImageEditingOverlay(props: ImageEditingOverlayInput & { page: number; viewport: PageViewport }) {
  const root = useRef<HTMLDivElement>(null);
  const gesture = useRef<{ start: [number, number]; placement: ImagePlacement; resize: boolean } | null>(null);
  const [preview, setPreview] = useState<ImagePlacement | null>(null);
  const callbacks = useRef(props);
  callbacks.current = props;
  useEffect(() => {
    if (props.active) void props.inspect(props.page);
  }, [props.active, props.page, props.inspect]);
  useEffect(() => () => callbacks.current.onGesture(false), [props.page]);
  const draft = props.draft?.page === props.page ? props.draft : null;
  const sx = props.viewport.width / props.geometry.widthPoints, sy = props.viewport.height / props.geometry.heightPoints;
  let placement: ImagePlacement | null = null;
  if (draft && !draft.remove) {
    try { placement = draftPlacement(draft, props.geometry); }
    catch { /* The panel exposes invalid numeric input; do not guess a preview. */ }
  }
  const editable = Boolean(props.active && !props.disabled && draft?.source.editable && !draft.remove && placement);
  useEffect(() => {
    if (!editable && gesture.current) {
      gesture.current = null; setPreview(null); callbacks.current.onGesture(false);
    }
  }, [editable]);
  function cancel() {
    gesture.current = null; setPreview(null); props.onGesture(false);
  }
  function point(event: { clientX: number; clientY: number }): [number, number] {
    const bounds = root.current!.getBoundingClientRect();
    return [(event.clientX - bounds.left) / sx, (event.clientY - bounds.top) / sy];
  }
  function start(event: PointerEvent<HTMLButtonElement>, resize: boolean) {
    if (!editable || !placement || event.button !== 0) return;
    event.preventDefault(); event.stopPropagation();
    event.currentTarget.focus({ preventScroll: true });
    window.getSelection()?.removeAllRanges();
    gesture.current = { start: point(event), placement, resize };
    props.onGesture(true);
    event.currentTarget.setPointerCapture(event.pointerId);
  }
  function move(event: PointerEvent<HTMLButtonElement>) {
    if (!gesture.current) return;
    const end = point(event), { start, placement, resize } = gesture.current;
    setPreview(gesturePlacement(placement, end[0] - start[0], end[1] - start[1], resize, props.geometry));
  }
  function finish(event: PointerEvent<HTMLButtonElement>) {
    if (!gesture.current) return;
    const end = point(event), { start, placement, resize } = gesture.current;
    const moved = Math.abs(end[0] - start[0]) > 0.01 || Math.abs(end[1] - start[1]) > 0.01;
    const next = gesturePlacement(placement, end[0] - start[0], end[1] - start[1], resize, props.geometry);
    cancel();
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    if (moved) props.onPlace(props.page, next);
  }
  const style = ([x, y, width, height]: ImagePlacement) => ({ left: x * sx, top: y * sy, width: width * sx, height: height * sy });
  const selectedPlacement = preview ?? placement;
  return <div ref={root} className="image-editing-overlay">
    {props.active && props.images.map((image, index) => {
      if (!image.placement) return null;
      const selected = Boolean(draft && imagePath(draft.source.path) === imagePath(image.path));
      const displayed = selected && selectedPlacement ? selectedPlacement : image.placement;
      const locked = props.disabled || imageDraftPending(props.draft) && !selected;
      return <button type="button" key={imagePath(image.path)} aria-pressed={selected}
        aria-label={`Select source image ${index + 1}, ${image.name}, page ${props.page + 1}${image.editable ? "" : ", read-only"}`}
        title={image.editable ? "Select this exact source occurrence" : image.reason ?? "Read-only source image"}
        disabled={locked}
        className={`source-image-target${selected ? " selected" : ""}${!image.editable ? " unsupported" : ""}${selected && draft?.remove ? " removing" : ""}`}
        style={style(displayed)} onClick={(event) => { event.stopPropagation(); props.onSelect(props.page, image); }}
        onPointerDown={(event) => { if (selected) start(event, false); else event.stopPropagation(); }}
        onPointerMove={move} onPointerUp={finish} onPointerCancel={cancel} onLostPointerCapture={cancel}
        onKeyDown={(event) => {
          if (event.key === "Escape" && gesture.current) { event.stopPropagation(); cancel(); return; }
          if (!selected || !editable || !placement || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
          event.preventDefault(); event.stopPropagation();
          const step = event.shiftKey ? 10 : 1;
          props.onPlace(props.page, gesturePlacement(placement, event.key === "ArrowLeft" ? -step : event.key === "ArrowRight" ? step : 0,
            event.key === "ArrowUp" ? -step : event.key === "ArrowDown" ? step : 0, false, props.geometry));
        }}>
        <span>{selected && imageDraftPending(draft) ? draft?.remove ? "Remove on Apply" : "Image draft" : `Image ${index + 1}${image.editable ? "" : " · read-only"}`}</span>
      </button>;
    })}
    {editable && selectedPlacement && <button type="button" className="source-image-resize" aria-label="Resize selected source image"
      style={{ left: (selectedPlacement[0] + selectedPlacement[2]) * sx - 7, top: (selectedPlacement[1] + selectedPlacement[3]) * sy - 7 }}
      onPointerDown={(event) => start(event, true)} onPointerMove={move} onPointerUp={finish}
      onPointerCancel={cancel} onLostPointerCapture={cancel}
      onKeyDown={(event) => {
        if (!placement || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
        event.preventDefault(); event.stopPropagation();
        const step = event.shiftKey ? 10 : 1;
        props.onPlace(props.page, gesturePlacement(placement, event.key === "ArrowLeft" ? -step : event.key === "ArrowRight" ? step : 0,
          event.key === "ArrowUp" ? -step : event.key === "ArrowDown" ? step : 0, true, props.geometry));
      }} />}
  </div>;
}
