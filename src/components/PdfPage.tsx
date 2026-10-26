import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Highlighter, LoaderCircle, MessageSquareText, TriangleAlert } from "lucide-react";
import { TextLayer } from "pdfjs-dist/legacy/build/pdf.mjs";
import type { PageViewport, PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import { errorMessage, intersectsPdfRect, selectionQuads } from "../lib/geometry";
import { findMatches, mapText } from "../lib/search";
import { isRecord } from "../lib/navigation";
import { validateRedactionRect } from "../lib/redaction";
import { pageScale } from "../lib/continuous";
import AuthoringOverlay from "./AuthoringOverlay";
import type { AuthoringDraft } from "../lib/authoring";
import type { AnnotationInfo, PdfRect, Point, Quad, RedactionMark, Tool, Zoom } from "../lib/types";

interface Props {
  pdf: PDFDocumentProxy;
  page: number;
  zoom: Zoom;
  width: number;
  height: number;
  initialSize?: { width: number; height: number };
  tool: Tool;
  disabled: boolean;
  copyAllowed: boolean;
  annotations: AnnotationInfo[];
  selectedAnnotation: string | null;
  query: string;
  matchCase: boolean;
  activeMatch: number;
  onScale(scale: number): void;
  onHighlight(quads: Quad[], text: string): void;
  onNote(point: Point): void;
  onText(rect: [number, number, number, number]): void;
  onEditExisting(text: string, origin: Point): void;
  onInk(points: Point[]): void;
  onGestureDraft?(pending: boolean): void;
  redactions: RedactionMark[];
  onRedaction(rect: PdfRect): void;
  onLink(destination: unknown, url: string | null): void;
  penColor: string;
  penWidth: number;
  onAnnotation(id: string): void;
  onError(message: string): void;
  authoringDraft: AuthoringDraft | null;
  onAuthoringPoint(point: Point): void;
  onAuthoringCrop(margins: [number, number, number, number]): void;
}

interface Layout {
  viewport: PageViewport;
  layer: TextLayer;
  text: ReturnType<typeof mapText>;
  items: { text: string; origin: Point }[];
}

interface SearchRect {
  left: number;
  top: number;
  width: number;
  height: number;
  index: number;
}

interface Selection {
  text: string;
  quads: Quad[];
  left: number;
  top: number;
}

function rangeForText(element: HTMLElement, start: number, end: number): Range | null {
  const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  let offset = 0;
  let foundStart = false;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const length = node.textContent?.length ?? 0;
    if (!foundStart && start <= offset + length) {
      range.setStart(node, Math.max(0, start - offset));
      foundStart = true;
    }
    if (foundStart && end <= offset + length) {
      range.setEnd(node, Math.max(0, end - offset));
      return range;
    }
    offset += length;
  }
  return null;
}

