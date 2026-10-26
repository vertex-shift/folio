import { useRef } from "react";
import type { PageViewport } from "pdfjs-dist";
import type { AuthoringDraft } from "../lib/authoring";
import type { Point, Tool } from "../lib/types";

interface Props {
  draft: AuthoringDraft;
  page: number;
  viewport: PageViewport;
  tool: Tool;
  disabled: boolean;
  onPoint(point: Point): void;
  onCrop(margins: [number, number, number, number]): void;
  onError(message: string): void;
}

export default function AuthoringOverlay(props: Props) {
  const start = useRef<Point | null>(null);
  const draft = props.draft;
  const scale = props.viewport.scale;
  const width = props.viewport.width / scale, height = props.viewport.height / scale;
  const target = draft.page === props.page;
  const active = !props.disabled && (draft.kind === "image" && props.tool === "image" && Boolean(draft.asset)
    || draft.kind === "crop" && props.tool === "crop");
  function point(event: { clientX: number; clientY: number }, element: HTMLElement): Point {
    const bounds = element.getBoundingClientRect();
    return [Math.max(0, Math.min(width, (event.clientX - bounds.left) / scale)),
      Math.max(0, Math.min(height, (event.clientY - bounds.top) / scale))];
  }
  let image: { x: number; y: number; width: number; height: number; opacity: number } | null = null;
  if (target && draft.kind === "image" && draft.previewUrl) {
    const values = [draft.x, draft.y, draft.width, draft.height, draft.opacity].map(Number);
    if (values.every(Number.isFinite) && values[2] > 0 && values[3] > 0) {
      image = { x: values[0], y: values[1], width: values[2], height: values[3], opacity: values[4] / 100 };
    }
  }
  let crop: { top: number; right: number; bottom: number; left: number } | null = null;
  if (target && draft.kind === "crop" && draft.dirty) {
    const [top, right, bottom, left] = draft.margins.map(Number);
    if ([top, right, bottom, left].every((value) => Number.isFinite(value) && value >= 0)
      && top + bottom < height && left + right < width) crop = { top, right, bottom, left };
  }
  if (!active && !image && !crop) return null;
  return <div className={`authoring-overlay ${active ? "capturing" : ""}`} aria-hidden="true"
    onPointerDown={(event) => {
      if (!active || event.button !== 0) return;
      event.preventDefault(); event.stopPropagation();
      window.getSelection()?.removeAllRanges();
      const local = point(event, event.currentTarget);
      if (draft.kind === "image") { props.onPoint(local); return; }
      start.current = local;
      event.currentTarget.setPointerCapture(event.pointerId);
    }}
    onPointerMove={(event) => {
      if (!active || !start.current || draft.kind !== "crop") return;
      const end = point(event, event.currentTarget), begin = start.current;
      if (Math.abs(end[0] - begin[0]) < 1 || Math.abs(end[1] - begin[1]) < 1) return;
      props.onCrop([Math.min(begin[1], end[1]), width - Math.max(begin[0], end[0]),
        height - Math.max(begin[1], end[1]), Math.min(begin[0], end[0])]);
    }}
    onPointerUp={(event) => {
      if (!active) return;
      event.stopPropagation();
      const begin = start.current;
      start.current = null;
      if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
      if (!begin) return;
      const end = point(event, event.currentTarget);
      if (Math.abs(end[0] - begin[0]) < 1 || Math.abs(end[1] - begin[1]) < 1) {
        props.onError("Drag a crop area at least one physical point wide and high, or enter margins in Page authoring.");
      }
    }}
    onPointerCancel={(event) => {
      start.current = null;
      if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    }}>
    {image && draft.kind === "image" && draft.previewUrl && <div className="authoring-image-placement"
      style={{ left: image.x * scale, top: image.y * scale, width: image.width * scale, height: image.height * scale }}>
      <img src={draft.previewUrl} alt="" draggable={false} style={{ opacity: image.opacity }} />
      <span>Image preview</span>
    </div>}
    {crop && <>
      <div className="authoring-crop-shade" style={{ inset: `0 0 ${height * scale - crop.top * scale}px` }} />
      <div className="authoring-crop-shade" style={{ inset: `${(height - crop.bottom) * scale}px 0 0` }} />
      <div className="authoring-crop-shade" style={{ top: crop.top * scale, bottom: crop.bottom * scale, left: 0, width: crop.left * scale }} />
      <div className="authoring-crop-shade" style={{ top: crop.top * scale, bottom: crop.bottom * scale, right: 0, width: crop.right * scale }} />
      <div className="authoring-crop-kept" style={{ top: crop.top * scale, right: crop.right * scale, bottom: crop.bottom * scale, left: crop.left * scale }}>
        <span>Crop preview</span>
      </div>
    </>}
  </div>;
}
