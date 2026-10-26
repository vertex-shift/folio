import { useEffect, useRef, useState } from "react";
import { FileDown, LoaderCircle, Printer } from "lucide-react";
import Dialog from "./Dialog";
import { native } from "../lib/bridge";
import { errorMessage, formatBytes } from "../lib/geometry";
import { loadPdf } from "../lib/pdf";
import { outputBridge } from "../lib/outputBridge";
import { PreparedPrintSurface } from "../lib/printSurface";
import { assertOutputCurrent, outputPages, renderOutputImages, renderOutputText } from "../lib/output";
import type { ExportFormat, OptimizationReport, OutputAction, OutputDpi, OutputPlan, OutputProgress } from "../lib/output";
import type { DocumentInfo } from "../lib/types";
import { extractDocxPages } from "../features/conversion/docx";
import type { DocxTextSummary } from "../features/conversion/docx";
import { MAX_DOCX_SOURCE_BYTES } from "../features/conversion/model";

type Phase = "settings" | "preparing" | "saving" | "print-ready" | "optimize-ready" | "docx-ready" | "cleanup";

interface Props {
  info: DocumentInfo | null;
  mode: OutputAction | null;
  busy: boolean;
  pendingDraft: boolean;
  error: string | null;
  current(): DocumentInfo | null;
  run(label: string, task: () => Promise<void>): Promise<void>;
  onClearError(): void;
  onResourceError(message: string): void;
  onNotice(message: string): void;
  onClose(): void;
}

function exactSize(bytes: number): string {
  return `${formatBytes(bytes)} (${bytes.toLocaleString()} bytes)`;
}

