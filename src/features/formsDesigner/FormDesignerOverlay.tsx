import { useEffect, useRef, useState } from "react";
import type { PageViewport } from "pdfjs-dist";
import type { DesignerDraft, DesignField } from "./types";
import type { Point } from "../../lib/types";

export interface FormDesignerOverlayInput {
  fields: DesignField[];
  draft: DesignerDraft | null;
  widgetIndex: number;
  active: boolean;
  disabled: boolean;
  onSelect(id: string, widget: string): void;
  onPlace(page: number, placement: [number, number, number, number]): void;
  onGesture(pending: boolean): void;
}

export default function FormDesignerOverlay(props: FormDesignerOverlayInput & { page: number; viewport: PageViewport }) {
  const start = useRef<Point | null>(null);
  const [preview, setPreview] = useState<[number, number, number, number] | null>(null);
  const callbacks = useRef(props);
  callbacks.current = props;
  useEffect(() => () => callbacks.current.onGesture(false), [props.page]);
  const { viewport, draft } = props;
  const scale = viewport.scale, width = viewport.width / scale, height = viewport.height / scale;
  const capturing = props.active && !props.disabled && Boolean(draft && !draft.deleteField);
  useEffect(() => {
    if (!capturing && start.current) {
      start.current = null; setPreview(null); callbacks.current.onGesture(false);
    }
  }, [capturing]);
  function point(event: { clientX: number; clientY: number }, element: HTMLElement): Point {
    const bounds = element.getBoundingClientRect();
    return [Math.max(0, Math.min(width, (event.clientX - bounds.left) / scale)),
      Math.max(0, Math.min(height, (event.clientY - bounds.top) / scale))];
  }
  function cancel() { start.current = null; setPreview(null); props.onGesture(false); }
  return <div className={`form-designer-overlay${capturing ? " capturing" : ""}`}
    onPointerDown={(event) => {
      if (!capturing || event.button !== 0 || event.target instanceof Element && event.target.closest("button")) return;
      event.preventDefault(); event.stopPropagation();
      start.current = point(event, event.currentTarget);
      window.getSelection()?.removeAllRanges();
      props.onGesture(true);
      event.currentTarget.setPointerCapture(event.pointerId);
    }}
    onPointerMove={(event) => {
      if (!start.current) return;
      const end = point(event, event.currentTarget), begin = start.current;
      setPreview([Math.min(begin[0], end[0]), Math.min(begin[1], end[1]), Math.abs(end[0] - begin[0]), Math.abs(end[1] - begin[1])]);
    }}
    onPointerUp={(event) => {
      if (!start.current || !draft) return;
      event.preventDefault(); event.stopPropagation();
      const end = point(event, event.currentTarget), begin = start.current;
      const widget = draft.widgets[props.widgetIndex];
      const dragged = Math.abs(end[0] - begin[0]) >= 3 && Math.abs(end[1] - begin[1]) >= 3;
      const placement: [number, number, number, number] = dragged
        ? [Math.min(begin[0], end[0]), Math.min(begin[1], end[1]), Math.abs(end[0] - begin[0]), Math.abs(end[1] - begin[1])]
        : [Math.max(0, Math.min(end[0], width - Number(widget?.width))), Math.max(0, Math.min(end[1], height - Number(widget?.height))), Number(widget?.width), Number(widget?.height)];
      cancel();
      if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
      props.onPlace(props.page, placement);
    }}
    onPointerCancel={cancel} onLostPointerCapture={cancel}>
    {props.fields.flatMap(({ field, editable }) => field.widgets.filter((widget) => widget.page === props.page
      && !(draft?.fieldId === field.id && draft.widgets.some((item) => item.id === widget.id))).map((widget) => {
      const a = viewport.convertToViewportPoint(widget.rect[0], widget.rect[1]);
      const b = viewport.convertToViewportPoint(widget.rect[2], widget.rect[3]);
      const rect = [a[0], a[1], b[0], b[1]];
      return <button type="button" className="designer-field-target" key={widget.id} disabled={props.disabled || !editable}
        aria-label={`Design ${field.label}, widget ${widget.id}, page ${widget.page + 1}`}
        style={{ left: Math.min(rect[0], rect[2]), top: Math.min(rect[1], rect[3]), width: Math.abs(rect[2] - rect[0]), height: Math.abs(rect[3] - rect[1]) }}
        onClick={(event) => { event.stopPropagation(); props.onSelect(field.id, widget.id); }}>
        <span>{field.kind === "signature" ? "Unsigned signature" : field.label}</span>
      </button>;
    }))}
    {!draft?.deleteField && draft?.widgets.filter((widget) => Number(widget.page) - 1 === props.page).map((widget) => {
      const values = [widget.x, widget.y, widget.width, widget.height].map(Number);
      if (!values.every(Number.isFinite) || values[2] <= 0 || values[3] <= 0) return null;
      return <div key={widget.id ?? draft.widgets.indexOf(widget)} className="designer-widget-preview" aria-hidden="true"
        style={{ left: values[0] * scale, top: values[1] * scale, width: values[2] * scale, height: values[3] * scale }}>
        <span>{draft.definition.kind === "signature" ? "Unsigned signature" : draft.definition.label || draft.definition.name} · draft</span>
      </div>;
    })}
    {preview && <div className="designer-widget-preview drawing" aria-hidden="true"
      style={{ left: preview[0] * scale, top: preview[1] * scale, width: preview[2] * scale, height: preview[3] * scale }} />}
  </div>;
}
