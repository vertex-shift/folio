import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ReactNode, RefObject } from "react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import { LoaderCircle } from "lucide-react";
import { continuousLayout, renderPages, visiblePage } from "../lib/continuous";
import type { PageFrame, PageSize } from "../lib/continuous";
import { errorMessage } from "../lib/geometry";
import type { Zoom } from "../lib/types";

interface Props {
  pdf: PDFDocumentProxy | null;
  documentId: number;
  page: number;
  width: number;
  height: number;
  zoom: Zoom;
  container: RefObject<HTMLDivElement | null>;
  onPage(page: number): void;
  onScale(scale: number): void;
  onError(message: string): void;
  renderPage(page: number, frame: PageFrame): ReactNode;
  initialPosition?: { page: number; fraction: number };
  onPosition?(position: { page: number; fraction: number }): void;
}

export default function ContinuousPages(props: Props) {
  const callbacks = useRef(props);
  callbacks.current = props;
  const [metadata, setMetadata] = useState<{ pdf: PDFDocumentProxy; documentId: number; sizes: PageSize[] } | null>(null);
  const [progress, setProgress] = useState(0);
  const [failure, setFailure] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [position, setPosition] = useState({ top: 0, height: props.height });
  const reported = useRef(-1);
  const pinned = useRef<number | null>(null);
  const anchor = useRef({ documentId: -1, page: 0, fraction: 0 });
  const previousLayout = useRef<ReturnType<typeof continuousLayout> | null>(null);
  const sizes = metadata?.documentId === props.documentId ? metadata.sizes : [];
  const layout = useMemo(() => continuousLayout(sizes, props.width, props.height, props.zoom),
    [sizes, props.width, props.height, props.zoom]);
  const layoutRef = useRef(layout);
  layoutRef.current = layout;
  const ready = metadata?.pdf === props.pdf && Boolean(props.pdf);

  useEffect(() => {
    const pdf = props.pdf;
    pinned.current = null;
    if (!pdf) return;
    let canceled = false;
    let next = 0, completed = 0;
    const sizes: PageSize[] = new Array(pdf.numPages);
    setFailure(null); setProgress(0);
    async function worker() {
      while (!canceled && next < pdf!.numPages) {
        const index = next++;
        const page = await pdf!.getPage(index + 1);
        if (canceled) return;
        const viewport = page.getViewport({ scale: 1 });
        if (![viewport.width, viewport.height].every((value) => Number.isFinite(value) && value > 0)) {
          throw new Error(`Page ${index + 1} has invalid dimensions.`);
        }
        sizes[index] = { width: viewport.width, height: viewport.height };
        completed++;
        if (completed % 16 === 0 || completed === pdf!.numPages) setProgress(completed);
      }
    }
    void Promise.all(Array.from({ length: Math.min(4, pdf.numPages) }, worker)).then(() => {
      if (!canceled) setMetadata({ pdf, documentId: props.documentId, sizes });
    }).catch((error: unknown) => {
      if (canceled) return;
      canceled = true;
      const message = `Continuous pages could not be prepared: ${errorMessage(error)}`;
      setFailure(message); callbacks.current.onError(message);
    });
    return () => { canceled = true; };
  }, [props.pdf, props.documentId, retry]);

  useLayoutEffect(() => {
    const viewport = props.container.current;
    const frame = layout.pages[props.page];
    if (!viewport || !frame || !ready) return;
    const changedDocument = anchor.current.documentId !== props.documentId;
    const saved = props.initialPosition;
    if (changedDocument && saved && saved.page === props.page) {
      viewport.scrollTop = Math.max(0, frame.top + saved.fraction * frame.height);
      reported.current = props.page;
    } else if (changedDocument || reported.current !== props.page) {
      viewport.scrollTop = Math.max(0, frame.top - 24);
      reported.current = props.page;
    } else if (previousLayout.current !== layout && anchor.current.page === props.page) {
      viewport.scrollTop = Math.max(0, frame.top + anchor.current.fraction * frame.height);
    }
    previousLayout.current = layout;
    anchor.current = { documentId: props.documentId, page: props.page,
      fraction: (viewport.scrollTop - frame.top) / frame.height };
    callbacks.current.onPosition?.({ page: props.page, fraction: anchor.current.fraction });
    setPosition({ top: viewport.scrollTop, height: viewport.clientHeight });
  }, [props.page, props.documentId, props.container, layout, ready]);

  useEffect(() => {
    const frame = layout.pages[props.page];
    if (frame) props.onScale(frame.scale);
  }, [layout, props.page, props.onScale]);

  useEffect(() => {
    const viewport = props.container.current;
    if (!viewport) return;
    let scheduled = 0;
    const measure = () => {
      scheduled = 0;
      const pages = layoutRef.current.pages;
      if (!pages.length) return;
      const top = viewport.scrollTop, height = viewport.clientHeight;
      const page = pinned.current !== null && pages[pinned.current]
        ? pinned.current : visiblePage(pages, top, height);
      const frame = pages[page];
      anchor.current = { documentId: callbacks.current.documentId, page,
        fraction: (top - frame.top) / frame.height };
      callbacks.current.onPosition?.({ page, fraction: anchor.current.fraction });
      setPosition({ top, height });
      if (reported.current !== page) {
        reported.current = page;
        callbacks.current.onPage(page);
      }
    };
    const schedule = () => { if (!scheduled) scheduled = requestAnimationFrame(measure); };
    const release = () => { pinned.current = null; schedule(); };
    viewport.addEventListener("scroll", schedule, { passive: true });
    window.addEventListener("pointerup", release);
    window.addEventListener("pointercancel", release);
    return () => {
      cancelAnimationFrame(scheduled);
      viewport.removeEventListener("scroll", schedule);
      window.removeEventListener("pointerup", release);
      window.removeEventListener("pointercancel", release);
    };
  }, [props.container]);

  const rendered = renderPages(layout.pages, position.top, position.height);
  if (pinned.current !== null && layout.pages[pinned.current] && !rendered.includes(pinned.current)) rendered.push(pinned.current);
  return <div className="continuous-pages" aria-label="Continuously scrolling PDF pages"
    style={{ width: layout.width, height: layout.height }}>
    {!ready && <div className="continuous-loading" role={failure ? "alert" : "status"}>
      {failure ? <><p>{failure}</p><button className="button secondary" onClick={() => setRetry((value) => value + 1)}>Try again</button></>
        : <><LoaderCircle size={20} className="spin" /><span>{props.pdf ? `Preparing pages ${progress} / ${props.pdf.numPages}...` : "Loading PDF..."}</span></>}
    </div>}
    {rendered.map((index) => {
      const frame = layout.pages[index];
      return <div key={index} className="continuous-page-slot" data-page-slot={index + 1}
        style={{ top: frame.top, left: (layout.width - frame.width) / 2, width: frame.width, height: frame.height }}
        onPointerDownCapture={() => {
          pinned.current = index;
          reported.current = index;
          callbacks.current.onPage(index);
        }}
        onFocusCapture={() => {
          reported.current = index;
          callbacks.current.onPage(index);
        }}>
        {ready ? props.renderPage(index, frame) : null}
      </div>;
    })}
  </div>;
}