export default function PdfPage(props: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const paperRef = useRef<HTMLDivElement>(null);
  const callbacks = useRef(props);
  callbacks.current = props;
  const [size, setSize] = useState(props.initialSize ?? { width: 540, height: 699 });
  const [layout, setLayout] = useState<Layout | null>(null);
  const [loading, setLoading] = useState(true);
  const [failure, setFailure] = useState<string | null>(null);
  const [searchRects, setSearchRects] = useState<SearchRect[]>([]);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [links, setLinks] = useState<{ rect: number[]; destination: unknown; url: string | null }[]>([]);
  const [stroke, setStroke] = useState<Point[]>([]);
  const strokeRef = useRef<Point[]>([]);
  const redactionStart = useRef<Point | null>(null);
  const [redactionPreview, setRedactionPreview] = useState<{ left: number; top: number; width: number; height: number } | null>(null);
  const customScale = props.zoom.mode === "custom" ? props.zoom.scale : 1;

  useEffect(() => {
    let cancelled = false;
    let render: RenderTask | undefined;
    let layer: TextLayer | undefined;
    setLoading(true);
    setFailure(null);
    setLayout(null);
    setSelection(null);
    setLinks([]);
    strokeRef.current = [];
    setStroke([]);
    redactionStart.current = null;
    setRedactionPreview(null);
    callbacks.current.onGestureDraft?.(false);

    async function draw() {
      const page = await props.pdf.getPage(props.page + 1);
      if (cancelled || !hostRef.current) return;
      const base = page.getViewport({ scale: 1 });
      const scale = pageScale(base, props.width, props.height, props.zoom);
      const viewport = page.getViewport({ scale });
      const pixelScale = Math.min(
        window.devicePixelRatio || 1, 2,
        16384 / viewport.width, 16384 / viewport.height,
        Math.sqrt(16_000_000 / (viewport.width * viewport.height)),
      );
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.floor(viewport.width * pixelScale));
      canvas.height = Math.max(1, Math.floor(viewport.height * pixelScale));
      canvas.style.width = `${viewport.width}px`;
      canvas.style.height = `${viewport.height}px`;
      canvas.setAttribute("role", "img");
      canvas.setAttribute("aria-label", `Rendered PDF page ${props.page + 1}`);
      const textContainer = document.createElement("div");
      textContainer.className = "textLayer";
      textContainer.setAttribute("aria-label", `Selectable text on page ${props.page + 1}`);
      textContainer.style.setProperty("--total-scale-factor", String(scale * page.userUnit));
      hostRef.current.replaceChildren(canvas, textContainer);
      setSize({ width: viewport.width, height: viewport.height });
      callbacks.current.onScale(scale);

      render = page.render({
        canvas,
        viewport,
        transform: [pixelScale, 0, 0, pixelScale, 0, 0],
      });
      await render.promise;
      if (cancelled) return;
      const content = await page.getTextContent();
      if (cancelled) return;
      layer = new TextLayer({ textContentSource: content, container: textContainer, viewport });
      // Explicit dimensions also support WebKit versions without CSS round().
      textContainer.style.width = `${viewport.rotation % 180 ? viewport.height : viewport.width}px`;
      textContainer.style.height = `${viewport.rotation % 180 ? viewport.width : viewport.height}px`;
      await layer.render();
      if (cancelled) return;
      const items = content.items.filter((item) => "str" in item);
      const annotations: unknown[] = await page.getAnnotations({ intent: "display" });
      if (cancelled) return;
      const links = annotations.filter((item) => isRecord(item) && item.annotationType === 2);
      setLinks(links.map((item) => {
        if (!isRecord(item) || !Array.isArray(item.rect) || item.rect.length !== 4
          || !item.rect.every((value) => typeof value === "number" && Number.isFinite(value))) {
          throw new Error("A document link has invalid page coordinates.");
        }
        return { rect: item.rect, destination: item.dest, url: typeof item.url === "string" ? item.url : null };
      }));
      setLayout({ viewport, layer, text: mapText(items), items: items.map((item) => ({
        text: item.str, origin: [Number(item.transform[4]), Number(item.transform[5])],
      })) });
      setLoading(false);
    }

    void draw().catch((error: unknown) => {
      if (cancelled) return;
      const message = `Page ${props.page + 1} could not be displayed: ${errorMessage(error)}`;
      setFailure(message);
      setLoading(false);
      callbacks.current.onError(message);
    });
    return () => {
      cancelled = true;
      render?.cancel();
      layer?.cancel();
      callbacks.current.onGestureDraft?.(false);
    };
  }, [props.pdf, props.page, props.zoom.mode, customScale, props.width, props.height]);

  useEffect(() => {
    if (!layout) return;
    const enabled = props.tool === "edit-text" && !props.disabled;
    layout.layer.textDivs.forEach((div, index) => {
      const text = layout.items[index]?.text.trim();
      if (enabled && text) {
        div.setAttribute("role", "button");
        div.setAttribute("tabindex", "0");
        div.setAttribute("aria-label", `Edit existing text: ${text.slice(0, 100)}`);
      } else {
        div.setAttribute("role", "presentation");
        div.removeAttribute("tabindex"); div.removeAttribute("aria-label");
      }
    });
    return () => layout.layer.textDivs.forEach((div) => {
      div.setAttribute("role", "presentation");
      div.removeAttribute("tabindex"); div.removeAttribute("aria-label");
    });
  }, [layout, props.tool, props.disabled]);

  useLayoutEffect(() => {
    if (!layout || !paperRef.current || !props.query.trim()) {
      setSearchRects([]);
      return;
    }
    const paper = paperRef.current.getBoundingClientRect();
    const rectangles: SearchRect[] = [];
    const matches = findMatches(layout.text.text, props.query, props.matchCase);
    matches.forEach((match, index) => {
      for (const run of layout.text.runs) {
        const start = Math.max(run.start, match.start);
        const end = Math.min(run.end, match.end);
        const div = layout.layer.textDivs[run.index];
        if (end <= start || !div) continue;
        const range = rangeForText(div, start - run.start, end - run.start);
        if (!range) continue;
        for (const rectangle of range.getClientRects()) {
          if (rectangle.width < 0.5 || rectangle.height < 1) continue;
          rectangles.push({
            left: rectangle.left - paper.left,
            top: rectangle.top - paper.top,
            width: rectangle.width,
            height: rectangle.height,
            index,
          });
        }
      }
    });
    setSearchRects(rectangles);
  }, [layout, props.query, props.matchCase]);

  useEffect(() => {
    if (props.activeMatch < 0) return;
    paperRef.current?.querySelector(`[data-search-index="${props.activeMatch}"]`)
      ?.scrollIntoView({ block: "center", inline: "nearest",
        behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }, [searchRects, props.activeMatch]);

  useEffect(() => {
    setSelection(null);
    strokeRef.current = [];
    setStroke([]);
    redactionStart.current = null;
    setRedactionPreview(null);
    callbacks.current.onGestureDraft?.(false);
  }, [props.tool]);

  function captureSelection(autoHighlight: boolean) {
    if (props.disabled || !layout || !paperRef.current || !["select", "highlight"].includes(props.tool)) return;
    const current = window.getSelection();
    if (!current || current.isCollapsed || !current.rangeCount) {
      setSelection(null);
      return;
    }
    const range = current.getRangeAt(0);
    if (!hostRef.current?.contains(range.startContainer) || !hostRef.current.contains(range.endContainer)) {
      setSelection(null);
      if (autoHighlight && props.tool === "highlight" && hostRef.current && range.intersectsNode(hostRef.current)) {
        props.onError("Highlight one page at a time. Select a separate passage on each page.");
      }
      return;
    }
    const text = current.toString().trim();
    if (!text) return;
    const page = paperRef.current.getBoundingClientRect();
    const rectangles = Array.from(range.getClientRects());
    const quads = selectionQuads(rectangles, page, (x, y) => layout.viewport.convertToPdfPoint(x, y));
    if (!quads.length) return;
    if (text.length > 4000) {
      props.onError("Select a shorter passage, up to 4,000 characters.");
      return;
    }
    if (autoHighlight && props.tool === "highlight") {
      current.removeAllRanges();
      props.onHighlight(quads, text);
      setSelection(null);
      return;
    }
    const first = rectangles[0];
    setSelection({
      text, quads,
      left: Math.max(8, Math.min(size.width - 154, first.left - page.left)),
      top: Math.max(6, first.top - page.top - 42),
    });
  }

  function localPoint(event: { clientX: number; clientY: number }): Point {
    const rect = paperRef.current?.getBoundingClientRect();
    if (!rect) throw new Error("The PDF page is not ready.");
    return [Math.max(0, Math.min(size.width, event.clientX - rect.left)),
      Math.max(0, Math.min(size.height, event.clientY - rect.top))];
  }

  function editExisting(target: EventTarget | null) {
    if (!layout || props.disabled || !(target instanceof Element)) return;
    const span = target.closest(".textLayer span");
    if (!(span instanceof HTMLElement)) return;
    const index = layout.layer.textDivs.indexOf(span);
    const item = layout.items[index];
    if (!item || !item.text.trim()) return;
    if (!item.origin.every(Number.isFinite)) {
      props.onError("This text has no reliable position. Choose its source run from the text list.");
      return;
    }
    window.getSelection()?.removeAllRanges();
    props.onEditExisting(item.text, item.origin);
  }

  return (
    <div
      ref={paperRef}
      className={`pdf-paper tool-${props.tool}${props.copyAllowed ? "" : "copy-restricted"}`}
      style={size}
      role="region"
      aria-label={`PDF page ${props.page + 1}`}
      aria-busy={loading}
      data-page={props.page + 1}
      onKeyDown={(event) => {
        if (props.tool === "edit-text" && ["Enter", " "].includes(event.key)) {
          event.preventDefault(); event.stopPropagation(); editExisting(event.target);
        }
      }}
      onPointerDown={(event) => {
        if (!["ink", "redact"].includes(props.tool) || props.disabled || !layout || event.button !== 0
          || event.target instanceof Element && event.target.closest("button")) return;
        event.preventDefault();
        props.onGestureDraft?.(true);
        if (props.tool === "redact") {
          redactionStart.current = localPoint(event);
          window.getSelection()?.removeAllRanges();
        } else {
          strokeRef.current = [localPoint(event)];
          setStroke(strokeRef.current);
        }
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (redactionStart.current && props.tool === "redact") {
          const start = redactionStart.current;
          const point = localPoint(event);
          setRedactionPreview({ left: Math.min(start[0], point[0]), top: Math.min(start[1], point[1]),
            width: Math.abs(start[0] - point[0]), height: Math.abs(start[1] - point[1]) });
          return;
        }
        if (!strokeRef.current.length || props.tool !== "ink") return;
        const point = localPoint(event);
        const last = strokeRef.current.at(-1);
        if (last && Math.hypot(point[0] - last[0], point[1] - last[1]) < 1) return;
        if (strokeRef.current.length >= 20_000) {
          strokeRef.current = []; setStroke([]);
          props.onGestureDraft?.(false);
          props.onError("This stroke is too long. Draw several shorter strokes.");
          return;
        }
        strokeRef.current = [...strokeRef.current, point];
        setStroke(strokeRef.current);
      }}
      onPointerCancel={() => {
        strokeRef.current = []; setStroke([]);
        redactionStart.current = null; setRedactionPreview(null);
        props.onGestureDraft?.(false);
      }}
      onLostPointerCapture={() => {
        strokeRef.current = []; setStroke([]);
        redactionStart.current = null; setRedactionPreview(null);
        props.onGestureDraft?.(false);
      }}
      onPointerUp={(event) => {
        props.onGestureDraft?.(false);
        if (redactionStart.current && layout) {
          const start = redactionStart.current;
          const end = localPoint(event);
          redactionStart.current = null; setRedactionPreview(null);
          if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
          try {
            if (Math.abs(start[0] - end[0]) < 3 || Math.abs(start[1] - end[1]) < 3) {
              throw new Error("Drag a larger redaction box so you can review what it covers.");
            }
            const a = layout.viewport.convertToPdfPoint(start[0], start[1]);
            const b = layout.viewport.convertToPdfPoint(end[0], end[1]);
            props.onRedaction(validateRedactionRect(
              [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[0], b[0]), Math.max(a[1], b[1])],
              layout.viewport.viewBox,
            ));
          } catch (error) { props.onError(errorMessage(error)); }
          return;
        }
        if (strokeRef.current.length && layout) {
          const points = strokeRef.current;
          strokeRef.current = []; setStroke([]);
          if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
          if (points.length >= 2) props.onInk(points.map(([x, y]) => {
            const point = layout.viewport.convertToPdfPoint(x, y);
            return [point[0], point[1]];
          }));
          return;
        }
        requestAnimationFrame(() => {
          try { captureSelection(true); } catch (error) { props.onError(errorMessage(error)); }
        });
      }}
      onKeyUp={() => {
        try { captureSelection(false); } catch (error) { props.onError(errorMessage(error)); }
      }}
      onClick={(event) => {
        if (props.tool === "edit-text") { editExisting(event.target); return; }
        if (!["note", "text"].includes(props.tool) || props.disabled || !layout || event.target instanceof Element && event.target.closest("button")) return;
        const [x, y] = localPoint(event);
        if (props.tool === "note") {
          const point = layout.viewport.convertToPdfPoint(x, y);
          if (Number.isFinite(point[0]) && Number.isFinite(point[1])) props.onNote([point[0], point[1]]);
        } else {
          const scale = layout.viewport.scale * layout.viewport.userUnit;
          const width = Math.min(300 * scale, size.width - 8 * scale);
          const height = Math.min(150 * scale, size.height - 8 * scale);
          const left = Math.max(0, Math.min(x, size.width - width));
          const top = Math.max(0, Math.min(y, size.height - height));
          const a = layout.viewport.convertToPdfPoint(left, top);
          const b = layout.viewport.convertToPdfPoint(left + width, top + height);
          props.onText([Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[0], b[0]), Math.max(a[1], b[1])]);
        }
      }}
    >
      <div ref={hostRef} className="page-content" />
      {layout && props.authoringDraft && <AuthoringOverlay draft={props.authoringDraft}
        viewport={layout.viewport} page={props.page} tool={props.tool} disabled={props.disabled}
        onPoint={props.onAuthoringPoint} onCrop={props.onAuthoringCrop} onError={props.onError} />}
      {layout && links.filter((link) => intersectsPdfRect(link.rect, layout.viewport.viewBox)).map((link, index) => {
        const a = layout.viewport.convertToViewportPoint(link.rect[0], link.rect[1]);
        const b = layout.viewport.convertToViewportPoint(link.rect[2], link.rect[3]);
        return <button className="pdf-link" key={index} disabled={props.tool !== "select"}
          style={{ left: Math.min(a[0], b[0]), top: Math.min(a[1], b[1]), width: Math.abs(a[0] - b[0]), height: Math.abs(a[1] - b[1]) }}
          title={link.url ? `Open link: ${link.url}` : "Go to document destination"}
          aria-label={link.url ? `Open link: ${link.url}` : `Go to document link ${index + 1}`}
          onClick={(event) => { event.stopPropagation(); props.onLink(link.destination, link.url); }}
        />;
      })}
      {stroke.length > 0 && <svg className="ink-preview" width={size.width} height={size.height} aria-hidden="true">
        <polyline points={stroke.map(([x, y]) => `${x},${y}`).join(" ")} fill="none" stroke={props.penColor}
          strokeWidth={props.penWidth * (layout?.viewport.scale ?? 1) * (layout?.viewport.userUnit ?? 1)} strokeLinecap="round" strokeLinejoin="round" />
      </svg>}
      {layout && props.redactions.map((mark) => {
        const a = layout.viewport.convertToViewportPoint(mark.rect[0], mark.rect[1]);
        const b = layout.viewport.convertToViewportPoint(mark.rect[2], mark.rect[3]);
        return <div key={mark.id} className="redaction-mark" role="img" aria-label={`Pending redaction area ${mark.id}`}
          style={{ left: Math.min(a[0], b[0]), top: Math.min(a[1], b[1]), width: Math.abs(a[0] - b[0]), height: Math.abs(a[1] - b[1]) }}>
          <span aria-hidden="true">{mark.id}</span>
        </div>;
      })}
      {redactionPreview && <div className="redaction-mark preview" style={redactionPreview} aria-hidden="true" />}
      {searchRects.map((rectangle, index) => (
        <span key={index} data-search-index={rectangle.index}
          className={`search-mark ${rectangle.index === props.activeMatch ? "current" : ""}`}
          style={{ left: rectangle.left, top: rectangle.top, width: rectangle.width, height: rectangle.height }}
        />
      ))}
      {layout && props.annotations.filter((annotation) => annotation.kind === "note"
        && intersectsPdfRect(annotation.rect, layout.viewport.viewBox)).map((annotation) => {
        const start = layout.viewport.convertToViewportPoint(annotation.rect[0], annotation.rect[1]);
        const end = layout.viewport.convertToViewportPoint(annotation.rect[2], annotation.rect[3]);
        return (
          <button
            key={annotation.id}
            className={`note-marker ${props.selectedAnnotation === annotation.id ? "selected" : ""}`}
            style={{ left: Math.min(start[0], end[0]), top: Math.min(start[1], end[1]) }}
            title={annotation.text}
            aria-label={`Open note: ${annotation.text.slice(0, 80)}`}
            onClick={(event) => { event.stopPropagation(); props.onAnnotation(annotation.id); }}
          ><MessageSquareText size={16} /></button>
        );
      })}
      {selection && !props.disabled && (
        <button className="selection-action" style={{ left: selection.left, top: selection.top }}
          onPointerDown={(event) => { event.preventDefault(); event.stopPropagation(); }}
          onPointerUp={(event) => event.stopPropagation()}
          onClick={(event) => {
            event.stopPropagation();
            window.getSelection()?.removeAllRanges();
            props.onHighlight(selection.quads, selection.text);
            setSelection(null);
          }}
        ><Highlighter size={15} />Highlight selection</button>
      )}
      {loading && <div className="paper-loading" role="status"><LoaderCircle className="spin" size={22} /><span>Rendering page...</span></div>}
      {failure && <div className="paper-loading paper-failure" role="alert"><TriangleAlert size={26} /><p>{failure}</p></div>}
    </div>
  );
}

