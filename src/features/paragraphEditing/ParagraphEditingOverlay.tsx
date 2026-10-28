import { useEffect, useRef, useState } from "react";
import type { PointerEvent } from "react";
import type { PageViewport } from "pdfjs-dist";
import type { PageGeometry } from "../../lib/authoring";
import { paragraphDraftPending, paragraphGesture, paragraphPath, paragraphPlacement } from "./model";
import type { ParagraphBlock, ParagraphDraft, ParagraphLayout, ParagraphPlacement } from "./types";

export interface ParagraphOverlayInput {
  blocks: ParagraphBlock[];
  draft: ParagraphDraft | null;
  layout: ParagraphLayout | null;
  geometry: PageGeometry;
  disabled: boolean;
  inspect(page: number): Promise<void>;
  onSelect(page: number, block: ParagraphBlock): void;
  onPlace(page: number, placement: ParagraphPlacement): void;
  onGesture(pending: boolean): void;
}

export default function ParagraphEditingOverlay(props: ParagraphOverlayInput & { page: number; viewport: PageViewport }) {
  const root = useRef<HTMLDivElement>(null);
  const gesture = useRef<{ start: [number, number]; placement: ParagraphPlacement; resize: boolean } | null>(null);
  const [temporary, setTemporary] = useState<ParagraphPlacement | null>(null);
  const callbacks = useRef(props);
  callbacks.current = props;
  useEffect(() => { void props.inspect(props.page); }, [props.page, props.inspect]);
  useEffect(() => () => { if (gesture.current) callbacks.current.onGesture(false); }, [props.page]);
  const sx = props.viewport.width / props.geometry.widthPoints, sy = props.viewport.height / props.geometry.heightPoints;
  const draft = props.draft?.page === props.page ? props.draft : null;
  let placement: ParagraphPlacement | null = null;
  if (draft) {
    try { placement = paragraphPlacement(draft, props.geometry); }
    catch { /* Invalid numeric fields are reported by the persistent panel, not replaced with guessed bounds. */ }
  }
  const editable = Boolean(!props.disabled && draft?.source.editable && placement);
  useEffect(() => {
    if (!editable && gesture.current) { gesture.current = null; setTemporary(null); callbacks.current.onGesture(false); }
  }, [editable]);
  function cancel() {
    if (!gesture.current) return;
    gesture.current = null; setTemporary(null); props.onGesture(false);
  }
  function position(event: { clientX: number; clientY: number }): [number, number] {
    const bounds = root.current!.getBoundingClientRect();
    return [(event.clientX - bounds.left) / sx, (event.clientY - bounds.top) / sy];
  }
  function start(event: PointerEvent<HTMLButtonElement>, resize: boolean) {
    if (!editable || !placement || event.button !== 0) return;
    event.preventDefault(); event.stopPropagation();
    event.currentTarget.focus({ preventScroll: true });
    window.getSelection()?.removeAllRanges();
    gesture.current = { start: position(event), placement, resize };
    props.onGesture(true);
    event.currentTarget.setPointerCapture(event.pointerId);
  }
  function move(event: PointerEvent<HTMLButtonElement>) {
    if (!gesture.current) return;
    const end = position(event), { start, placement, resize } = gesture.current;
    setTemporary(paragraphGesture(placement, end[0] - start[0], end[1] - start[1], resize, props.geometry));
  }
  function finish(event: PointerEvent<HTMLButtonElement>) {
    if (!gesture.current) return;
    const end = position(event), { start, placement, resize } = gesture.current;
    const moved = Math.abs(end[0] - start[0]) > 0.01 || Math.abs(end[1] - start[1]) > 0.01;
    const next = paragraphGesture(placement, end[0] - start[0], end[1] - start[1], resize, props.geometry);
    cancel();
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    if (moved) props.onPlace(props.page, next);
  }
  const style = ([x, y, width, height]: ParagraphPlacement) => ({ left: x * sx, top: y * sy, width: width * sx, height: height * sy });
  const selectedPlacement = temporary ?? placement;
  return <div ref={root} className="paragraph-editing-overlay">
    {props.blocks.map((block, index) => {
      if (!block.placement) return null;
      const selected = Boolean(draft && paragraphPath(draft.source.path) === paragraphPath(block.path));
      const displayed = selected && selectedPlacement ? selectedPlacement : block.placement;
      return <button type="button" key={`${block.id}:${paragraphPath(block.path)}`} aria-pressed={selected}
        aria-label={`Select source paragraph ${index + 1}, page ${props.page + 1}${block.editable ? "" : ", read-only"}`}
        title={block.editable ? "Select this actual source paragraph" : block.reason ?? "Read-only paragraph"}
        disabled={props.disabled || paragraphDraftPending(props.draft) && !selected}
        className={`source-paragraph-target${selected ? " selected" : ""}${!block.editable ? " unsupported" : ""}`}
        style={style(displayed)}
        onClick={(event) => { event.stopPropagation(); props.onSelect(props.page, block); }}
        onPointerDown={(event) => { if (selected) start(event, false); else event.stopPropagation(); }}
        onPointerMove={move} onPointerUp={finish} onPointerCancel={cancel} onLostPointerCapture={cancel}
        onKeyDown={(event) => {
          if (event.key === "Escape" && gesture.current) { event.stopPropagation(); cancel(); return; }
          if (!selected || !editable || !placement || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
          event.preventDefault(); event.stopPropagation();
          const step = event.shiftKey ? 10 : 1;
          props.onPlace(props.page, paragraphGesture(placement, event.key === "ArrowLeft" ? -step : event.key === "ArrowRight" ? step : 0,
            event.key === "ArrowUp" ? -step : event.key === "ArrowDown" ? step : 0, false, props.geometry));
        }}><span>{selected && paragraphDraftPending(draft) ? "Paragraph draft" : `Paragraph ${index + 1}${block.editable ? "" : " - read-only"}`}</span></button>;
    })}
    {editable && selectedPlacement && <button type="button" className="source-paragraph-resize" aria-label="Resize selected source paragraph"
      style={{ left: (selectedPlacement[0] + selectedPlacement[2]) * sx - 7, top: (selectedPlacement[1] + selectedPlacement[3]) * sy - 7 }}
      onPointerDown={(event) => start(event, true)} onPointerMove={move} onPointerUp={finish} onPointerCancel={cancel} onLostPointerCapture={cancel}
      onKeyDown={(event) => {
        if (!placement || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
        event.preventDefault(); event.stopPropagation();
        const step = event.shiftKey ? 10 : 1;
        props.onPlace(props.page, paragraphGesture(placement, event.key === "ArrowLeft" ? -step : event.key === "ArrowRight" ? step : 0,
          event.key === "ArrowUp" ? -step : event.key === "ArrowDown" ? step : 0, true, props.geometry));
      }} />}
    {draft && selectedPlacement && props.layout && !temporary && paragraphDraftPending(draft) && <svg className="paragraph-ink-bounds"
      width={props.viewport.width} height={props.viewport.height} aria-hidden="true">
      {props.layout.lines.map((line, index) => {
        if (!line.inkBounds) return null;
        const [left, top, width, height] = line.inkBounds;
        const [x, y, boxWidth, boxHeight] = selectedPlacement;
        const rectangle = draft.source.rotation === 0 ? [x + left, y + top, width, height]
          : draft.source.rotation === 90 ? [x + boxWidth - top - height, y + left, height, width]
          : draft.source.rotation === 180 ? [x + boxWidth - left - width, y + boxHeight - top - height, width, height]
          : [x + top, y + boxHeight - left - width, height, width];
        return <rect key={index} x={rectangle[0] * sx} y={rectangle[1] * sy} width={rectangle[2] * sx} height={rectangle[3] * sy} />;
      })}
    </svg>}
  </div>;
}
