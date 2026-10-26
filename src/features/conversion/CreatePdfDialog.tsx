import { useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowUp, Check, FilePlus2, FolderOpen, LoaderCircle } from "lucide-react";
import Dialog from "../../components/Dialog";
import { native } from "../../lib/bridge";
import { errorMessage, formatBytes } from "../../lib/geometry";
import { PAGE_SIZES } from "../../lib/pages";
import type { DocumentInfo } from "../../lib/types";
import { conversionBridge } from "./bridge";
import { creationSettings, moveCreationInput, validateCreationFiles, waitForCreation } from "./model";
import type { CreationKind, CreationPlan, ReadyCreationStatus } from "./model";
import "./styles.css";

type Phase = "empty" | "selecting" | "settings" | "preparing" | "ready" | "opening" | "cleanup";

interface Props {
  onCreated(info: DocumentInfo): void;
  onClose(): void;
  onActivity(busy: boolean, pending: boolean): void;
  onResourceError(message: string): void;
}

export default function CreatePdfDialog(props: Props) {
  const [kind, setKind] = useState<CreationKind>("images");
  const [phase, setPhase] = useState<Phase>("empty");
  const [plan, setPlan] = useState<CreationPlan | null>(null);
  const [order, setOrder] = useState<number[]>([]);
  const [size, setSize] = useState<keyof typeof PAGE_SIZES>("letter");
  const [landscape, setLandscape] = useState(false);
  const [margin, setMargin] = useState("36");
  const [fontSize, setFontSize] = useState("12");
  const [ready, setReady] = useState<ReadyCreationStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [canceling, setCanceling] = useState(false);
  const [uploaded, setUploaded] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const primary = useRef<HTMLButtonElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const chooser = useRef<HTMLButtonElement>(null);
  const feedback = useRef<HTMLDivElement>(null);
  const job = useRef<CreationPlan | null>(null);
  const controller = useRef<AbortController | null>(null);
  const locked = useRef(false);
  const latest = useRef(props);
  latest.current = props;
  const busy = ["selecting", "preparing", "opening"].includes(phase);

  useEffect(() => {
    props.onActivity(busy, Boolean(plan));
  }, [busy, plan, props.onActivity]);

  useEffect(() => {
    if (phase === "settings" || phase === "ready") primary.current?.focus();
    else if (phase === "preparing" || phase === "selecting" && !native) cancel.current?.focus();
    else if (phase === "empty") chooser.current?.focus();
  }, [phase]);

  useEffect(() => {
    const body = feedback.current?.closest<HTMLElement>(".dialog-body");
    if (body && (busy || ready || error || notice)) body.scrollTop = 0;
  }, [phase, error, notice]);

  useEffect(() => () => {
    controller.current?.abort();
    const current = job.current;
    if (current) void conversionBridge.cancel(current.jobId).catch((error: unknown) => {
      latest.current.onResourceError(`New-PDF cleanup failed: ${errorMessage(error)}`);
    });
    latest.current.onActivity(false, false);
  }, []);

  async function release() {
    const current = job.current;
    if (current) await conversionBridge.cancel(current.jobId);
    job.current = null;
    setPlan(null);
    setOrder([]);
    setReady(null);
  }

  async function cleanup(failure: unknown, aborted: boolean) {
    try {
      await release();
      setPhase("empty");
      if (aborted) setNotice("Creation canceled. No PDF tab was opened; existing documents and drafts are unchanged.");
      else setError(errorMessage(failure));
    } catch (error) {
      setPhase("cleanup");
      setError(`${aborted ? "Creation canceled." : errorMessage(failure)} Cleanup failed: ${errorMessage(error)}`);
    }
  }

  async function choose(files?: File[]) {
    if (locked.current) return;
    locked.current = true;
    setError(null); setNotice(null); setPhase("selecting"); setUploaded(0);
    const abort = new AbortController();
    controller.current = abort;
    try {
      if (files) validateCreationFiles(kind, files);
      let current: CreationPlan | null = await conversionBridge.begin(kind);
      job.current = current;
      abort.signal.throwIfAborted();
      if (native) {
        current = await conversionBridge.pick(current.jobId);
        if (!current) {
          job.current = null;
          setPhase("empty");
          setNotice("File selection canceled. No document was opened.");
          return;
        }
      } else {
        for (const [index, file] of (files ?? []).entries()) {
          current = await conversionBridge.stage(current.jobId, file, abort.signal);
          setUploaded(index + 1);
        }
      }
      abort.signal.throwIfAborted();
      job.current = current;
      setPlan(current);
      setOrder(current.inputs.map((input) => input.index));
      setPhase("settings");
    } catch (error) {
      await cleanup(error, abort.signal.aborted);
    } finally {
      locked.current = false; controller.current = null; setCanceling(false);
    }
  }

  function prepare() {
    if (locked.current || !job.current) return;
    let settings;
    try { settings = creationSettings(size, landscape, margin, fontSize); }
    catch (error) { setError(errorMessage(error)); return; }
    const current = job.current;
    locked.current = true;
    const abort = new AbortController();
    controller.current = abort;
    setPhase("preparing"); setError(null); setNotice(null);
    void (async () => {
      try {
        await conversionBridge.prepare(current.jobId, settings, order, abort.signal);
        const result = await waitForCreation(current.jobId, (id) => conversionBridge.status(id, abort.signal), abort.signal);
        setReady(result);
        setPhase("ready");
      } catch (error) {
        await cleanup(error, abort.signal.aborted);
      } finally {
        locked.current = false; controller.current = null; setCanceling(false);
      }
    })();
  }

  function open() {
    if (locked.current || !job.current) return;
    const current = job.current;
    locked.current = true;
    setPhase("opening"); setError(null);
    void conversionBridge.commit(current.jobId).then((info) => {
      job.current = null;
      props.onCreated(info);
      props.onClose();
    }).catch(async (error: unknown) => {
      await cleanup(error, false);
    }).finally(() => { locked.current = false; });
  }

  function clear(close: boolean) {
    if (locked.current) return;
    locked.current = true;
    void release().then(() => {
      setPhase("empty"); setError(null); setNotice(null);
      if (close) props.onClose();
    }).catch((error: unknown) => {
      setPhase("cleanup"); setError(`New-PDF cleanup failed: ${errorMessage(error)}`);
    }).finally(() => { locked.current = false; });
  }

  const freeze = busy || phase === "ready" || phase === "cleanup";
  return <Dialog title="Create PDF from files" className="conversion-dialog" busy={busy || phase === "cleanup"}
    onClose={() => clear(true)}
    actions={phase === "preparing" || phase === "selecting" && !native
      ? <button ref={cancel} className="button secondary" disabled={canceling} onClick={() => { controller.current?.abort(); setCanceling(true); }}>
        {canceling ? "Canceling..." : "Cancel preparation"}</button>
      : phase === "selecting" || phase === "opening"
        ? <button className="button secondary" disabled>{phase === "opening" ? "Opening new tab..." : "Cancel in the file picker"}</button>
        : phase === "cleanup"
          ? <button className="button secondary" onClick={() => clear(true)}>Retry cleanup &amp; close</button>
          : <><button className="button secondary" onClick={() => clear(true)}>Cancel</button>
            {phase === "ready" ? <button ref={primary} className="button primary" onClick={open}><FilePlus2 size={15} />Open new PDF</button>
              : <button ref={primary} className="button primary" disabled={!plan?.inputs.length} onClick={prepare}><FilePlus2 size={15} />Prepare PDF</button>}</>}>
    <p>Create a <strong>new, independent PDF</strong> from local files. Existing tabs, files, unsaved edits and document drafts stay unchanged.</p>
    <div ref={feedback} className="creation-feedback">
      {error && <p className="inline-error" role="alert">{error}</p>}
      {busy && <div className="output-progress" role="status" aria-live="polite"><span><LoaderCircle size={15} className="spin" />
        {canceling ? "Canceling without opening a document..."
          : phase === "selecting" ? native ? "Choose local files in the file picker..." : `Staging local files (${uploaded} received)...`
            : phase === "opening" ? "Opening the completed PDF in a new tab..." : "Building the complete PDF locally..."}</span></div>}
      {ready && phase === "ready" && <div className="creation-ready" role="status"><Check size={18} />
        <div><strong>New PDF ready</strong><p>{ready.pageCount} pages, {formatBytes(ready.byteLength)}.
          {" "}{PAGE_SIZES[size].label}, {landscape ? "landscape" : "portrait"}, {margin} pt margins{kind === "text" ? `, ${fontSize} pt text` : ""}.
          {" "}Open it in a new tab, then save a copy to keep it.</p></div></div>}
      {notice && <p role="status">{notice}</p>}
    </div>
    <input ref={input} className="file-input" type="file" multiple accept={kind === "images" ? ".png,.jpg,.jpeg,image/png,image/jpeg" : ".txt,.text,text/plain"}
      aria-label="Choose creation files" onChange={(event) => {
        const files = Array.from(event.currentTarget.files ?? []);
        event.currentTarget.value = "";
        if (files.length) void choose(files);
      }} />
    <fieldset className="output-scope" disabled={freeze || Boolean(plan)}><legend>Source files</legend>
      <div className="output-scope-options">
        <label><input data-autofocus type="radio" name="creation-kind" checked={kind === "images"} onChange={() => setKind("images")} />PNG / JPEG images</label>
        <label><input type="radio" name="creation-kind" checked={kind === "text"} onChange={() => setKind("text")} />UTF-8 text</label>
      </div>
    </fieldset>
    {!plan && !busy && phase !== "cleanup" && <div className="creation-empty">
      <FolderOpen size={24} aria-hidden="true" />
      <p>{kind === "images" ? "Choose still images, then arrange their page order." : "Choose UTF-8 text files. Each file starts on a new page."}</p>
      <button ref={chooser} className="button secondary" onClick={() => native ? void choose() : input.current?.click()}><FolderOpen size={15} />Choose files</button>
    </div>}
    {plan && <section className="creation-files" aria-label="Chosen file order">
      <div className="creation-files-heading"><h3>File order <span>{order.length} / 50</span></h3>
        <button className="text-button" disabled={freeze} onClick={() => clear(false)}>Clear files</button></div>
      <ol>{order.map((index, position) => {
        const file = plan.inputs[index];
        return <li key={index}><span className="creation-file-position">{position + 1}</span>
          <span className="creation-file-name"><strong>{file.name}</strong><span>{formatBytes(file.byteLength)}</span></span>
          <button className="icon-button" aria-label={`Move ${file.name} up`} disabled={freeze || position === 0}
            onClick={() => setOrder(moveCreationInput(order, position, -1))}><ArrowUp size={16} /></button>
          <button className="icon-button" aria-label={`Move ${file.name} down`} disabled={freeze || position === order.length - 1}
            onClick={() => setOrder(moveCreationInput(order, position, 1))}><ArrowDown size={16} /></button>
        </li>;
      })}</ol>
    </section>}
    {phase !== "empty" && phase !== "selecting" && <fieldset className="creation-layout" disabled={freeze}><legend>Page layout</legend>
      <label className="setting-field">Page size<select aria-label="Page size" value={size} onChange={(event) => {
        const value = event.target.value; if (value === "letter" || value === "a4") setSize(value);
      }}>{Object.entries(PAGE_SIZES).map(([key, page]) => <option key={key} value={key}>{page.label}</option>)}</select></label>
      <label className="setting-field">Orientation<select aria-label="Orientation" value={landscape ? "landscape" : "portrait"} onChange={(event) => setLandscape(event.target.value === "landscape")}>
        <option value="portrait">Portrait</option><option value="landscape">Landscape</option></select></label>
      <label className="setting-field">Margin (points)<input type="number" min="0" max="144" step="1" value={margin} onChange={(event) => setMargin(event.target.value)} /></label>
      {kind === "text" && <label className="setting-field">Text size (points)<input type="number" min="8" max="36" step="1" value={fontSize} onChange={(event) => setFontSize(event.target.value)} /></label>}
    </fieldset>}
    <p className="muted">{kind === "images"
      ? "One centered image per page, fitted inside the margins without distortion. EXIF orientation and PNG transparency are retained; image metadata is not copied. Up to 16 MiB, 24 megapixels and 8,192 pixels per image; 64 MiB input and 128 MiB decoded pixels per batch."
      : "Readable, wrapped and paginated Noto Sans text; Latin, Greek and Cyrillic only. Unsupported glyphs and invalid UTF-8 are refused. A leading UTF-8 BOM is removed; CRLF/CR become newlines and tabs become four spaces. No truncation, OCR or rich-text import. Up to 1 MiB per file and 4 MiB total."}</p>
    <p className="muted">Up to 50 files, 500 output pages and 128 MiB per new PDF. Settings use PDF points (72 points = 1 inch); margins are 0-144 points. Nothing is opened until the complete batch succeeds.</p>
  </Dialog>;
}
