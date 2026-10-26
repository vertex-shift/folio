import { useCallback, useEffect, useRef, useState } from "react";
import { CircleHelp, FolderOpen, LoaderCircle, Plus, ShieldCheck, X } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import DocumentWorkspace from "./DocumentWorkspace";
import type { DocumentActivity, DocumentControls } from "./DocumentWorkspace";
import AppearancePicker from "./components/AppearancePicker";
import Dialog from "./components/Dialog";
import DocumentTabs from "./components/DocumentTabs";
import IconButton from "./components/IconButton";
import PasswordDialog from "./components/PasswordDialog";
import { bridge, native, runtimeIssue } from "./lib/bridge";
import { errorMessage } from "./lib/geometry";
import { adjacentTab, unsavedTabs } from "./lib/tabs";
import type { DocumentInfo, OpenOutcome, PasswordChallenge, WorkspaceInfo } from "./lib/types";

export default function App() {
  const [documents, setDocuments] = useState<DocumentInfo[]>([]);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [activities, setActivities] = useState<Record<number, DocumentActivity>>({});
  const [writingTabs, setWritingTabs] = useState<ReadonlySet<number>>(() => new Set());
  const [loading, setLoading] = useState(true);
  const [working, setWorking] = useState(false);
  const [showWorking, setShowWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [passwords, setPasswords] = useState<PasswordChallenge[]>([]);
  const [passwordBusy, setPasswordBusy] = useState(false);
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const [quit, setQuit] = useState(false);
  const [dragActive, setDragActive] = useState(false);
  const controls = useRef(new Map<number, DocumentControls>());
  const fileInput = useRef<HTMLInputElement>(null);
  const documentsRef = useRef(documents);
  const activeRef = useRef(activeId);
  const locked = useRef(false);
  const blockedRef = useRef(false);
  const initial = useRef<Promise<WorkspaceInfo> | null>(null);
  documentsRef.current = documents;
  activeRef.current = activeId;
  const pending = new Set(documents.filter((document) => activities[document.id]?.pendingDraft).map((document) => document.id));
  const busy = loading || working || documents.some((document) => activities[document.id]?.busy)
    || !documents.length && Boolean(activities[0]?.busy);
  const blocked = busy || passwords.length > 0 || quit
    || Boolean(activities[activeId ?? 0]?.modal);
  blockedRef.current = blocked;
  const unsaved = unsavedTabs(documents, pending);
  const events = useRef({
    open: () => {},
    paths: (_paths: string[]) => {},
    menu: (_id: string) => {},
    quit: () => {},
    select: (_id: number) => {},
    close: (_id: number) => {},
    hasUnsaved: false,
  });

  const onInfo = useCallback((info: DocumentInfo) => {
    setDocuments((current) => current.map((item) => item.id === info.id ? info : item));
  }, []);

  const onActivity = useCallback((id: number, activity: DocumentActivity | null) => {
    setActivities((current) => {
      const old = current[id];
      if (activity && old?.busy === activity.busy && old.pendingDraft === activity.pendingDraft && old.modal === activity.modal) return current;
      const next = { ...current };
      if (activity) next[id] = activity; else delete next[id];
      return next;
    });
  }, []);

  const onOpened = useCallback((info: DocumentInfo) => {
    setDocuments((current) => current.some((item) => item.id === info.id)
      ? current.map((item) => item.id === info.id ? info : item)
      : [...current, info]);
    setActiveId(info.id);
  }, []);

  const onCreated = useCallback((info: DocumentInfo) => {
    setWritingTabs((current) => new Set([...current, info.id]));
    onOpened(info);
  }, [onOpened]);

  const reportError = useCallback((message: string) => setError(message), []);

  useEffect(() => {
    if (!working) { setShowWorking(false); return; }
    const timeout = window.setTimeout(() => setShowWorking(true), 160);
    return () => window.clearTimeout(timeout);
  }, [working]);

  function accept(outcomes: OpenOutcome[]) {
    const failures: string[] = [];
    const challenges: PasswordChallenge[] = [];
    for (const outcome of outcomes) {
      if (outcome.status === "opened") onOpened(outcome.document);
      else if (outcome.status === "password_required") challenges.push(outcome);
      else failures.push(`${outcome.name}: ${outcome.message}`);
    }
    if (challenges.length) setPasswords((current) => [...current, ...challenges]);
    if (failures.length) setError(failures.join("\n"));
  }

  useEffect(() => {
    let canceled = false;
    initial.current ??= bridge.bootstrap();
    void initial.current.then((snapshot) => {
      if (canceled) return;
      setDocuments(snapshot.documents);
      setActiveId(snapshot.activeId);
      accept(snapshot.opening);
    }).catch((error: unknown) => {
      if (!canceled) setError(errorMessage(error));
    }).finally(() => { if (!canceled) setLoading(false); });
    return () => { canceled = true; };
  }, []);

  async function run(task: () => Promise<void>) {
    if (locked.current) { setError("Wait for the current document action to finish."); return; }
    locked.current = true;
    setWorking(true);
    setError(null);
    try { await task(); } catch (error) { setError(errorMessage(error)); }
    finally { locked.current = false; setWorking(false); }
  }

  function openFiles(files: File[]) {
    if (blockedRef.current) { setError("Finish the current action or dialog before opening more PDFs."); return; }
    void run(async () => {
      const outcomes: OpenOutcome[] = [];
      for (const file of files) {
        try {
          if (!/\.pdf$/i.test(file.name)) throw new Error("Choose a PDF document.");
          outcomes.push(await bridge.openFile(file));
        } catch (error) {
          outcomes.push({ status: "failed", name: file.name, message: errorMessage(error) });
        }
      }
      accept(outcomes);
    });
  }

  function openPaths(paths: string[]) {
    if (blockedRef.current) { setError("Finish the current action or dialog before opening more PDFs."); return; }
    void run(async () => accept(await bridge.openPaths(paths)));
  }

  function open() {
    if (blockedRef.current) { setError("Finish the current action or dialog before opening more PDFs."); return; }
    if (!native) { fileInput.current?.click(); return; }
    void run(async () => accept(await bridge.open()));
  }

  function select(id: number) {
    if (blockedRef.current || activeRef.current === id) return;
    void run(async () => {
      const document = await bridge.activate(id);
      onInfo(document);
      setActiveId(id);
    });
  }

  async function closeDocument(info: DocumentInfo, discardChanges: boolean) {
    const snapshot = await bridge.closeDocument(info, discardChanges);
    setDocuments(snapshot.documents);
    setActiveId(snapshot.activeId);
    setWritingTabs((current) => {
      const next = new Set(current);
      next.delete(info.id);
      return next;
    });
  }

  function requestClose(id: number) {
    if (blockedRef.current) return;
    void run(async () => {
      const info = documentsRef.current.find((document) => document.id === id);
      if (!info) throw new Error("The selected PDF tab is unavailable.");
      if (activeRef.current !== id && !info.dirty && !activities[id]?.pendingDraft) {
        await closeDocument(info, false);
        return;
      }
      if (activeRef.current !== id) {
        await bridge.activate(id);
        setActiveId(id);
      }
      const control = controls.current.get(id);
      if (!control) throw new Error("The selected PDF tab is unavailable.");
      control.close();
    });
  }

  function requestQuit() {
    if (blockedRef.current) { setError("Finish the current action or dialog before quitting."); return; }
    if (events.current.hasUnsaved) setQuit(true);
    else void run(async () => bridge.close(false));
  }

  function menu(id: string) {
    if (id === "open") { open(); return; }
    if (id === "close-tab") {
      if (activeRef.current !== null) requestClose(activeRef.current);
      return;
    }
    if (blockedRef.current) return;
    const control = controls.current.get(activeRef.current ?? 0);
    if (!control) { setError("The PDF workspace is not ready yet."); return; }
    control.menu(id);
  }

  events.current = { open, paths: openPaths, menu, quit: requestQuit, select, close: requestClose, hasUnsaved: unsaved.length > 0 };

  useEffect(() => {
    const beforeUnload = (event: BeforeUnloadEvent) => {
      if (events.current.hasUnsaved) { event.preventDefault(); event.returnValue = ""; }
    };
    const keydown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "w") {
        event.preventDefault();
        const id = activeRef.current;
        if (id !== null) events.current.close(id);
      } else if (event.ctrlKey && event.key === "Tab") {
        event.preventDefault();
        const id = adjacentTab(documentsRef.current.map((document) => document.id), activeRef.current, event.shiftKey ? -1 : 1);
        if (id !== null) events.current.select(id);
      }
    };
    window.addEventListener("beforeunload", beforeUnload);
    window.addEventListener("keydown", keydown);
    return () => {
      window.removeEventListener("beforeunload", beforeUnload);
      window.removeEventListener("keydown", keydown);
    };
  }, []);

  useEffect(() => {
    if (!native) return;
    let canceled = false;
    let disposers: (() => void)[] = [];
    void Promise.all([
      listen<string>("menu-action", (event) => events.current.menu(event.payload)),
      listen("request-close", () => events.current.quit()),
      listen<string[]>("open-paths", (event) => events.current.paths(event.payload)),
      getCurrentWindow().onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") setDragActive(true);
        else if (payload.type === "leave") setDragActive(false);
        else if (payload.type === "drop") {
          setDragActive(false);
          events.current.paths(payload.paths);
        }
      }),
    ]).then((items) => {
      if (canceled) items.forEach((dispose) => dispose()); else disposers = items;
    }).catch((error: unknown) => setError(`Native window events are unavailable: ${errorMessage(error)}`));
    return () => { canceled = true; disposers.forEach((dispose) => dispose()); };
  }, []);

  async function answerPassword(password: string | null) {
    if (passwordBusy) return;
    const challenge = passwords[0];
    if (!challenge) return;
    setPasswordBusy(true);
    setPasswordError(null);
    try {
      if (password === null) {
        await bridge.cancelOpen(challenge.requestId);
      } else {
        const outcome = await bridge.unlock(challenge.requestId, password);
        if (outcome.status === "password_required") {
          setPasswords((current) => [outcome, ...current.slice(1)]);
          return;
        }
        if (outcome.status === "failed") throw new Error(outcome.message);
        onOpened(outcome.document);
      }
      setPasswords((current) => current.slice(1));
    } catch (error) { setPasswordError(errorMessage(error)); }
    finally { setPasswordBusy(false); }
  }

  const workspaceProps = {
    blocked, onOpen: open, onOpened, onCreated, onInfo,
    onClose: (info: DocumentInfo) => closeDocument(info, true), onActivity,
  };

  return <div className={`app ${native ? "native" : "browser-preview"}`}
    onDragOver={(event) => {
      if (!native && event.dataTransfer.types.includes("Files")) { event.preventDefault(); setDragActive(true); }
    }}
    onDragLeave={(event) => {
      if (!event.currentTarget.contains(event.relatedTarget instanceof Node ? event.relatedTarget : null)) setDragActive(false);
    }}
    onDrop={(event) => {
      if (native || !event.dataTransfer.files.length) return;
      event.preventDefault(); setDragActive(false);
      openFiles(Array.from(event.dataTransfer.files));
    }}>
    <input ref={fileInput} className="file-input" type="file" multiple accept=".pdf,application/pdf" aria-label="Choose PDF files"
      onChange={(event) => {
        const files = Array.from(event.currentTarget.files ?? []);
        event.currentTarget.value = "";
        if (files.length) openFiles(files);
      }} />
    <header className="titlebar" data-tauri-drag-region={native || undefined}>
      <div className="brand" data-tauri-drag-region={native || undefined}>
        <img src="/folio.svg" width="25" height="25" alt="" draggable="false" /><span>folio</span>
      </div>
      <DocumentTabs documents={documents} activeId={activeId} pending={pending} disabled={blocked}
        onSelect={select} onClose={requestClose} />
      <IconButton label="Open PDFs in new tabs" className="new-document" disabled={blocked} onClick={open}><Plus size={18} /></IconButton>
      <div className="titlebar-space" data-tauri-drag-region={native || undefined} />
      <span className="runtime-label"><ShieldCheck size={13} />{runtimeIssue ? "Runtime unavailable" : native ? "Local files" : "Browser preview"}</span>
      <AppearancePicker onError={reportError} />
      <IconButton label="About Folio" disabled={blocked} onClick={() => menu("about")}><CircleHelp size={17} /></IconButton>
    </header>
    {loading ? <div className="workspace-empty"><LoaderCircle className="spin" size={26} /><p>Opening workspace...</p></div>
      : documents.length ? documents.map((document) => <DocumentWorkspace key={document.id} document={document}
        active={activeId === document.id} startWriting={writingTabs.has(document.id)} {...workspaceProps}
        ref={(control) => { if (control) controls.current.set(document.id, control); else controls.current.delete(document.id); }} />)
        : <DocumentWorkspace document={null} active {...workspaceProps}
          ref={(control) => { if (control) controls.current.set(0, control); else controls.current.delete(0); }} />}
    {showWorking && <div className="tab-action-status" role="status"><LoaderCircle size={14} className="spin" />Preparing document tabs...</div>}
    {error && <div className="notification error-notification workspace-error" role="alert">
      <span><strong>That didn&apos;t work.</strong>{error}</span><IconButton label="Dismiss workspace error" onClick={() => setError(null)}><X size={16} /></IconButton>
    </div>}
    {dragActive && <div className="drop-overlay"><div><FolderOpen size={40} /><h2>Open PDFs in new tabs</h2><p>Your existing documents stay open.</p></div></div>}
    {passwords[0] && <PasswordDialog key={passwords[0].requestId} challenge={passwords[0]} busy={passwordBusy}
      error={passwordError} onSubmit={(password) => { void answerPassword(password); }}
      onCancel={() => { void answerPassword(null); }} />}
    {quit && <Dialog title="Keep your open PDFs?" error={error} busy={working} onClose={() => setQuit(false)}
      actions={<><button className="button secondary" disabled={working} onClick={() => setQuit(false)}>Cancel</button>
        <button className="button text-danger" disabled={working} onClick={() => { void run(async () => bridge.close(true)); }}>Discard all &amp; quit</button>
        <button className="button primary" disabled={working} onClick={() => {
          const document = unsaved[0];
          setQuit(false);
          if (document) void run(async () => { await bridge.activate(document.id); setActiveId(document.id); });
        }}>Review changes</button></>}>
      <p>These tabs have unsaved edits, drafts, or pending marks. Apply or discard drafts and save the documents you want to keep before quitting.</p>
      <ul className="unsaved-tab-list">{unsaved.map((document) => <li key={document.id}>
        <strong>{document.name}</strong><span>{pending.has(document.id) ? "Pending document drafts or marks" : "Unsaved PDF edits"}</span>
      </li>)}</ul>
      <p className="muted">Discarding closes all tabs and loses their unsaved work. Files already saved on disk are not deleted.</p>
    </Dialog>}
  </div>;
}
