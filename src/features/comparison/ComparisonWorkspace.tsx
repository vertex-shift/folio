import { useEffect, useImperativeHandle, useRef, useState } from "react";
import type { Ref } from "react";
import { ArrowLeft, ChevronLeft, ChevronRight, FileDown, Files, FolderOpen, LoaderCircle, X } from "lucide-react";
import IconButton from "../../components/IconButton";
import { native } from "../../lib/bridge";
import { errorMessage } from "../../lib/geometry";
import type { DocumentInfo } from "../../lib/types";
import { comparisonBridge } from "./bridge";
import { runComparison } from "./engine";
import { assertComparisonCurrent, geometryChanges, pageChanged, pagePresence } from "./model";
import type { ComparisonPlan, ComparisonProgress, ComparisonResult, ComparisonSettings, PagePreview, PageResult, SourceState } from "./model";
import "./comparison.css";

export interface ComparisonControls { invalidate(documentId: number): void }
interface Props {
  documents: DocumentInfo[];
  activeId: number | null;
  open: boolean;
  blocked: boolean;
  current(documentId: number): SourceState;
  prepare(documentId: number): Promise<void>;
  onOpen(): void;
  onClose(): void;
  onActivity(running: boolean, exporting: boolean): void;
  onResourceError(message: string): void;
  ref?: Ref<ComparisonControls>;
}

const geometryLabels = {
  media_box: "MediaBox", effective_crop: "Effective crop", rotation: "Rotation",
  user_unit: "UserUnit", physical_size: "Physical page size",
};

function pageLabels(plan: ComparisonPlan, page: PageResult): string {
  const parts: string[] = [];
  const presence = pagePresence(plan, page.index);
  if (presence !== "matched") parts.push(presence === "added" ? "Added at this index" : "Removed at this index");
  if (page.text.spans.some((span) => span.kind !== "equal")) parts.push("Text");
  if (page.pixels.changed) parts.push("Pixels");
  if (geometryChanges(plan.before.pages[page.index], plan.after.pages[page.index]).length) parts.push("Geometry");
  return parts.join(" / ") || "No observed differences";
}

function PageImages({ preview, plan, page, overlay, mask, opacity }: {
  preview: PagePreview; plan: ComparisonPlan; page: PageResult; overlay: boolean; mask: boolean; opacity: number;
}) {
  const [urls, setUrls] = useState<{ before: string | null; after: string | null; mask: string } | null>(null);
  useEffect(() => {
    const values = { before: preview.before ? URL.createObjectURL(preview.before) : null,
      after: preview.after ? URL.createObjectURL(preview.after) : null, mask: URL.createObjectURL(preview.mask) };
    setUrls(values);
    return () => { for (const url of Object.values(values)) if (url) URL.revokeObjectURL(url); };
  }, [preview]);
  if (!urls) return <p role="status">Preparing page previews...</p>;
  const style = { aspectRatio: `${page.pixels.width} / ${page.pixels.height}` };
  return <div className={`comparison-images ${overlay ? "is-overlay" : ""}`}>
    {overlay ? <figure>
      <figcaption>Top-left aligned overlay, page {page.index + 1}</figcaption>
      <div className="comparison-paper" style={style}>
        {urls.before && <img src={urls.before} alt={`Before page ${page.index + 1}`} />}
        {urls.after && <img src={urls.after} alt={`After page ${page.index + 1}`} style={{ opacity: urls.before ? opacity / 100 : 1 }} />}
        {mask && <img src={urls.mask} alt="" className="comparison-mask" />}
      </div>
    </figure> : (["before", "after"] as const).map((side) => <figure key={side}>
      <figcaption><strong>{side === "before" ? "Before" : "After"}</strong><span>{plan[side].name}</span></figcaption>
      <div className="comparison-paper" style={style}>
        {urls[side] ? <img src={urls[side]} alt={`${side === "before" ? "Before" : "After"} page ${page.index + 1}`} />
          : <p>No page at this index</p>}
        {mask && urls[side] && <img src={urls.mask} alt="" className="comparison-mask" />}
      </div>
    </figure>)}
  </div>;
}