export default function OutputDialog(props: Props) {
  const [allPages, setAllPages] = useState(true);
  const [range, setRange] = useState("");
  const [format, setFormat] = useState<ExportFormat>("png");
  const [dpi, setDpi] = useState<OutputDpi>(150);
  const [quality, setQuality] = useState(90);
  const [acknowledged, setAcknowledged] = useState(false);
  const [phase, setPhase] = useState<Phase>("settings");
  const [progress, setProgress] = useState<OutputProgress | null>(null);
  const [canceling, setCanceling] = useState(false);
  const [report, setReport] = useState<OptimizationReport | null>(null);
  const [docxSummary, setDocxSummary] = useState<DocxTextSummary | null>(null);
  const [printRequested, setPrintRequested] = useState(false);
  const job = useRef<OutputPlan | null>(null);
  const print = useRef<PreparedPrintSurface | null>(null);
  const controller = useRef<AbortController | null>(null);
  const latest = useRef(props);
  latest.current = props;

  useEffect(() => {
    if (!props.mode || job.current) return;
    setAcknowledged(false);
    setPhase("settings");
    setReport(null);
    setDocxSummary(null);
    setPrintRequested(false);
    if (props.mode === "print" && !props.info?.permissions.highQualityPrintAllowed && dpi > 150) setDpi(150);
  }, [props.mode]);

  useEffect(() => () => {
    controller.current?.abort();
    if (print.current?.requested || print.current?.printing) return;
    print.current?.dispose();
    const plan = job.current;
    if (plan) void outputBridge.cancel(plan.jobId).catch((error: unknown) => {
      latest.current.onResourceError(`Output cleanup failed: ${errorMessage(error)}`);
    });
  }, []);

  function check(plan: OutputPlan) {
    assertOutputCurrent(plan, latest.current.current(), latest.current.pendingDraft);
  }

  async function release(confirmedPrintFinished = false) {
    print.current?.dispose(confirmedPrintFinished);
    print.current = null;
    const plan = job.current;
    if (plan) await outputBridge.cancel(plan.jobId);
    job.current = null;
    setProgress(null);
  }

  function close() {
    void props.run("Closing prepared output...", async () => {
      await release(phase === "print-ready" && printRequested);
      setPhase("settings");
      props.onClose();
    });
  }

  async function save(plan: OutputPlan, signal: AbortSignal) {
    check(plan);
    signal.throwIfAborted();
    await outputBridge.check(plan.jobId);
    check(plan);
    signal.throwIfAborted();
    setPhase("saving");
    const saved = await outputBridge.save(plan, signal);
    if (!saved) {
      if (plan.kind === "optimize" || plan.kind === "docx") setPhase(plan.kind === "docx" ? "docx-ready" : "optimize-ready");
      else { await release(); setPhase("settings"); }
      props.onNotice("Save canceled. Your PDF, unsaved work and other tabs are unchanged.");
      return;
    }
    const summary = plan.kind === "optimize" && report
      ? report.reduced
        ? `Optimized copy: ${report.beforeBytes.toLocaleString()} to ${report.afterBytes.toLocaleString()} bytes.`
        : `No size reduction. The copy retains the original ${report.beforeBytes.toLocaleString()} bytes.`
      : plan.kind === "docx"
        ? `Editable document text from ${plan.pages.length} ${plan.pages.length === 1 ? "page" : "pages"} exported as DOCX. PDF layout and signatures are not retained.`
        : plan.kind === "text"
        ? `Selectable text from ${plan.pages.length} ${plan.pages.length === 1 ? "page" : "pages"} exported as UTF-8.`
        : `${plan.pages.length} ${plan.kind === "png" ? "PNG" : "JPEG"} ${plan.pages.length === 1 ? "page" : "pages in a ZIP"} exported.`;
    await release();
    setPhase("settings");
    props.onClose();
    props.onNotice(`${native ? summary : `Download started. ${summary}`} Your source PDF is unchanged.`);
  }

  function prepare() {
    const mode = props.mode;
    if (!mode) return;
    void props.run(mode === "optimize" ? "Measuring lossless optimization..." : mode === "docx" ? "Extracting document text..." : "Preparing document output...", async () => {
      const info = props.current();
      if (!info) throw new Error("Open a PDF before printing or exporting.");
      if (job.current) throw new Error("Close the previous prepared output before preparing it again.");
      const kind = mode === "export" ? format : mode;
      if (kind === "docx" && info.byteLength > MAX_DOCX_SOURCE_BYTES) throw new Error("DOCX conversion requires a source snapshot no larger than 64 MiB.");
      assertOutputCurrent({ documentId: info.id, revision: info.revision }, info, props.pendingDraft);
      const pages = mode === "optimize" ? [] : outputPages(allPages, range, info.pageCount, kind);
      const abort = new AbortController();
      controller.current = abort;
      setPhase("preparing");
      setCanceling(false);
      setProgress({ page: 0, count: pages.length });
      let failure: unknown;
      try {
        const plan = await outputBridge.begin({
          documentId: info.id, revision: info.revision, kind, pages,
          dpi: kind === "text" || kind === "docx" || kind === "optimize" ? null : dpi,
          quality: kind === "jpeg" ? quality : null,
          allowUnencrypted: acknowledged,
        });
        job.current = plan;
        abort.signal.throwIfAborted();
        check(plan);
        if (kind === "optimize") {
          const measured = await outputBridge.optimize(plan.jobId);
          abort.signal.throwIfAborted();
          check(plan);
          await outputBridge.check(plan.jobId);
          setReport(measured);
          setPhase("optimize-ready");
          return;
        }
        const bytes = await outputBridge.renderBytes(plan.jobId, abort.signal);
        check(plan);
        abort.signal.throwIfAborted();
        const loading = loadPdf(bytes);
        let destroyed: Promise<void> | null = null;
        const destroy = () => destroyed ??= loading.destroy();
        const abortLoading = () => {
          void destroy().catch((error: unknown) => latest.current.onResourceError(`Output renderer cleanup failed: ${errorMessage(error)}`));
        };
        abort.signal.addEventListener("abort", abortLoading, { once: true });
        try {
          const snapshot = await loading.promise;
          abort.signal.throwIfAborted();
          check(plan);
          if (snapshot.numPages !== info.pageCount) throw new Error("The renderer and output snapshot disagree on the page count. Reopen this PDF.");
          if (kind === "docx") {
            setDocxSummary(await extractDocxPages(snapshot, plan, outputBridge.stage, setProgress, abort.signal, () => check(plan)));
          } else if (kind === "text") {
            await renderOutputText(snapshot, plan, outputBridge.stage, setProgress, abort.signal, () => check(plan));
          } else {
            if (kind === "print") print.current = new PreparedPrintSurface();
            await renderOutputImages(snapshot, plan, outputBridge.stage, setProgress, abort.signal,
              () => check(plan), kind === "print" ? (image) => print.current!.append(image) : undefined);
          }
        } finally {
          abort.signal.removeEventListener("abort", abortLoading);
          await destroy();
        }
        abort.signal.throwIfAborted();
        check(plan);
        await outputBridge.check(plan.jobId);
        if (kind === "print") {
          print.current!.finish(plan.pages.length);
          setPhase("print-ready");
        } else if (kind === "docx") {
          setPhase("docx-ready");
        } else {
          await save(plan, abort.signal);
        }
        return;
      } catch (error) {
        failure = error;
      } finally {
        controller.current = null;
        setCanceling(false);
      }
      try {
        await release();
        setPhase("settings");
      } catch (error) {
        setPhase("cleanup");
        throw new Error(`${abort.signal.aborted ? "Output canceled." : errorMessage(failure)} Output cleanup failed: ${errorMessage(error)}`);
      }
      if (abort.signal.aborted) {
        props.onNotice("Output preparation canceled. Your source, drafts and other tabs are unchanged.");
      } else {
        throw failure;
      }
    });
  }

  function savePreparedCopy() {
    void props.run(props.mode === "docx" ? "Saving document-content DOCX..." : "Saving optimized PDF copy...", async () => {
      const plan = job.current;
      if (!plan || plan.kind !== "optimize" && plan.kind !== "docx") throw new Error("Prepare the copy again.");
      const abort = new AbortController();
      try { await save(plan, abort.signal); }
      finally { if (job.current) setPhase(plan.kind === "docx" ? "docx-ready" : "optimize-ready"); }
    });
  }

  function openPrint() {
    void props.run("Opening platform print dialog...", async () => {
      const plan = job.current;
      const surface = print.current;
      if (!plan || !surface) throw new Error("Prepare the print snapshot again.");
      check(plan);
      surface.markRequested();
      await outputBridge.print(plan.jobId);
      setPrintRequested(true);
    });
  }

  if (!props.mode || !props.info) return null;
  const { info, mode } = props;
  const preparing = phase === "preparing";
  const prepared = phase === "print-ready" || phase === "optimize-ready" || phase === "docx-ready";
  const unencrypted = (mode === "export" || mode === "docx") && info.passwordProtected;
  const title = mode === "print" ? "Print PDF pages" : mode === "optimize" ? "Optimize a PDF copy"
    : mode === "docx" ? "Export Word document content" : "Export images or text";
  const locked = props.busy || prepared || phase === "cleanup";
  const selection = allPages ? `All ${info.pageCount} pages` : `Selected pages: ${range}`;

  return <Dialog className="output-dialog" title={title} busy={locked} error={props.error} onClose={close}
    actions={preparing
      ? <button className="button secondary" disabled={canceling} onClick={() => {
        controller.current?.abort(); setCanceling(true);
      }}>{canceling ? "Canceling..." : "Cancel preparation"}</button>
      : phase === "saving"
        ? <button className="button secondary" disabled>{native ? "Cancel in the Save dialog" : "Finishing download..."}</button>
        : phase === "print-ready"
          ? <><button className="button secondary" disabled={props.busy} onClick={close}>{printRequested ? "Finished with print dialog" : "Discard snapshot"}</button>
            <button className="button primary" disabled={props.busy} onClick={openPrint}><Printer size={15} />{printRequested ? "Open print dialog again" : "Open print dialog"}</button></>
          : phase === "optimize-ready"
            ? <><button className="button secondary" disabled={props.busy} onClick={close}>Cancel</button>
              <button className="button primary" disabled={props.busy} onClick={savePreparedCopy}><FileDown size={15} />{report?.reduced ? "Save optimized copy" : "Save unchanged copy"}</button></>
            : phase === "docx-ready"
              ? <><button className="button secondary" disabled={props.busy} onClick={close}>Cancel</button>
                <button className="button primary" disabled={props.busy} onClick={savePreparedCopy}><FileDown size={15} />Save DOCX copy</button></>
            : phase === "cleanup"
              ? <button className="button secondary" disabled={props.busy} onClick={close}>Retry cleanup &amp; close</button>
              : <><button className="button secondary" disabled={props.busy} onClick={close}>Cancel</button>
                <button className="button primary" disabled={props.busy || unencrypted && !acknowledged || !allPages && mode !== "optimize" && !range.trim()}
                  onClick={prepare}>{mode === "print" ? <Printer size={15} /> : <FileDown size={15} />}
                  {mode === "print" ? "Prepare print" : mode === "optimize" ? "Measure optimization" : mode === "docx" ? "Prepare DOCX" : "Export copy"}</button></>}>
    <div className="output-settings">
      {mode === "optimize" ? <>
        <p>Compress eligible uncompressed streams and remove unreachable objects in a <strong>separate copy</strong>.
          Existing image codecs, fonts, page content, forms, bookmarks and metadata are not downsampled or flattened.</p>
        <p className="muted">No guaranteed reduction, PDF/A conversion or linearization. Signed PDFs are not rewritten.
          If the candidate is not smaller, only an unchanged copy is offered.</p>
        {info.passwordProtected && <p className="output-disclosure">The optimized copy keeps the source PDF's passwords, encryption and permissions.</p>}
      </> : <>
        <p>{mode === "print"
          ? "Prepare every selected page, including pages not on screen, then open the platform print dialog. Only the page images are printed."
          : mode === "docx" ? "Convert actual extracted PDF text into an editable Word document (.docx), in your selected page order. The source PDF is never rewritten."
          : "Export actual PDF page pixels or selectable page text without changing the working document."}</p>
        {phase === "settings" && <>
          {mode === "export" && <label className="setting-field">Format<select data-autofocus value={format} disabled={props.busy} onChange={(event) => {
            const value = event.target.value;
            if (value === "png" || value === "jpeg" || value === "text") { setFormat(value); props.onClearError(); }
          }}><option value="png">PNG images (lossless pixels)</option><option value="jpeg">JPEG images (smaller, lossy)</option><option value="text">Selectable text (.txt)</option></select></label>}
          <fieldset className="output-scope" disabled={props.busy}><legend>Pages</legend>
            <div className="output-scope-options">
              <label><input type="radio" name={`output-pages-${info.id}`} checked={allPages} onChange={() => setAllPages(true)} />All pages</label>
              <label><input type="radio" name={`output-pages-${info.id}`} checked={!allPages} onChange={() => setAllPages(false)} />Page range</label>
            </div>
            {!allPages && <label className="setting-field">Page numbers<input type="text" value={range} maxLength={4096}
              placeholder={`1-${info.pageCount}`} onChange={(event) => setRange(event.target.value)} /></label>}
          </fieldset>
          {(mode === "print" || mode === "export" && format !== "text") && <label className="setting-field">Resolution<select value={dpi} disabled={props.busy}
            onChange={(event) => {
              const value = Number(event.target.value);
              if (value === 96 || value === 150 || value === 300) setDpi(value);
            }}>
            <option value={96}>96 DPI - compact</option><option value={150}>150 DPI - standard</option>
            <option value={300} disabled={mode === "print" && !info.permissions.highQualityPrintAllowed}>300 DPI - high resolution</option>
          </select></label>}
          {mode === "export" && format === "jpeg" && <label className="setting-field">JPEG quality<select value={quality} disabled={props.busy}
            onChange={(event) => setQuality(Number(event.target.value))}>
            <option value={60}>60% - compact</option><option value={80}>80% - balanced</option>
            <option value={90}>90% - high</option><option value={100}>100% - maximum</option>
          </select></label>}
        </>}
        {mode === "print" ? <>
          <p className="muted">Raster printing at {dpi} DPI, not vector/native-PDF printing. Crops, rotations and physical page sizes come from the PDF renderer.
            In the print dialog, check paper sizes/scaling and turn off browser headers and footers.</p>
          {!info.permissions.highQualityPrintAllowed && <p className="output-disclosure">This PDF permits low-resolution printing only (up to 150 DPI).</p>}
          {info.passwordProtected && <p className="muted">A printer or Print-to-PDF driver receives page images, not the encrypted original or its signatures.</p>}
          <p className="muted">Up to 200 selected pages, 100 megapixels in the prepared snapshot, and 24 megapixels per page.
            Oversized jobs are refused rather than silently lowering quality.</p>
        </> : mode === "docx" ? <>
          <p className="output-disclosure"><strong>Text / document-content conversion, not high-fidelity Word layout.</strong><br />
            PDF.js extraction order becomes plain paragraphs with explicit page breaks. Columns, reading order, line wrapping and page counts may differ in Word.
            No table/chart reconstruction, images, OCR, source-PDF paragraph editing, full Office import/export, form-field values, annotations or signature preservation.</p>
          <p className="muted">Use page numbers/ranges in the desired order, such as 3, 1-2. Up to 200 selected pages, 64 MiB source snapshot,
            256 KiB text per page and 8 MiB total text. Empty scans and unsupported XML characters are refused, not replaced or silently omitted.</p>
          {unencrypted && <div className="output-disclosure"><strong>The DOCX is unencrypted.</strong>
            <p>It retains no PDF passwords, permissions or cryptographic signatures.</p>
            <label className="checkbox-setting"><input type="checkbox" disabled={props.busy} checked={acknowledged}
              onChange={(event) => setAcknowledged(event.target.checked)} />I understand and want to export unencrypted document content.</label>
          </div>}
        </> : <>
          <p className="muted">{format === "text"
            ? "UTF-8 text in PDF reading order, with a form-feed separator between selected pages. No OCR, layout, comments, images or form-field values are included."
            : "Multiple pages become one ZIP with numbered image files in the selected order. Images have no PDF text layer, forms, bookmarks or digital signatures."}</p>
          <p className="muted">Use page numbers/ranges such as 1-3, 5. Up to 500 pages, 24 megapixels per image, 128 MiB output or 16 MiB text.</p>
          {unencrypted && <div className="output-disclosure"><strong>This export removes the protection boundary.</strong>
            <p>PNG, JPEG, ZIP and text files are unencrypted. They do not retain PDF passwords, permissions or signatures.</p>
            <label className="checkbox-setting"><input type="checkbox" disabled={props.busy} checked={acknowledged}
              onChange={(event) => setAcknowledged(event.target.checked)} />I understand and want to export unencrypted content.</label>
          </div>}
        </>}
      </>}
      {preparing && <div className="output-progress" role="status" aria-live="polite"><span><LoaderCircle className="spin" size={15} />
        {canceling ? "Canceling without saving a copy..."
          : mode === "optimize" ? "Measuring safe compression and unused-object cleanup..."
            : progress?.page ? `${mode === "docx" || format === "text" && mode === "export" ? "Reading text from" : "Rendering"} selected page ${progress.page} of ${progress.count}...`
              : "Loading an isolated document snapshot..."}</span>
        {mode !== "optimize" && <progress value={progress?.page ?? 0} max={progress?.count || 1} aria-label="Output preparation progress" />}
      </div>}
      {phase === "docx-ready" && docxSummary && <div className="output-result" role="status">
        <h3>Editable document text is ready</h3>
        <p className="output-selection">{selection}. {docxSummary.pagesWithText} pages contain editable text; {formatBytes(docxSummary.byteLength)} of Unicode text.</p>
        <p>Selected pages are separated by explicit page breaks. This is a separate, unencrypted DOCX, not a layout-preserving copy of the PDF.</p>
      </div>}
      {phase === "print-ready" && <div className="output-result" role="status">
        <h3>{printRequested ? "Print dialog requested" : "Print snapshot ready"}</h3>
        <p className="output-selection">{selection} at {dpi} DPI.</p>
        <p>{printRequested ? "Folio cannot tell whether the platform printed or canceled. Keep this window open until the print dialog and its job have finished, then choose Finished with print dialog."
          : "The complete snapshot is retained while the dialog is open. Your original PDF and other tabs are not part of the print surface."}</p>
      </div>}
      {report && <div className="output-result" role="status">
        <h3>{report.reduced ? "A smaller, lossless copy is ready" : "No size reduction"}</h3>
        <dl className="output-sizes"><dt>Working PDF</dt><dd>{exactSize(report.beforeBytes)}</dd>
          <dt>Copy to save</dt><dd>{exactSize(report.afterBytes)}</dd></dl>
        <p>{report.reduced ? `${(report.beforeBytes - report.afterBytes).toLocaleString()} bytes saved; ${report.compressedStreams} streams compressed and ${report.removedObjects} unused objects removed.`
          : `The rewritten candidate was ${report.candidateBytes.toLocaleString()} bytes, not smaller. The offered copy uses the original bytes instead.`}</p>
      </div>}
      <p className="muted">Separate-copy outputs never replace existing files or folders. The source, saved state and undo history stay unchanged.</p>
    </div>
  </Dialog>;
}