interface ThumbnailProps {
  pdf: PDFDocumentProxy;
  page: number;
  width?: number;
}

export function Thumbnail({ pdf, page: index, width = 96 }: ThumbnailProps) {
  const host = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const observer = new IntersectionObserver(
      ([entry]) => setVisible(entry.isIntersecting),
      { rootMargin: "160px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!visible) {
      host.current?.replaceChildren();
      return;
    }
    let cancelled = false;
    let render: RenderTask | undefined;
    setFailure(null);
    async function draw() {
      const page = await pdf.getPage(index + 1);
      if (cancelled || !host.current) return;
      const base = page.getViewport({ scale: 1 });
      const scale = Math.min(width / base.width, width * 1.34 / base.height);
      const viewport = page.getViewport({ scale });
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.floor(viewport.width * 1.5));
      canvas.height = Math.max(1, Math.floor(viewport.height * 1.5));
      canvas.style.width = `${viewport.width}px`;
      canvas.style.height = `${viewport.height}px`;
      canvas.setAttribute("aria-hidden", "true");
      host.current.replaceChildren(canvas);
      render = page.render({ canvas, viewport, transform: [1.5, 0, 0, 1.5, 0, 0] });
      await render.promise;
    }
    void draw().catch((error: unknown) => {
      if (!cancelled) setFailure(errorMessage(error));
    });
    return () => { cancelled = true; render?.cancel(); };
  }, [pdf, index, width, visible]);

  return <div className="thumbnail-art" style={{ width, height: width * 1.34 }}
    aria-hidden="true" title={failure ?? undefined}>
    <div ref={host} className="thumbnail-canvas" />
    {failure && <TriangleAlert size={18} />}
  </div>;
}
