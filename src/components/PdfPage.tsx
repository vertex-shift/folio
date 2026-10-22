import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Highlighter, LoaderCircle, MessageSquareText, TriangleAlert } from "lucide-react";
import { TextLayer } from "pdfjs-dist/legacy/build/pdf.mjs";
import type { PageViewport, PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import { errorMessage, selectionQuads } from "../lib/geometry";
import { findMatches, mapText } from "../lib/search";
import type { AnnotationInfo, Point, Quad, Tool, Zoom } from "../lib/types";

interface Props {
  pdf: PDFDocumentProxy;
  page: number;
  zoom: Zoom;
  width: number;
  height: number;
  tool: Tool;
  disabled: boolean;
  annotations: AnnotationInfo[];
  selectedAnnotation: string | null;
  query: string;
  matchCase: boolean;
  activeMatch: number;
  onScale(scale: number): void;
  onHighlight(quads: Quad[], text: string): void;
  onNote(point: Point): void;
  onAnnotation(id: string): void;
  onError(message: string): void;
}

interface Layout {
  viewport: PageViewport;
  layer: TextLayer;
  text: ReturnType<typeof mapText>;
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
  const [size, setSize] = useState({ width: 540, height: 699 });
  const [layout, setLayout] = useState<Layout | null>(null);
  const [loading, setLoading] = useState(true);
  const [failure, setFailure] = useState<string | null>(null);
  const [searchRects, setSearchRects] = useState<SearchRect[]>([]);
  const [selection, setSelection] = useState<Selection | null>(null);
  const customScale = props.zoom.mode === "custom" ? props.zoom.scale : 1;

  useEffect(() => {
    let cancelled = false;
    let render: RenderTask | undefined;
    let layer: TextLayer | undefined;
    setLoading(true);
    setFailure(null);
    setLayout(null);
    setSelection(null);

    async function draw() {
      const page = await props.pdf.getPage(props.page + 1);
      if (cancelled || !hostRef.current) return;
      const base = page.getViewport({ scale: 1 });
      const availableWidth = Math.max(120, props.width - (props.width < 560 ? 48 : 128));
      const availableHeight = Math.max(160, props.height - 108);
      const scale = props.zoom.mode === "page"
        ? Math.min(availableWidth / base.width, availableHeight / base.height)
        : props.zoom.mode === "width" ? availableWidth / base.width : customScale;
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
      setLayout({ viewport, layer, text: mapText(items) });
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
    };
  }, [props.pdf, props.page, props.zoom.mode, customScale, props.width, props.height]);

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
      ?.scrollIntoView({ block: "center", inline: "nearest", behavior: "smooth" });
  }, [searchRects, props.activeMatch]);

  useEffect(() => {
    setSelection(null);
  }, [props.tool]);

  function captureSelection(autoHighlight: boolean) {
    if (props.disabled || !layout || !paperRef.current || props.tool === "hand" || props.tool === "note") return;
    const current = window.getSelection();
    if (!current || current.isCollapsed || !current.rangeCount) {
      setSelection(null);
      return;
    }
    const range = current.getRangeAt(0);
    if (!hostRef.current?.contains(range.startContainer) || !hostRef.current.contains(range.endContainer)) return;
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

  return (
    <div
      ref={paperRef}
      className={`pdf-paper tool-${props.tool}`}
      style={size}
      role="region"
      aria-label={`PDF page ${props.page + 1}`}
      aria-busy={loading}
      data-page={props.page + 1}
      onPointerUp={() => {
        requestAnimationFrame(() => {
          try { captureSelection(true); } catch (error) { props.onError(errorMessage(error)); }
        });
      }}
      onKeyUp={() => {
        try { captureSelection(false); } catch (error) { props.onError(errorMessage(error)); }
      }}
      onClick={(event) => {
        if (props.tool !== "note" || props.disabled || !layout || event.target instanceof Element && event.target.closest("button")) return;
        const rectangle = event.currentTarget.getBoundingClientRect();
        const point = layout.viewport.convertToPdfPoint(event.clientX - rectangle.left, event.clientY - rectangle.top);
        if (Number.isFinite(point[0]) && Number.isFinite(point[1])) props.onNote([point[0], point[1]]);
      }}
    >
      <div ref={hostRef} className="page-content" />
      {searchRects.map((rectangle, index) => (
        <span key={index} data-search-index={rectangle.index}
          className={`search-mark ${rectangle.index === props.activeMatch ? "current" : ""}`}
          style={{ left: rectangle.left, top: rectangle.top, width: rectangle.width, height: rectangle.height }}
        />
      ))}
      {layout && props.annotations.filter((annotation) => annotation.kind === "note").map((annotation) => {
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
