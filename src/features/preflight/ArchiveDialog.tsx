import { useEffect, useRef, useState } from "react";
import { Archive, LoaderCircle } from "lucide-react";
import Dialog from "../../components/Dialog";
import { native } from "../../lib/bridge";
import { errorMessage } from "../../lib/geometry";
import { loadPdf } from "../../lib/pdf";
import type { DocumentInfo } from "../../lib/types";
import { archiveBridge } from "./bridge";
import { archivePages, assertTechnicalCurrent } from "./model";
import type { ArchivePlan } from "./model";
import { renderArchivePages } from "./render";

interface Props {
  info: DocumentInfo;
  busy: boolean;
  pending: boolean;
  error: string | null;
  current(): DocumentInfo | null;
  hasPending(): boolean;
  prepare(): Promise<DocumentInfo>;
  run(label: string, task: () => Promise<void>): Promise<void>;
  onError(message: string): void;
  onNotice(message: string): void;
  onClose(): void;
}

export default function ArchiveDialog(props: Props) {
  const [all, setAll] = useState(true);
  const [range, setRange] = useState("");
  const [dpi, setDpi] = useState<150 | 300>(150);
  const [loss, setLoss] = useState(false);
  const [unencrypted, setUnencrypted] = useState(false);
  const [phase, setPhase] = useState<"settings" | "rendering" | "saving" | "cleanup">("settings");
  const [progress, setProgress] = useState({ page: 0, count: 0 });
  const [canceling, setCanceling] = useState(false);
  const job = useRef<ArchivePlan | null>(null);
  const controller = useRef<AbortController | null>(null);
  const latest = useRef(props); latest.current = props;

  useEffect(() => () => {
    controller.current?.abort();
    if (job.current) void archiveBridge.cancel(job.current.archiveId).catch((error: unknown) => {
      latest.current.onError(`Archival cleanup failed: ${errorMessage(error)}`);
    });
  }, []);

  async function release() {
    if (job.current) await archiveBridge.cancel(job.current.archiveId);
    job.current = null;
  }

  function close() {
    void props.run("Releasing archival resources...", async () => {
      await release(); props.onClose();
    });
  }

  function prepare() {
    void props.run("Creating image-only archival copy...", async () => {
      const info = await props.prepare();
      if (job.current) throw new Error("Release the previous archival job before preparing another.");
      const pages = archivePages(all, range, info.pageCount);
      const abort = new AbortController(); controller.current = abort;
      let expired = false;
      const deadline = window.setTimeout(() => {
        expired = true; abort.abort();
        if (job.current) void archiveBridge.cancel(job.current.archiveId).catch((error: unknown) => {
          latest.current.onError(`Expired archival staging cleanup failed: ${errorMessage(error)}`);
        });
      }, 15 * 60 * 1000);
      setPhase("rendering"); setCanceling(false); setProgress({ page: 0, count: pages.length });
      let saved: string | null = null;
      let failure: unknown;
      const check = () => {
        const plan = job.current;
        if (!plan) throw new Error("This archival job was canceled.");
        assertTechnicalCurrent(plan, latest.current.current(), latest.current.hasPending());
      };
      try {
        const plan = await archiveBridge.begin({ documentId: info.id, revision: info.revision, pages, dpi,
          acknowledgeLoss: loss, acknowledgeUnsigned: loss, allowUnencrypted: unencrypted });
        job.current = plan;
        abort.signal.throwIfAborted(); check();
        const bytes = await archiveBridge.bytes(plan.archiveId, abort.signal);
        check();
        const loading = loadPdf(bytes);
        const stopLoading = () => { void loading.destroy().catch((error: unknown) => latest.current.onError(`Archival renderer cleanup failed: ${errorMessage(error)}`)); };
        abort.signal.addEventListener("abort", stopLoading, { once: true });
        try {
          const snapshot = await loading.promise;
          abort.signal.throwIfAborted(); check();
          await renderArchivePages(snapshot, plan, archiveBridge.stage,
            (page, count) => setProgress({ page, count }), abort.signal, check);
        } finally {
          abort.signal.removeEventListener("abort", stopLoading);
          await loading.destroy();
        }
        abort.signal.throwIfAborted(); check();
        await archiveBridge.check(plan.archiveId);
        abort.signal.throwIfAborted(); check();
        setPhase("saving");
        saved = await archiveBridge.save(plan, abort.signal, check);
      } catch (error) { failure = error; }
      try { await release(); }
      catch (error) {
        setPhase("cleanup");
        throw new Error(`${failure ? errorMessage(failure) : saved ? "The copy was exported." : "No copy was saved."} Archival staging cleanup failed: ${errorMessage(error)}`);
      } finally { window.clearTimeout(deadline); controller.current = null; setCanceling(false); }
      setPhase("settings");
      if (expired) throw new Error("The archival job exceeded 15 minutes and was canceled. No source bytes, history or saved paths were changed.");
      if (failure && !abort.signal.aborted) throw failure;
      if (abort.signal.aborted) props.onNotice("Archival preparation canceled. Source bytes, saved state and history are unchanged.");
      else if (!saved) props.onNotice("Archival save canceled. The source is unchanged.");
      else {
        props.onClose();
        props.onNotice(`${native ? "Saved" : "Downloaded"} a fresh, unsigned, image-only PDF/A-2b archival copy at ${dpi} DPI. Review its appearances; the source is unchanged.`);
      }
    });
  }

  const ownerRequired = props.info.passwordProtected && !props.info.permissions.ownerAuthenticated;
  return <Dialog title="Create PDF/A-2b archival copy" className="archive-dialog output-dialog"
    busy={props.busy || phase !== "settings"} error={props.error} onClose={close}
    actions={phase === "rendering" ? <button className="button secondary" disabled={canceling} onClick={() => {
      controller.current?.abort(); setCanceling(true);
    }}>{canceling ? "Canceling..." : "Cancel preparation"}</button>
      : phase === "saving" ? <button className="button secondary" disabled>{native ? "Cancel in the Save dialog" : "Finishing download..."}</button>
        : phase === "cleanup" ? <button className="button secondary" disabled={props.busy} onClick={close}>Retry cleanup &amp; close</button>
          : <><button className="button secondary" disabled={props.busy} onClick={close}>Cancel</button>
            <button className="button primary" disabled={props.busy || props.pending || ownerRequired || !props.info.copyAllowed || !loss
              || props.info.passwordProtected && !unencrypted || !all && !range.trim()} onClick={prepare}><Archive size={15} />Create archival copy</button></>}>
    <div className="output-settings archive-settings">
      <p>Create a <strong>separate, fresh PDF 1.7 image-only PDF/A-2b document</strong> with lossless sRGB page pixels and an embedded ICC output intent. This does not certify or repair the original.</p>
      <div className="archive-losses"><strong>Appearance survives; document structure does not.</strong>
        <p>Text, fonts and vectors become pixels. Search, selection, accessibility tags, interactive forms, attachments, active code,
          original metadata/history and cryptographic signatures are removed. This is not OCR or automatic redaction.</p>
        <p>Annotations and form values are captured only as persistent PDF page appearances, never workspace HTML controls.
          Review the output. Folio opens its archival copies read-only; edit the original and export again.</p>
      </div>
      {phase === "settings" && <>
        <fieldset className="output-scope" disabled={props.busy}><legend>Source pages</legend>
          <div className="output-scope-options">
            <label><input data-autofocus type="radio" name={`archive-pages-${props.info.id}`} checked={all} onChange={() => setAll(true)} />All {props.info.pageCount} pages</label>
            <label><input type="radio" name={`archive-pages-${props.info.id}`} checked={!all} onChange={() => setAll(false)} />Explicit range</label>
          </div>
          {!all && <label className="setting-field">Ascending page numbers<input type="text" maxLength={4096} value={range}
            placeholder={`1-${Math.min(100, props.info.pageCount)}`} onChange={(event) => setRange(event.target.value)} /></label>}
        </fieldset>
        <label className="setting-field">Resolution<select value={dpi} disabled={props.busy} onChange={(event) => {
          const value = Number(event.target.value); if (value === 150 || value === 300) setDpi(value);
        }}><option value={150}>150 DPI - standard raster</option><option value={300}>300 DPI - high-resolution raster</option></select></label>
        <label className="checkbox-setting archive-acknowledgment"><input type="checkbox" disabled={props.busy} checked={loss}
          onChange={(event) => setLoss(event.target.checked)} />I accept rasterization and the listed losses, including a fresh unsigned copy even if the source is signed or certified.</label>
        {props.info.passwordProtected && <div className="output-disclosure">
          <strong>PDF/A forbids encryption.</strong>
          <p>The copy will have no password, encryption, PDF permissions or signatures. Actual owner authorization is required, not just opening-password access or permissive copy rights.</p>
          {ownerRequired && <p className="inline-error">Reopen the source with its owner password before making this unencrypted archive.</p>}
          <label className="checkbox-setting"><input type="checkbox" checked={unencrypted} disabled={props.busy}
            onChange={(event) => setUnencrypted(event.target.checked)} />I explicitly want an unencrypted archival copy.</label>
        </div>}
      </>}
      {!props.info.copyAllowed && <p className="inline-error">This source restricts copying. Reopen with its owner password.</p>}
      {phase === "rendering" && <div className="output-progress" role="status" aria-live="polite">
        <span><LoaderCircle size={15} className="spin" />{canceling ? "Canceling without saving..." : progress.page ? `Rendering selected page ${progress.page} of ${progress.count} at ${dpi} DPI...` : "Loading an isolated PDF snapshot..."}</span>
        <progress value={progress.page} max={progress.count || 1} aria-label="Archival rendering progress" />
      </div>}
      <p className="muted">Limits: 100 selected pages, 24 megapixels per page, 100 megapixels total, 16,384 pixels per dimension,
        64 MiB per PNG, 128 MiB output and 15 minutes. Oversized or unsupported jobs fail explicitly; DPI is never lowered and pages are never skipped.</p>
      <p className="muted">No external validator or Java service runs in the app. The bundled archival generator is independently acceptance-tested
        with local veraPDF, not an arbitrary-PDF conformance checker. Source bytes, unsaved work, saved paths and history are never replaced.</p>
    </div>
  </Dialog>;
}
