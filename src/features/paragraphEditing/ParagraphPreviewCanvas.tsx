import { useEffect, useRef, useState } from "react";
import type { RenderTask } from "pdfjs-dist";
import { errorMessage } from "../../lib/geometry";
import { loadPdf } from "../../lib/pdf";

export default function ParagraphPreviewCanvas({ bytes }: { bytes: number[] }) {
  const host = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(240);
  const [error, setError] = useState<string | null>(null);
  const [rendering, setRendering] = useState(true);
  useEffect(() => {
    if (!host.current) return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry.contentRect.width > 0) setWidth(Math.floor(entry.contentRect.width));
    });
    observer.observe(host.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    let active = true;
    let render: RenderTask | undefined;
    const task = loadPdf(new Uint8Array(bytes));
    setError(null);
    setRendering(true);
    host.current?.replaceChildren();
    void task.promise.then(async (pdf) => {
      const page = await pdf.getPage(1);
      if (!active || !host.current) return;
      const base = page.getViewport({ scale: 1 });
      const scale = width / base.width;
      const viewport = page.getViewport({ scale });
      const pixels = Math.min(window.devicePixelRatio || 1, 2, 12_000 / viewport.height,
        Math.sqrt(4_000_000 / (viewport.width * viewport.height)));
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.ceil(viewport.width * pixels));
      canvas.height = Math.max(1, Math.ceil(viewport.height * pixels));
      canvas.style.width = `${viewport.width}px`;
      canvas.style.height = `${viewport.height}px`;
      canvas.setAttribute("role", "img");
      canvas.setAttribute("aria-label", "Actual PDF paragraph line-layout preview");
      host.current.replaceChildren(canvas);
      render = page.render({ canvas, viewport, transform: [pixels, 0, 0, pixels, 0, 0] });
      await render.promise;
      if (active) setRendering(false);
    }).catch((error: unknown) => { if (active) { setRendering(false); setError(errorMessage(error)); } });
    return () => {
      active = false;
      render?.cancel();
      void task.destroy().catch((error: unknown) => console.error("Could not release paragraph preview:", error));
    };
  }, [bytes, width]);
  return <div className="paragraph-pdf-preview" aria-busy={rendering}>
    {rendering && <p role="status">Rendering actual PDF preview...</p>}
    <div ref={host} />
    {error && <p className="paragraph-error" role="alert">PDF preview failed: {error}</p>}
  </div>;
}