export default function ComparisonWorkspace(props: Props) {
  const [beforeId, setBeforeId] = useState<number | null>(null);
  const [afterId, setAfterId] = useState<number | null>(null);
  const [settings, setSettings] = useState<ComparisonSettings>({ dpi: 96, tolerance: 0 });
  const [result, setResult] = useState<ComparisonResult | null>(null);
  const [stale, setStale] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [progress, setProgress] = useState<ComparisonProgress | null>(null);
  const [running, setRunning] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [selectedPage, setSelectedPage] = useState(0);
  const [overlay, setOverlay] = useState(false);
  const [showMask, setShowMask] = useState(true);
  const [opacity, setOpacity] = useState(50);
  const [acknowledged, setAcknowledged] = useState(false);
  const latest = useRef(props);
  latest.current = props;
  const plan = useRef<ComparisonPlan | null>(null);
  const controller = useRef<AbortController | null>(null);
  const locked = useRef(false);
  const staleRef = useRef<string | null>(null);
  const startingSources = useRef<number[]>([]);
  const heading = useRef<HTMLHeadingElement>(null);

  function invalidate(id: number) {
    if (!startingSources.current.includes(id) && plan.current?.before.documentId !== id && plan.current?.after.documentId !== id) return;
    const message = "A source changed, acquired a draft, or closed. These results are stale; compare the current PDFs again.";
    staleRef.current = message;
    setStale(message);
    controller.current?.abort();
  }
  useImperativeHandle(props.ref, () => ({ invalidate }), []);

  useEffect(() => {
    props.onActivity(running, exporting);
  }, [running, exporting, props.onActivity]);
  useEffect(() => () => {
    controller.current?.abort();
    latest.current.onActivity(false, false);
    const current = plan.current;
    if (current) void comparisonBridge.cancel(current.comparisonId).catch((error: unknown) => {
      latest.current.onResourceError(`Comparison cleanup failed: ${errorMessage(error)}`);
    });
  }, []);

  useEffect(() => {
    if (!props.open) return;
    heading.current?.focus();
    if (beforeId === null) setBeforeId(props.activeId ?? props.documents[0]?.id ?? null);
  }, [props.open]);
  useEffect(() => {
    if (afterId === null && beforeId !== null) setAfterId(props.documents.find((document) => document.id !== beforeId)?.id ?? null);
  }, [props.documents, beforeId, afterId]);
  useEffect(() => {
    const current = plan.current;
    if (!current || staleRef.current) return;
    try { assertComparisonCurrent(current, props.current); }
    catch { invalidate(current.before.documentId); }
  }, [props.documents, props.current]);

  function check(current: ComparisonPlan) {
    if (staleRef.current) throw new Error(staleRef.current);
    assertComparisonCurrent(current, latest.current.current);
  }

  async function release() {
    const current = plan.current;
    if (current) await comparisonBridge.cancel(current.comparisonId);
    plan.current = null;
  }

  async function reset(action?: () => void) {
    if (locked.current) return;
    locked.current = true;
    setError(null);
    try {
      await release();
      setResult(null); setProgress(null); setStale(null); staleRef.current = null;
      setNotice(null); setAcknowledged(false);
      action?.();
    } catch (error) { setError(`Comparison cleanup failed: ${errorMessage(error)}`); }
    finally { locked.current = false; }
  }

  async function start() {
    if (locked.current || beforeId === null || afterId === null) return;
    locked.current = true;
    const abort = new AbortController();
    controller.current = abort;
    startingSources.current = [beforeId, afterId];
    setRunning(true); setError(null); setNotice(null);
    setResult(null); setStale(null); staleRef.current = null; setAcknowledged(false);
    try {
      await release();
      await Promise.all([latest.current.prepare(beforeId), latest.current.prepare(afterId)]);
      abort.signal.throwIfAborted();
      const before = latest.current.current(beforeId).document;
      const after = latest.current.current(afterId).document;
      if (!before || !after) throw new Error("Choose two open PDFs.");
      const references = { before: { documentId: before.id, revision: before.revision }, after: { documentId: after.id, revision: after.revision } };
      assertComparisonCurrent(references, latest.current.current);
      const current = await comparisonBridge.begin(
        { documentId: before.id, revision: before.revision }, { documentId: after.id, revision: after.revision }, settings,
      );
      plan.current = current;
      check(current); abort.signal.throwIfAborted();
      const compared = await runComparison(current, {
        bytes: (side, signal) => comparisonBridge.bytes(current.comparisonId, side, signal),
        check: () => check(current),
        verify: async () => { await comparisonBridge.check(current.comparisonId); },
      }, abort.signal, setProgress);
      check(current); abort.signal.throwIfAborted();
      await comparisonBridge.finish(current.comparisonId, compared.pages, abort.signal);
      check(current);
      await comparisonBridge.check(current.comparisonId);
      check(current); abort.signal.throwIfAborted();
      setResult(compared);
      setSelectedPage(compared.pages.findIndex((page) => pageChanged(current, page)) >= 0
        ? compared.pages.findIndex((page) => pageChanged(current, page)) : 0);
    } catch (error) {
      let message = abort.signal.aborted ? staleRef.current ?? "Comparison canceled. Neither source PDF was changed." : errorMessage(error);
      try { await release(); }
      catch (cleanup) { message += ` Cleanup failed: ${errorMessage(cleanup)}. Clear the comparison to retry.`; }
      setError(message);
    } finally {
      controller.current = null; startingSources.current = [];
      setRunning(false); setProgress(null); locked.current = false;
    }
  }

  async function save() {
    const current = plan.current;
    if (!current || !result || locked.current) return;
    locked.current = true;
    const abort = new AbortController();
    controller.current = abort;
    setExporting(true); setError(null); setNotice(null);
    try {
      check(current);
      await Promise.all([latest.current.prepare(current.before.documentId), latest.current.prepare(current.after.documentId)]);
      check(current); abort.signal.throwIfAborted();
      await comparisonBridge.check(current.comparisonId);
      check(current); abort.signal.throwIfAborted();
      const saved = await comparisonBridge.save(current, acknowledged, abort.signal, () => check(current));
      check(current);
      setNotice(saved ? `${native ? "Report saved" : "Report download started"}. Neither source PDF, history nor reading position was changed.` : "Report export canceled. Both PDFs are unchanged.");
    } catch (error) {
      setError(errorMessage(error));
      try { await comparisonBridge.check(current.comparisonId); }
      catch { invalidate(current.before.documentId); }
    } finally { controller.current = null; locked.current = false; setExporting(false); }
  }

  const before = props.documents.find((document) => document.id === beforeId);
  const after = props.documents.find((document) => document.id === afterId);
  const changed = result?.pages.filter((page) => pageChanged(result.plan, page)).map((page) => page.index) ?? [];
  const selected = result?.pages[selectedPage];
  const protectedInput = result ? result.plan.before.passwordProtected || result.plan.after.passwordProtected : Boolean(before?.passwordProtected || after?.passwordProtected);
  let blockedReason: string | null = null;
  if (!before || !after) blockedReason = "Choose two open PDF tabs.";
  else if (beforeId === afterId) blockedReason = "Choose two different PDF tabs.";
  else {
    try { assertComparisonCurrent({ before: { documentId: before.id, revision: before.revision }, after: { documentId: after.id, revision: after.revision } }, props.current); }
    catch (error) { blockedReason = errorMessage(error); }
  }
  const disabled = running || exporting || props.blocked;

  return <section className="comparison-workspace" hidden={!props.open} aria-label="PDF document comparison">
    <header className="comparison-header">
      <button className="button secondary" disabled={exporting} onClick={props.onClose}><ArrowLeft size={15} />Return to PDFs</button>
      <h1 ref={heading} tabIndex={-1}>Compare PDFs</h1>
      <span className="comparison-local"><Files size={15} />Local working snapshots</span>
    </header>
    <div className="comparison-layout">
      <aside className="comparison-sidebar" aria-label="Comparison settings and pages">
        <div className="comparison-settings">
          <p>Compare two open tabs, including their applied but unsaved edits. Neither source is modified.</p>
          {(["before", "after"] as const).map((side) => {
            const id = side === "before" ? beforeId : afterId;
            return <label className="setting-field" key={side}>{side === "before" ? "Before PDF" : "After PDF"}
              <select aria-label={side === "before" ? "Before PDF" : "After PDF"} title={side === "before" ? before?.name : after?.name}
                value={id ?? ""} disabled={disabled} onChange={(event) => {
                const value = Number(event.target.value);
                if (!Number.isSafeInteger(value) || value < 1) return;
                void reset(() => side === "before" ? setBeforeId(value) : setAfterId(value));
              }}>
                <option value="" disabled>Choose an open PDF</option>
                {id !== null && !props.documents.some((document) => document.id === id) && <option value={id}>Closed PDF tab</option>}
                {props.documents.map((document, index) => <option key={document.id} value={document.id}>{document.name}{document.dirty ? " (unsaved)" : ""} - tab {index + 1}</option>)}
              </select>
            </label>;
          })}
          <div className="comparison-setting-pair">
            <label className="setting-field">Render resolution<select aria-label="Render resolution" disabled={disabled} value={settings.dpi} onChange={(event) => {
              const dpi = Number(event.target.value);
              if (dpi === 72 || dpi === 96 || dpi === 150) void reset(() => setSettings((current) => ({ ...current, dpi })));
            }}><option value={72}>72 DPI</option><option value={96}>96 DPI</option><option value={150}>150 DPI</option></select></label>
            <label className="setting-field">RGB tolerance<select aria-label="RGB tolerance" disabled={disabled} value={settings.tolerance} onChange={(event) => {
              const tolerance = Number(event.target.value);
              if ([0, 4, 12, 32, 64].includes(tolerance)) void reset(() => setSettings((current) => ({ ...current, tolerance })));
            }}>{[0, 4, 12, 32, 64].map((value) => <option key={value} value={value}>{value} / 255{value === 0 ? " (exact)" : ""}</option>)}</select></label>
          </div>
          <p className="comparison-method">Page 1 matches page 1, not a semantic or moved-page match. A middle insertion shifts later pairs; only unmatched trailing indexes are labeled added/removed.</p>
          {blockedReason && <p className="comparison-blocked" role="status">{blockedReason}</p>}
          <div className="comparison-start-actions">
            <button className="button primary" disabled={disabled || Boolean(blockedReason)} onClick={() => { void start(); }}>Compare working PDFs</button>
            {(result || stale || error) && !running && <button className="button secondary" disabled={exporting} onClick={() => { void reset(); }}>Clear comparison</button>}
            {running && <button className="button secondary" onClick={() => controller.current?.abort()}>Cancel comparison</button>}
          </div>
          {running && <div className="comparison-progress" role="status"><LoaderCircle size={16} className="spin" />
            <span>{progress?.stage === "snapshots" ? "Preparing guarded snapshots..."
              : progress?.stage === "finishing" ? "Final source checks..." : `${progress?.stage === "text" ? "Extracting text" : "Comparing pixels"}: page ${progress?.page ?? 0} of ${progress?.count ?? 0}`}</span></div>}
        </div>
        {result && <nav className="comparison-page-list" aria-label="Compared pages">
          <h2>Indexed pages <span>{result.pages.length}</span></h2>
          {result.pages.map((page) => <button key={page.index} aria-current={selectedPage === page.index ? "page" : undefined}
            className={selectedPage === page.index ? "selected" : ""} onClick={() => setSelectedPage(page.index)}>
            <strong>Page {page.index + 1}</strong><span>{pageLabels(result.plan, page)}</span>
          </button>)}
        </nav>}
        <details className="comparison-limits"><summary>Method and limits</summary>
          <p>Visible page pixels, embedded text in PDF.js extraction order (including off-crop or invisible text returned by extraction), and effective page geometry. No OCR or inferred reading order. Text normalization is disabled; PDF.js still decodes source encodings.</p>
          <p>White background, top-left alignment, no scaling to fit mismatched pages. A pixel differs only when an RGB channel delta exceeds the tolerance. Geometry is checked independently, without a pixel tolerance.</p>
          <p>Metadata, attachments, bookmarks, signatures/trust and other hidden objects are not compared. Annotation/form appearances contribute only where rendered. Matching results never prove document identity.</p>
          <p>100 pages and 64 MiB per PDF; 8 megapixels per aligned pair, 100 megapixels total; 256 KiB/20,000 tokens per page side, 2 MiB/100,000 tokens/4,000 text spans overall; 32 MiB previews and 4 MiB report. Sequential processing, one comparison. Oversized/complex work is refused, not silently truncated.</p>
        </details>
      </aside>
      <main className="comparison-main">
        {error && <div className="comparison-message is-error" role="alert"><span>{error}</span>
          <IconButton label="Dismiss comparison error" onClick={() => setError(null)}><X size={15} /></IconButton></div>}
        {stale && <p className="comparison-message is-stale" role="status"><strong>Stale comparison.</strong> {stale}</p>}
        {notice && <p className="comparison-message" role="status">{notice}</p>}
        {result && selected ? <>
          <div className="comparison-summary">
            <div><h2>{changed.length ? `${changed.length} indexed ${changed.length === 1 ? "page has" : "pages have"} differences` : "No observed differences in compared content"}</h2>
              <p>Text, pixels and geometry only. <strong>Not a claim that the PDFs are identical.</strong></p></div>
            <div className="comparison-change-navigation">
              <IconButton label="Previous changed page" disabled={!changed.some((index) => index < selectedPage)}
                onClick={() => setSelectedPage(changed.filter((index) => index < selectedPage).at(-1) ?? selectedPage)}><ChevronLeft size={17} /></IconButton>
              <span>Page {selectedPage + 1} / {result.pages.length}</span>
              <IconButton label="Next changed page" disabled={!changed.some((index) => index > selectedPage)}
                onClick={() => setSelectedPage(changed.find((index) => index > selectedPage) ?? selectedPage)}><ChevronRight size={17} /></IconButton>
            </div>
          </div>
          <div className="comparison-view-controls">
            <div className="comparison-view-choice" role="group" aria-label="Comparison view">
              <button className={`button ${!overlay ? "primary" : "secondary"}`} aria-pressed={!overlay} onClick={() => setOverlay(false)}>Side by side</button>
              <button className={`button ${overlay ? "primary" : "secondary"}`} aria-pressed={overlay} onClick={() => setOverlay(true)}>Overlay</button>
            </div>
            <label><input type="checkbox" checked={showMask} onChange={(event) => setShowMask(event.target.checked)} /><span className="comparison-mask-swatch" />Show changed pixels</label>
            {overlay && <label>After opacity<input aria-label="After opacity" type="range" min={0} max={100} step={10} value={opacity}
              onChange={(event) => setOpacity(Number(event.target.value))} /><span>{opacity}%</span></label>}
          </div>
          <div className="comparison-page-detail">
            <h3>Page {selectedPage + 1}: {pageLabels(result.plan, selected)}</h3>
            <p>{selected.pixels.changed.toLocaleString()} / {selected.pixels.total.toLocaleString()} pixels differ at {result.plan.settings.dpi} DPI,
              RGB tolerance {result.plan.settings.tolerance}/255. Previews fit within 1,200 px; comparison uses the stated full resolution.</p>
            {geometryChanges(result.plan.before.pages[selectedPage], result.plan.after.pages[selectedPage]).length > 0 && <p className="comparison-geometry">
              Geometry differs: {geometryChanges(result.plan.before.pages[selectedPage], result.plan.after.pages[selectedPage]).map((key) => geometryLabels[key]).join(", ")}.
              {" "}{([["Before", result.plan.before.pages[selectedPage]], ["After", result.plan.after.pages[selectedPage]]] as const).map(([label, geometry]) =>
                geometry ? `${label}: ${geometry.widthPoints.toFixed(2)} x ${geometry.heightPoints.toFixed(2)} pt, ${geometry.rotation} degrees, UserUnit ${geometry.userUnit}. ` : "")}
            </p>}
          </div>
          {props.open && <PageImages key={selectedPage} preview={result.previews[selectedPage]} plan={result.plan} page={selected}
            overlay={overlay} mask={showMask} opacity={opacity} />}
          <section className="comparison-text" aria-label="Embedded text comparison">
            <h3>Embedded text differences</h3>
            {!selected.text.beforeTokens && !selected.text.afterTokens ? <p>No embedded text on this indexed page pair. Pixel and geometry results still apply; no OCR was run.</p>
              : <><p>Exact extracted strings and line endings, not paragraph layout. <del>Removed from Before</del> / <ins>Added in After</ins>.</p>
                {selected.text.spans.some((span) => span.kind !== "equal")
                  ? <pre>{selected.text.spans.map((span, index) => span.kind === "removed" ? <del key={index}>{span.text}</del>
                    : span.kind === "added" ? <ins key={index}>{span.text}</ins> : <span key={index}>{span.text}</span>)}</pre>
                  : <details><summary>No extracted-text differences. Read compared text</summary><pre>{selected.text.spans.map((span) => span.text).join("")}</pre></details>}</>}
          </section>
          <footer className="comparison-export">
            <div><h3>Separate comparison report</h3><p>Bounded JSON with per-page text spans, pixel counts/bounds and geometry. No page images. This is not a complete PDF-object or signature comparison.</p>
              {protectedInput && <label className="comparison-protection-warning"><input type="checkbox" checked={acknowledged}
                disabled={exporting} onChange={(event) => setAcknowledged(event.target.checked)} />
                I understand the report contains unencrypted extracted content from a protected PDF and retains no passwords, permissions or signatures.</label>}</div>
            <button className="button secondary" disabled={disabled || Boolean(stale) || protectedInput && !acknowledged}
              onClick={() => { void save(); }}><FileDown size={15} />{exporting ? "Exporting report..." : "Export JSON report"}</button>
          </footer>
        </> : !running && <div className="comparison-empty"><Files size={30} /><h2>Review what changed between two PDFs</h2>
          <p>Select Before and After tabs, then compare their current working snapshots. Results stay here when you return to your documents.</p>
          <p>Page-index matching, embedded text, rendered pixels and geometry. Metadata, attachments and signatures are outside this comparison.</p>
          {props.documents.length < 2 && <button className="button secondary" disabled={props.blocked} onClick={props.onOpen}><FolderOpen size={15} />Open another PDF</button>}
        </div>}
      </main>
    </div>
  </section>;
}
