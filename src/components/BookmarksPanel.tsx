import { useEffect, useState } from "react";
import { Bookmark, ChevronRight, LoaderCircle } from "lucide-react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import { flattenBookmarks } from "../lib/navigation";
import type { Bookmark as Entry } from "../lib/navigation";
import { errorMessage } from "../lib/geometry";

export default function BookmarksPanel({ pdf, onNavigate, disabled }: {
  pdf: PDFDocumentProxy | null; disabled: boolean;
  onNavigate(destination: unknown, url: string | null): void;
}) {
  const [entries, setEntries] = useState<Entry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    setEntries([]); setError(null);
    if (!pdf) return;
    setLoading(true);
    void pdf.getOutline().then((outline) => {
      if (!cancelled) { setEntries(flattenBookmarks(outline)); setLoading(false); }
    }).catch((error: unknown) => {
      if (!cancelled) { setError(errorMessage(error)); setLoading(false); }
    });
    return () => { cancelled = true; };
  }, [pdf]);
  return <div className="panel-scroll bookmarks-panel">
    {loading && <p className="panel-progress" role="status"><LoaderCircle size={14} className="spin" />Reading bookmarks...</p>}
    {error && <p className="inline-error" role="alert">{error}</p>}
    {!loading && !entries.length && !error && <div className="panel-empty"><Bookmark size={28} /><h3>No bookmarks in this PDF</h3><p>Document bookmarks appear here when the PDF contains an outline.</p></div>}
    {entries.map((entry, index) => <button className="bookmark-entry" key={index}
      style={{ paddingLeft: 20 + Math.min(entry.depth, 6) * 13 }} disabled={disabled}
      onClick={() => onNavigate(entry.destination, entry.url)}><Bookmark size={14} /><span>{entry.title}</span><ChevronRight size={13} /></button>)}
  </div>;
}
