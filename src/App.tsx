import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft, ArrowRight, BookOpen, Check, ChevronLeft, ChevronRight,
  CircleHelp, Copy, FileText, FolderOpen, GripVertical, Hand, Highlighter,
  Keyboard, LayoutGrid, LoaderCircle, LockKeyhole, MessageSquare,
  MessageSquarePlus, Minus, MousePointer2, PanelLeft, PanelLeftClose,
  PanelRightClose, Plus, Redo2, RotateCcw, RotateCw, ScanLine, Search,
  ShieldCheck, Trash2, Undo2, X,
  Bookmark, FilePlus2, FileInput, FileOutput, Pencil, Type, SquarePen, ExternalLink,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { PDFDocumentProxy } from "pdfjs-dist";
import Dialog from "./components/Dialog";
import IconButton from "./components/IconButton";
import AppearancePicker from "./components/AppearancePicker";
import PdfPage, { Thumbnail } from "./components/PdfPage";
import BookmarksPanel from "./components/BookmarksPanel";
import { bridge, native, runtimeIssue } from "./lib/bridge";
import { errorMessage, formatBytes, pageFromInput } from "./lib/geometry";
import { loadPdf } from "./lib/pdf";
import { useSearch } from "./lib/useSearch";
import { PAGE_SIZES, parsePageRange, parseWritingNumber, rgbFromHex } from "./lib/pages";
import { destinationPage, safeExternalUrl } from "./lib/navigation";
import type { DocumentInfo, Edit, Mode, Panel, Point, Quad, Tool, Zoom } from "./lib/types";

type Action = { type: "open" | "example" | "close" }
  | { type: "file"; file: File } | { type: "path"; path: string }
  | { type: "create"; width: number; height: number };
type Modal = { type: "shortcuts" | "about" }
  | { type: "unsaved"; action: Action }
  | { type: "delete"; page: number }
  | { type: "note"; page: number; point: Point; text: string }
  | { type: "text"; page: number; rect: [number, number, number, number]; text: string }
  | { type: "edit-annotation"; id: string; text: string }
  | { type: "create" | "blank"; size: keyof typeof PAGE_SIZES; landscape: boolean }
  | { type: "extract"; range: string }
  | { type: "external"; url: string };

const quickTools = [
  { tool: "select", icon: MousePointer2, label: "Select text (V)" },
  { tool: "hand", icon: Hand, label: "Pan document (H)" },
  { tool: "highlight", icon: Highlighter, label: "Highlight text (M)" },
  { tool: "note", icon: MessageSquarePlus, label: "Add a note (N)" },
  { tool: "text", icon: Type, label: "Add text (T)" },
  { tool: "ink", icon: Pencil, label: "Draw with pen (D)" },
] as const;
const panelLabels: Record<Panel, string> = {
  tools: "Document tools", search: "Find in document", comments: "Comments",
  bookmarks: "Bookmarks", write: "Writing tools",
};

function typingTarget(target: EventTarget | null): boolean {
  return target instanceof Element && Boolean(target.closest("input, textarea, select, [contenteditable='true']"));
}

export default function App() {
  const [info, setInfo] = useState<DocumentInfo | null>(null);
  const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null);
  const [page, setPage] = useState(0);
  const [pageInput, setPageInput] = useState("1");
  const [mode, setMode] = useState<Mode>("read");
  const [panel, setPanel] = useState<Panel>("tools");
  const [panelOpen, setPanelOpen] = useState(window.innerWidth >= 1100);
  const [railOpen, setRailOpen] = useState(window.innerWidth >= 700);
  const [tool, setTool] = useState<Tool>("select");
  const [zoom, setZoom] = useState<Zoom>({ mode: "page" });
  const [actualScale, setActualScale] = useState(1);
  const [query, setQuery] = useState("");
  const [matchCase, setMatchCase] = useState(false);
  const [activeResult, setActiveResult] = useState(0);
  const [selectedAnnotation, setSelectedAnnotation] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [modal, setModal] = useState<Modal | null>(null);
  const pendingDraft = Boolean(modal && "text" in modal && modal.text.trim());
  const pendingDraftRef = useRef(pendingDraft);
  pendingDraftRef.current = pendingDraft;
  const [dragActive, setDragActive] = useState(false);
  const [draggedPage, setDraggedPage] = useState<number | null>(null);
  const [overPage, setOverPage] = useState<number | null>(null);
  const [viewportSize, setViewportSize] = useState({ width: 800, height: 740 });
  const [textSize, setTextSize] = useState("14");
  const [penWidth, setPenWidth] = useState("2");
  const [writingColor, setWritingColor] = useState("#253b57");
  const viewerRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const importRef = useRef<HTMLInputElement>(null);
  const infoRef = useRef(info);
  const pageRef = useRef(page);
  const busyRef = useRef(false);
  const panRef = useRef<{ x: number; y: number; left: number; top: number } | null>(null);
  const actionRef = useRef({
    open: () => {}, save: (_copy: boolean) => {}, history: (_redo: boolean) => {},
    find: () => {}, action: (_action: Action) => {}, page: (_delta: number) => {},
    zoom: (_factor: number) => {}, tool: (_tool: Tool) => {},
    menu: (_id: string) => {}, escape: () => {},
    hasModal: false,
  });
  infoRef.current = info;
  pageRef.current = page;
  const search = useSearch(pdf, info, query, matchCase, panelOpen && panel === "search");
  const readOnly = Boolean(info?.readOnlyReason);
  const disabled = Boolean(busy) || !info;
  const editDisabled = disabled || readOnly;
  const saveLabel = native ? "Save a copy" : "Download PDF";

  const notifyError = useCallback((message: string) => setError(message), []);
  const onScale = useCallback((scale: number) => setActualScale(scale), []);

  function updateInfo(next: DocumentInfo) {
    infoRef.current = next;
    setInfo(next);
  }

  async function adopt(next: DocumentInfo) {
    const previous = infoRef.current;
    const previousKey = previous?.pages[pageRef.current]?.key;
    const sameDocument = previous?.id === next.id;
    const matchingPage = sameDocument ? next.pages.findIndex((item) => item.key === previousKey) : 0;
    updateInfo(next);
    setPage(matchingPage >= 0 ? matchingPage : Math.min(pageRef.current, next.pageCount - 1));
    setSelectedAnnotation(null);
    if (!sameDocument) {
      setPage(0);
      setMode("read");
      setTool("select");
      setQuery("");
      setPanel("tools");
      setZoom({ mode: "page" });
    }
    setPdf(null);
    const bytes = await bridge.bytes(next);
    const task = loadPdf(bytes);
    try {
      const document = await task.promise;
      if (document.numPages !== next.pageCount) {
        await document.loadingTask.destroy();
        throw new Error("The renderer and document engine disagree on the page count. Reopen this PDF before editing it.");
      }
      setPdf(document);
    } catch (error) {
      await task.destroy();
      throw new Error(`The PDF could not be displayed. ${errorMessage(error)}`);
    }
  }

  async function run(label: string, task: () => Promise<void>) {
    if (busyRef.current) {
      setToast("Wait for the current document action to finish.");
      return;
    }
    busyRef.current = true;
    setBusy(label);
    setError(null);
    try {
      await task();
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      busyRef.current = false;
      setBusy(null);
    }
  }

  async function perform(action: Action) {
    switch (action.type) {
      case "open": {
        const next = await bridge.open(true);
        if (next) await adopt(next);
        break;
      }
      case "example": await adopt(await bridge.example(true)); break;
      case "file": await adopt(await bridge.openFile(action.file, true)); break;
      case "path": await adopt(await bridge.openPath(action.path, true)); break;
      case "close": await bridge.close(); break;
      case "create":
        await adopt(await bridge.create(action.width, action.height, true));
        setMode("write"); setPanel("write"); setPanelOpen(true);
        break;
    }
  }

  function requestAction(action: Action) {
    if (busyRef.current) {
      setToast("Wait for the current document action to finish.");
      return;
    }
    if (pendingDraftRef.current) {
      setError("Finish or cancel the current text or note before leaving this document.");
      return;
    }
    if (infoRef.current?.dirty) {
      setModal({ type: "unsaved", action });
    } else {
      void run(action.type === "close" ? "Closing..." : "Opening PDF...", () => perform(action));
    }
  }

  function open() {
    if (native) requestAction({ type: "open" });
    else fileRef.current?.click();
  }

  function changePage(delta: number) {
    const document = infoRef.current;
    if (!document || busyRef.current) return;
    setPage((page) => Math.max(0, Math.min(document.pageCount - 1, page + delta)));
  }

  function changeZoom(factor: number) {
    setZoom({ mode: "custom", scale: Math.max(0.2 * 4 / 3, Math.min(4 * 4 / 3, actualScale * factor)) });
  }

  function showPanel(next: Panel) {
    setPanel(next);
    setPanelOpen(true);
    if (mode === "organize") setMode("read");
    if (next === "comments") setMode("comment");
  }

  function chooseTool(next: Tool) {
    if (!["select", "hand"].includes(next) && readOnly) {
      setError(info?.readOnlyReason ?? "This PDF is read-only.");
      return;
    }
    setTool(next);
    if (mode === "organize") setMode("read");
    if (next === "highlight" || next === "note") setMode("comment");
    if (next === "text" || next === "ink") { setMode("write"); setPanel("write"); setPanelOpen(true); }
  }

  function changeMode(next: Mode) {
    setMode(next);
    setTool("select");
    if (next === "comment") { setPanel("comments"); setPanelOpen(true); }
    if (next === "write") { setPanel("write"); setPanelOpen(true); }
  }

  function edit(change: Edit, label: string, onSuccess?: () => void) {
    void run(label, async () => {
      const current = infoRef.current;
      if (!current) throw new Error("Open a PDF before editing.");
      await adopt(await bridge.edit(current, change));
      onSuccess?.();
      if (change.type === "highlight") {
        setToast("Highlight added to the PDF.");
      } else if (change.type === "note") {
        setToast("Note added to the PDF.");
        setPanel("comments"); setPanelOpen(true);
      } else if (change.type === "delete") {
        setToast("Page removed. Undo is available.");
      } else if (change.type === "text" || change.type === "ink" || change.type === "set_annotation_text") {
        setToast("Writing saved in the working PDF. Save a copy to keep it.");
      }
    });
  }

  function importPages(file?: File) {
    if (!native && !file) { importRef.current?.click(); return; }
    void run("Inserting PDF pages...", async () => {
      const current = infoRef.current;
      if (!current) throw new Error("Open or create a PDF before inserting pages.");
      const at = pageRef.current + 1;
      const next = file ? await bridge.importFile(current, file, at) : await bridge.import(current, at);
      if (!next) return;
      await adopt(next); setPage(at); setMode("organize");
      setToast("PDF pages inserted. Undo is available.");
    });
  }

  function navigateLink(destination: unknown, url: string | null) {
    if (url) {
      try { setModal({ type: "external", url: safeExternalUrl(url) }); }
      catch (error) { setError(errorMessage(error)); }
      return;
    }
    void run("Following document link...", async () => {
      if (!pdf) throw new Error("Wait for the PDF to finish loading.");
      const target = await destinationPage(pdf, destination);
      setPage(target); setSelectedAnnotation(null); setMode("read");
    });
  }

  function draw(points: Point[]) {
    try {
      edit({ type: "ink", page, points, color: rgbFromHex(writingColor),
        width: parseWritingNumber(penWidth, 0.5, 12, "Pen width") }, "Saving drawing...");
    } catch (error) { setError(errorMessage(error)); }
  }

  function highlight(quads: Quad[], text: string) {
    edit({ type: "highlight", page, quads, color: [1, 0.86, 0.17], text }, "Adding highlight...");
  }

  function history(redo: boolean) {
    if (typingTarget(document.activeElement)) {
      document.execCommand(redo ? "redo" : "undo");
      return;
    }
    const current = infoRef.current;
    if (!current || !(redo ? current.canRedo : current.canUndo)) return;
    void run(redo ? "Redoing change..." : "Undoing change...", async () => {
      await adopt(await (redo ? bridge.redo(current) : bridge.undo(current)));
    });
  }

  function save(copy = true) {
    void run(native ? "Saving PDF..." : "Preparing download...", async () => {
      const current = infoRef.current;
      if (!current) throw new Error("Open a PDF before saving.");
      const saved = await bridge.save(current, copy);
      if (!saved) return;
      updateInfo(saved);
      setToast(native ? "PDF saved." : "PDF downloaded. Browser preview does not overwrite local files.");
    });
  }

  function chooseResult(index: number) {
    if (!search.results.length) return;
    const next = (index + search.results.length) % search.results.length;
    setActiveResult(next);
    setPage(search.results[next].page);
  }

  function selectAnnotation(id: string) {
    const annotation = info?.annotations.find((annotation) => annotation.id === id);
    if (!annotation) return;
    setPage(annotation.page);
    setSelectedAnnotation(id);
    setPanel("comments");
    setPanelOpen(true);
    setMode("comment");
  }

  function handleMenu(id: string) {
    if ((id === "undo" || id === "redo") && typingTarget(document.activeElement)) {
      history(id === "redo");
      return;
    }
    if (modal) return;
    switch (id) {
      case "new": setModal({ type: "create", size: "letter", landscape: false }); break;
      case "open": open(); break;
      case "example": requestAction({ type: "example" }); break;
      case "save": save(false); break;
      case "save-copy": save(true); break;
      case "undo": history(false); break;
      case "redo": history(true); break;
      case "find": showPanel("search"); break;
    }
  }

  actionRef.current = {
    open, save, history, find: () => showPanel("search"),
    action: requestAction, page: changePage, zoom: changeZoom, tool: chooseTool,
    menu: handleMenu, hasModal: Boolean(modal),
    escape: () => { if (!busyRef.current) { setModal(null); setTool("select"); } },
  };

  useEffect(() => {
    void run("Opening workspace...", async () => adopt(await bridge.bootstrap()));
  }, []);

  useEffect(() => () => {
    if (pdf) void pdf.loadingTask.destroy().catch((error: unknown) => console.error("Could not release PDF renderer resources:", error));
  }, [pdf]);

  useEffect(() => {
    const viewport = viewerRef.current;
    if (!viewport) return;
    const observer = new ResizeObserver(([entry]) => {
      setViewportSize({ width: entry.contentRect.width, height: entry.contentRect.height });
    });
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [mode]);

  useEffect(() => setPageInput(String(page + 1)), [page]);

  useEffect(() => {
    if (!native || !info) return;
    let active = true;
    void bridge.draft(info.id, pendingDraft).catch((error: unknown) => {
      if (active) setError(`Native draft protection could not be updated: ${errorMessage(error)}`);
    });
    return () => { active = false; };
  }, [info?.id, pendingDraft]);

  useEffect(() => {
    if (panelOpen && panel === "search") searchRef.current?.focus();
  }, [panel, panelOpen]);

  useEffect(() => {
    if (!search.results.length) return;
    setActiveResult(0);
    setPage(search.results[0].page);
  }, [search.results]);

  useEffect(() => {
    const selected = document.querySelector<HTMLElement>(".page-rail .page-thumb.selected");
    selected?.scrollIntoView({ block: "nearest" });
  }, [page]);

  useEffect(() => {
    if (!toast) return;
    const timeout = window.setTimeout(() => setToast(null), 5000);
    return () => window.clearTimeout(timeout);
  }, [toast]);

  useEffect(() => {
    const beforeUnload = (event: BeforeUnloadEvent) => {
      if (infoRef.current?.dirty || pendingDraftRef.current) { event.preventDefault(); event.returnValue = ""; }
    };
    const keydown = (event: KeyboardEvent) => {
      const actions = actionRef.current;
      const key = event.key.toLowerCase();
      const mod = event.metaKey || event.ctrlKey;
      if (actions.hasModal) return;
      if (mod && ["o", "n", "s", "f", "z", "y", "+", "=", "-"].includes(key)) {
        if (typingTarget(event.target) && (key === "z" || key === "y")) return;
        event.preventDefault();
        if (key === "o") actions.open();
        else if (key === "n") actions.menu("new");
        else if (key === "s") actions.save(event.shiftKey);
        else if (key === "f") actions.find();
        else if (key === "z" || key === "y") actions.history(event.shiftKey || key === "y");
        else actions.zoom(key === "-" ? 1 / 1.2 : 1.2);
      } else if (!mod && !typingTarget(event.target)) {
        if (["ArrowRight", "PageDown"].includes(event.key)) { event.preventDefault(); actions.page(1); }
        if (["ArrowLeft", "PageUp"].includes(event.key)) { event.preventDefault(); actions.page(-1); }
        if (key === "v") actions.tool("select");
        if (key === "h") actions.tool("hand");
        if (key === "m") actions.tool("highlight");
        if (key === "n") actions.tool("note");
        if (key === "t") actions.tool("text");
        if (key === "d") actions.tool("ink");
        if (key === "escape") actions.escape();
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
    let cancelled = false;
    let disposers: (() => void)[] = [];
    void Promise.all([
      listen<string>("menu-action", (event) => actionRef.current.menu(event.payload)),
      listen("request-close", () => actionRef.current.action({ type: "close" })),
      listen<string>("open-path", (event) => actionRef.current.action({ type: "path", path: event.payload })),
      getCurrentWindow().onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") setDragActive(true);
        else if (payload.type === "leave") setDragActive(false);
        else if (payload.type === "drop") {
          setDragActive(false);
          if (payload.paths.length !== 1 || !/\.pdf$/i.test(payload.paths[0])) {
            setError("Drop one PDF document at a time.");
          } else {
            actionRef.current.action({ type: "path", path: payload.paths[0] });
          }
        }
      }),
    ]).then((items) => {
      if (cancelled) items.forEach((dispose) => dispose());
      else disposers = items;
    }).catch((error: unknown) => setError(`Native window events are unavailable: ${errorMessage(error)}`));
    return () => { cancelled = true; disposers.forEach((dispose) => dispose()); };
  }, []);

  useEffect(() => {
    const compact = window.matchMedia("(max-width: 1099px)");
    const narrow = window.matchMedia("(max-width: 699px)");
    const collapseTools = () => { if (compact.matches) setPanelOpen(false); };
    const collapsePages = () => { if (narrow.matches) setRailOpen(false); };
    compact.addEventListener("change", collapseTools);
    narrow.addEventListener("change", collapsePages);
    return () => {
      compact.removeEventListener("change", collapseTools);
      narrow.removeEventListener("change", collapsePages);
    };
  }, []);

  const currentMatch = search.results[activeResult];
  const zoomPercent = Math.round(actualScale * 75);
  const customPercent = zoom.mode === "custom" ? String(Math.round(zoom.scale * 75)) : zoom.mode;
  const zoomOptions = ["50", "75", "100", "125", "150", "200", "300", "400"];

  return <div className={`app ${native ? "native" : "browser-preview"}`}
    onDragOver={(event) => {
      if (!native && event.dataTransfer.types.includes("Files")) { event.preventDefault(); setDragActive(true); }
    }}
    onDragLeave={(event) => {
      if (!event.currentTarget.contains(event.relatedTarget instanceof Node ? event.relatedTarget : null)) setDragActive(false);
    }}
    onDrop={(event) => {
      if (native || !event.dataTransfer.files.length) return;
      event.preventDefault();
      setDragActive(false);
      const files = Array.from(event.dataTransfer.files);
      if (files.length !== 1 || !/\.pdf$/i.test(files[0].name)) setError("Drop one PDF document at a time.");
      else requestAction({ type: "file", file: files[0] });
    }}
  >
    <input ref={fileRef} className="file-input" type="file" accept=".pdf,application/pdf" aria-label="Choose PDF file"
      onChange={(event) => {
        const file = event.currentTarget.files?.[0];
        if (file) requestAction({ type: "file", file });
        event.currentTarget.value = "";
      }} />
    <input ref={importRef} className="file-input" type="file" accept=".pdf,application/pdf" aria-label="Choose PDF to insert"
      onChange={(event) => { const file = event.currentTarget.files?.[0]; if (file) importPages(file); event.currentTarget.value = ""; }} />

    <header className="titlebar" data-tauri-drag-region={native || undefined}>
      <div className="brand" data-tauri-drag-region={native || undefined}>
        <img src="/folio.svg" width="25" height="25" alt="" draggable="false" /><span>folio</span>
      </div>
      <div className="document-tab" title={info?.name}>
        <FileText size={16} className="tab-file-icon" />
        <span>{info?.name ?? "PDF workspace"}</span>
        {(info?.dirty || pendingDraft) && <span className="unsaved-dot" aria-label="Unsaved changes" />}
      </div>
      <IconButton label="Open another PDF" className="new-document" disabled={Boolean(busy)} onClick={open}><Plus size={18} /></IconButton>
      <div className="titlebar-space" data-tauri-drag-region={native || undefined} />
      <span className="runtime-label"><ShieldCheck size={13} />{runtimeIssue ? "Runtime unavailable" : native ? "On this Mac" : "Browser preview"}</span>
      <AppearancePicker onError={notifyError} />
      <IconButton label="About Folio" onClick={() => setModal({ type: "about" })}><CircleHelp size={17} /></IconButton>
    </header>

    <div className="commandbar">
      <button className={`all-tools ${panelOpen && panel === "tools" ? "selected" : ""}`} onClick={() => {
        if (panel === "tools") setPanelOpen((open) => !open);
        else { setPanel("tools"); setPanelOpen(true); }
      }} aria-expanded={panelOpen && panel === "tools"} aria-label="All tools" title="All tools">
        <PanelLeft size={17} /><span>All tools</span>
      </button>
      <nav className="mode-tabs" aria-label="Document workspace">
        {([{ mode: "read", label: "Read", icon: BookOpen }, { mode: "write", label: "Write", icon: SquarePen }, { mode: "comment", label: "Comment", icon: MessageSquare },
          { mode: "organize", label: "Organize", icon: LayoutGrid }] as const).map((item) =>
          <button key={item.mode} className={mode === item.mode ? "selected" : ""}
            aria-label={item.label} title={item.label}
            aria-current={mode === item.mode ? "page" : undefined}
            disabled={disabled} onClick={() => changeMode(item.mode)}>
            <item.icon size={16} /><span>{item.label}</span>
          </button>)}
      </nav>
      <div className="command-actions">
        <IconButton label="Find in document (Command+F)" disabled={disabled} onClick={() => showPanel("search")}><Search size={18} /></IconButton>
        <span className="toolbar-divider" />
        <IconButton label="Undo (Command+Z)" disabled={disabled || !info?.canUndo} onClick={() => history(false)}><Undo2 size={18} /></IconButton>
        <IconButton label="Redo (Command+Shift+Z)" disabled={disabled || !info?.canRedo} onClick={() => history(true)}><Redo2 size={18} /></IconButton>
        <button className="button primary save-button" aria-label={saveLabel} title={saveLabel} disabled={disabled} onClick={() => save(true)}>
          <Copy size={15} /><span>{saveLabel}</span>
        </button>
      </div>
    </div>

    {info?.readOnlyReason && <div className="readonly-banner" role="status"><LockKeyhole size={15} />{info.readOnlyReason}</div>}

    <div className="workspace">
      {panelOpen && <aside className={`tools-panel panel-${panel}`} aria-label={panel === "tools" ? "PDF tools" : panelLabels[panel]}>
        <div className="panel-heading">
          <h2>{panelLabels[panel]}</h2>
          {panel === "comments" && <span className="count-badge">{info?.annotations.length ?? 0}</span>}
          <IconButton label="Close side panel" onClick={() => setPanelOpen(false)}><PanelLeftClose size={16} /></IconButton>
        </div>
        {panel === "tools" && <>
          <div className="tools-list">
            <button className={mode === "read" && tool === "select" ? "chosen" : ""} disabled={disabled} onClick={() => { changeMode("read"); setTool("select"); }}>
              <span className="tool-icon blue"><BookOpen size={19} /></span><span>Read a PDF</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button className={tool === "highlight" ? "chosen" : ""} disabled={editDisabled} onClick={() => chooseTool("highlight")}>
              <span className="tool-icon amber"><Highlighter size={19} /></span><span>Highlight text</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button className={tool === "note" ? "chosen" : ""} disabled={editDisabled} onClick={() => chooseTool("note")}>
              <span className="tool-icon violet"><MessageSquarePlus size={19} /></span><span>Add a note</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button disabled={editDisabled} onClick={() => chooseTool("text")}><span className="tool-icon blue"><Type size={19} /></span><span>Add text</span><ChevronRight size={14} className="list-arrow" /></button>
            <button disabled={editDisabled} onClick={() => chooseTool("ink")}><span className="tool-icon violet"><Pencil size={19} /></span><span>Draw with pen</span><ChevronRight size={14} className="list-arrow" /></button>
            <button className={mode === "organize" ? "chosen" : ""} disabled={disabled} onClick={() => changeMode("organize")}>
              <span className="tool-icon teal"><LayoutGrid size={19} /></span><span>Organize pages</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <div className="tool-list-divider" />
            <button disabled={disabled} onClick={() => showPanel("search")}>
              <span className="tool-icon neutral"><Search size={18} /></span><span>Find in document</span>
            </button>
            <button disabled={Boolean(busy)} onClick={open}>
              <span className="tool-icon neutral"><FolderOpen size={18} /></span><span>Open PDF</span><kbd>Cmd+O</kbd>
            </button>
            <button disabled={Boolean(busy)} onClick={() => setModal({ type: "create", size: "letter", landscape: false })}><span className="tool-icon neutral"><FilePlus2 size={18} /></span><span>Create a PDF</span></button>
          </div>
          <div className="panel-spacer" />
          <div className="quick-tip"><Highlighter size={18} /><h3>A small mark. A useful memory.</h3>
            <p>Select a passage, then choose Highlight. Your annotations travel with the PDF.</p>
          </div>
          <button className="shortcuts-link" onClick={() => setModal({ type: "shortcuts" })}><Keyboard size={16} />Keyboard shortcuts</button>
        </>}
        {panel === "bookmarks" && <BookmarksPanel pdf={pdf} disabled={disabled} onNavigate={navigateLink} />}
        {panel === "write" && <div className="panel-scroll writing-settings">
          <p>Add text or freehand marks without changing the original page content.</p>
          <div className="writing-tool-buttons">
            <button className={`button ${tool === "text" ? "primary" : "secondary"}`} disabled={editDisabled} onClick={() => chooseTool("text")}><Type size={16} />Add text</button>
            <button className={`button ${tool === "ink" ? "primary" : "secondary"}`} disabled={editDisabled} onClick={() => chooseTool("ink")}><Pencil size={16} />Pen</button>
          </div>
          <label className="setting-field">Text size (pt)<input type="number" min="6" max="96" step="1" value={textSize} onChange={(event) => setTextSize(event.target.value)} disabled={editDisabled} /></label>
          <label className="setting-field">Pen width (pt)<input type="number" min="0.5" max="12" step="0.5" value={penWidth} onChange={(event) => setPenWidth(event.target.value)} disabled={editDisabled} /></label>
          <label className="setting-field">Writing color<input type="color" value={writingColor} onChange={(event) => setWritingColor(event.target.value)} disabled={editDisabled} /></label>
          <p className="writing-hint">{tool === "text" ? "Click the page to place a text box. Text stays editable in Comments." : tool === "ink" ? "Drag on the page to draw. Each stroke can be removed or undone." : "Choose a writing tool, then work directly on the page."}</p>
          <p>Text uses an embedded Noto Sans font and supports Latin, Greek, and Cyrillic. This does not rewrite existing PDF text.</p>
          <button className="button secondary" onClick={() => showPanel("comments")}><MessageSquare size={15} />Review your marks</button>
        </div>}
        {panel === "search" && <>
          <div className="search-controls">
            <label className="search-field"><Search size={16} /><input ref={searchRef} aria-label="Search PDF text" value={query} maxLength={200}
              placeholder="Word or phrase" onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); chooseResult(activeResult + (event.shiftKey ? -1 : 1)); } }} />
              {query && <button aria-label="Clear search" title="Clear search" onClick={() => setQuery("")}><X size={14} /></button>}
            </label>
            <div className="search-options"><label><input type="checkbox" checked={matchCase} onChange={(event) => setMatchCase(event.target.checked)} />Match case</label>
              <div><IconButton label="Previous search result" disabled={!search.results.length} onClick={() => chooseResult(activeResult - 1)}><ChevronLeft size={16} /></IconButton>
                <IconButton label="Next search result" disabled={!search.results.length} onClick={() => chooseResult(activeResult + 1)}><ChevronRight size={16} /></IconButton></div>
            </div>
          </div>
          <div className="search-summary" role="status">
            {search.loading ? <><LoaderCircle size={13} className="spin" />Searching page {search.progress || 1}...</>
              : query.trim() ? `${search.results.length}${search.limited ? "+" : ""} ${search.results.length === 1 ? "match" : "matches"}` : "Search the document"}
          </div>
          <div className="panel-scroll search-results">
            {search.error && <p className="inline-error" role="alert">{search.error}</p>}
            {!query.trim() && <div className="panel-empty"><Search size={28} /><h3>Find your place.</h3><p>Search for a word or phrase to jump to the part that matters.</p></div>}
            {query.trim() && !search.loading && !search.results.length && !search.error &&
              <div className="panel-empty"><Search size={28} /><h3>{search.noText ? "No searchable text" : "No matches found"}</h3>
                <p>{search.noText ? "Scanned pages need OCR, which is outside this first release." : "Try a different word, or turn off Match case."}</p></div>}
            {search.results.map((result, index) =>
              <button key={`${result.page}-${result.index}`} className={`search-result ${activeResult === index ? "selected" : ""}`} onClick={() => chooseResult(index)}>
                <span>Page {result.page + 1}</span><p>{result.before}<mark>{result.match}</mark>{result.after}</p>
              </button>)}
          </div>
        </>}
        {panel === "comments" && <>
          <div className="comment-actions"><button className="button secondary" disabled={editDisabled} onClick={() => chooseTool("note")}><MessageSquarePlus size={15} />Add a note</button>
            <p>{tool === "note" ? "Click on the page to place your note." : "Your notes and highlights, together."}</p>
          </div>
          <div className="panel-scroll comments-list">
            {!info?.annotations.length && <div className="panel-empty"><MessageSquare size={28} /><h3>No comments yet</h3><p>Highlight a passage or leave a note to start your review.</p></div>}
            {info?.annotations.map((annotation) =>
              <article className={`comment-item ${selectedAnnotation === annotation.id ? "selected" : ""}`} key={annotation.id}>
                <div className="comment-author"><span className={`annotation-type ${annotation.kind}`}>
                  {annotation.kind === "highlight" ? <Highlighter size={14} /> : annotation.kind === "text" ? <Type size={14} /> : annotation.kind === "ink" ? <Pencil size={14} /> : <MessageSquare size={14} />}</span>
                  <strong>{annotation.author}</strong>
                  {annotation.editable && <IconButton label="Edit annotation text" disabled={editDisabled} onClick={() => setModal({ type: "edit-annotation", id: annotation.id, text: annotation.text })}><SquarePen size={13} /></IconButton>}
                  <IconButton label="Remove annotation" disabled={editDisabled || !annotation.deletable}
                    onClick={() => edit({ type: "remove_annotation", id: annotation.id }, "Removing annotation...")}><Trash2 size={13} /></IconButton>
                </div>
                <button className="comment-content" onClick={() => selectAnnotation(annotation.id)}>
                  <span className="comment-kind">{{ highlight: "Highlighted text", note: "Note", text: "Placed text", ink: "Pen drawing" }[annotation.kind]}</span>
                  <p>{annotation.text || "Highlighted passage"}</p><span className="comment-page">Page {annotation.page + 1}<ChevronRight size={12} /></span>
                </button>
              </article>)}
          </div>
        </>}
      </aside>}

      <main className={`document-area ${mode === "organize" ? "organizing" : ""}`} aria-label="Document">
        {mode === "organize" && info && pdf ? <>
          <div className="organize-heading"><div><h1>Every page in its place.</h1><p>Drag to reorder, or select a page and use the controls.</p></div>
            <button className="button secondary" onClick={() => changeMode("read")}><BookOpen size={15} />Back to document</button>
          </div>
          <div className="page-actions" role="toolbar" aria-label="Page authoring actions">
            <button className="button secondary" disabled={editDisabled} onClick={() => setModal({ type: "blank", size: "letter", landscape: false })}><FilePlus2 size={15} />Blank page</button>
            <button className="button secondary" disabled={editDisabled} onClick={() => importPages()}><FileInput size={15} />Insert PDF</button>
            <button className="button secondary" disabled={editDisabled} onClick={() => edit({ type: "duplicate", page }, "Duplicating page...", () => setPage(page + 1))}><Copy size={15} />Duplicate</button>
            <button className="button secondary" disabled={editDisabled} onClick={() => setModal({ type: "extract", range: String(page + 1) })}><FileOutput size={15} />Extract pages</button>
          </div>
          <div className="organize-toolbar"><span><strong>{info.pageCount}</strong> {info.pageCount === 1 ? "page" : "pages"}</span>
            <div className="organize-selection"><span>Page {page + 1} selected</span>
              <IconButton label="Move page earlier" disabled={editDisabled || page === 0} onClick={() => edit({ type: "move", from: page, to: page - 1 }, "Moving page...")}><ArrowLeft size={17} /></IconButton>
              <IconButton label="Move page later" disabled={editDisabled || page === info.pageCount - 1} onClick={() => edit({ type: "move", from: page, to: page + 1 }, "Moving page...")}><ArrowRight size={17} /></IconButton>
              <span className="toolbar-divider" />
              <IconButton label="Rotate page left" disabled={editDisabled} onClick={() => edit({ type: "rotate", page, degrees: -90 }, "Rotating page...")}><RotateCcw size={17} /></IconButton>
              <IconButton label="Rotate page right" disabled={editDisabled} onClick={() => edit({ type: "rotate", page, degrees: 90 }, "Rotating page...")}><RotateCw size={17} /></IconButton>
              <span className="toolbar-divider" />
              <IconButton label="Remove selected page" disabled={editDisabled || info.pageCount <= 1} onClick={() => setModal({ type: "delete", page })}><Trash2 size={17} /></IconButton>
            </div>
          </div>
          <div className="organize-grid">
            {info.pages.map((item, index) =>
              <div key={item.key} className={`organize-card ${page === index ? "selected" : ""} ${overPage === index ? "drag-over" : ""} ${draggedPage === index ? "dragging" : ""}`}
                draggable={!editDisabled}
                onDragStart={(event) => { setDraggedPage(index); event.dataTransfer.setData("text/plain", String(index)); event.dataTransfer.effectAllowed = "move"; }}
                onDragEnd={() => { setDraggedPage(null); setOverPage(null); }}
                onDragOver={(event) => { if (draggedPage !== null && !editDisabled) { event.preventDefault(); setOverPage(index); } }}
                onDragLeave={() => setOverPage(null)}
                onDrop={(event) => {
                  if (draggedPage === null) return;
                  event.preventDefault(); event.stopPropagation();
                  if (draggedPage !== index) edit({ type: "move", from: draggedPage, to: index }, "Reordering pages...");
                  setDraggedPage(null); setOverPage(null);
                }}>
                <button className="organize-page" aria-label={`Select page ${index + 1}`} aria-pressed={page === index} onClick={() => setPage(index)} onDoubleClick={() => changeMode("read")}>
                  <Thumbnail pdf={pdf} page={index} width={150} />
                  <span className="organize-page-label"><GripVertical size={14} />Page {index + 1}{page === index ? <span className="selection-check"><Check size={11} /></span> : <span />}</span>
                </button>
              </div>)}
          </div>
          {readOnly && <p className="organize-readonly">This signed PDF can be viewed but not reorganized.</p>}
        </> : <>
          <div className={`page-scroll ${tool === "hand" ? "pannable" : ""}`} ref={viewerRef}
            onPointerDown={(event) => {
              if (tool !== "hand" || event.button !== 0) return;
              const element = event.currentTarget;
              panRef.current = { x: event.clientX, y: event.clientY, left: element.scrollLeft, top: element.scrollTop };
              element.setPointerCapture(event.pointerId);
            }}
            onPointerMove={(event) => {
              const pan = panRef.current;
              if (pan && tool === "hand") {
                event.currentTarget.scrollLeft = pan.left - (event.clientX - pan.x);
                event.currentTarget.scrollTop = pan.top - (event.clientY - pan.y);
              }
            }}
            onPointerUp={() => { panRef.current = null; }}
            onPointerCancel={() => { panRef.current = null; }}
          >
            <div className="page-stage">
              {pdf && info ? <PdfPage pdf={pdf} page={page} zoom={zoom} width={viewportSize.width} height={viewportSize.height}
                tool={tool} disabled={editDisabled} annotations={info.annotations.filter((annotation) => annotation.page === page)}
                selectedAnnotation={selectedAnnotation}
                query={panel === "search" && panelOpen ? query : ""} matchCase={matchCase}
                activeMatch={currentMatch?.page === page ? currentMatch.index : -1}
                onScale={onScale} onHighlight={highlight} onNote={(point) => setModal({ type: "note", page, point, text: "" })}
                onText={(rect) => setModal({ type: "text", page, rect, text: "" })} onInk={draw} onLink={navigateLink}
                penColor={writingColor} penWidth={Number.isFinite(Number(penWidth)) ? Math.max(0.5, Math.min(12, Number(penWidth))) : 2}
                onAnnotation={selectAnnotation} onError={notifyError}
              /> : <div className="workspace-empty">
                {busy ? <><LoaderCircle size={30} className="spin" /><h2>{busy}</h2><p>Your document stays on this device.</p></>
                  : <><img src="/folio.svg" width="58" height="58" alt="" /><h1>A little less paperwork.</h1><p>Open a PDF to read, review, and make it your own.</p>
                    <button className="button primary" onClick={open}><FolderOpen size={16} />Open PDF</button>
                    <button className="text-button" onClick={() => requestAction({ type: "example" })}>Open the example document</button></>}
              </div>}
            </div>
          </div>
          {info && <div className="quick-tools" role="toolbar" aria-label="Review tools">
            {quickTools.map((item, index) => <span key={item.tool}>
              {index === 2 && <span className="quick-tool-divider" />}
              <IconButton label={item.label} active={tool === item.tool}
                disabled={disabled || readOnly && !["select", "hand"].includes(item.tool)}
                onClick={() => chooseTool(item.tool)}><item.icon size={19} />
                {item.tool === "highlight" && <span className="highlighter-swatch" />}
              </IconButton>
            </span>)}
          </div>}
          {info && <div className="view-controls" role="toolbar" aria-label="Page and zoom controls">
            <IconButton label="Previous page" disabled={disabled || page === 0} onClick={() => changePage(-1)}><ChevronLeft size={17} /></IconButton>
            <div className="page-number"><input aria-label="Page number" value={pageInput} inputMode="numeric" disabled={disabled}
              onChange={(event) => setPageInput(event.target.value)}
              onBlur={(event) => {
                const next = pageFromInput(event.currentTarget.value, info.pageCount);
                if (next === null) { setPageInput(String(page + 1)); setToast(`Enter a page from 1 to ${info.pageCount}.`); }
                else { setPage(next); setPageInput(String(next + 1)); }
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
                if (event.key === "Escape") { event.currentTarget.value = String(page + 1); setPageInput(String(page + 1)); event.currentTarget.blur(); }
              }} /><span>/ {info.pageCount}</span></div>
            <IconButton label="Next page" disabled={disabled || page === info.pageCount - 1} onClick={() => changePage(1)}><ChevronRight size={17} /></IconButton>
            <span className="toolbar-divider" />
            <IconButton label="Zoom out" disabled={disabled} onClick={() => changeZoom(1 / 1.2)}><Minus size={16} /></IconButton>
            <select aria-label="Zoom level" title={`Current zoom: ${zoomPercent}%`} value={customPercent} disabled={disabled} onChange={(event) => {
              const value = event.target.value;
              setZoom(value === "page" || value === "width" ? { mode: value } : { mode: "custom", scale: Number(value) / 75 });
            }}>
              <option value="page">Fit page</option><option value="width">Fit width</option>
              {zoom.mode === "custom" && !zoomOptions.includes(customPercent) && <option value={customPercent}>{customPercent}%</option>}
              {zoomOptions.map((value) => <option value={value} key={value}>{value}%</option>)}
            </select>
            <IconButton label="Zoom in" disabled={disabled} onClick={() => changeZoom(1.2)}><Plus size={16} /></IconButton>
            <span className="toolbar-divider" />
            <IconButton label="Fit page" active={zoom.mode === "page"} disabled={disabled} onClick={() => setZoom({ mode: "page" })}><ScanLine size={17} /></IconButton>
          </div>}
          {tool === "note" && !disabled && <div className="tool-instruction" role="status"><MessageSquarePlus size={14} />Click on the page to add a note.</div>}
          {tool === "highlight" && !disabled && <div className="tool-instruction" role="status"><Highlighter size={14} />Drag across text to highlight.</div>}
          {tool === "text" && !disabled && <div className="tool-instruction" role="status"><Type size={14} />Click to place a text box.</div>}
          {tool === "ink" && !disabled && <div className="tool-instruction" role="status"><Pencil size={14} />Drag to draw. Undo removes a stroke.</div>}
        </>}
        {busy && info && <div className="document-busy" role="status"><LoaderCircle size={14} className="spin" />{busy}</div>}
      </main>

      {railOpen && info && pdf && mode !== "organize" && <aside className="page-rail" aria-label="Page thumbnails">
        <div className="page-rail-heading"><h2>Pages</h2><span>{info.pageCount}</span><IconButton label="Hide page thumbnails" onClick={() => setRailOpen(false)}><PanelRightClose size={15} /></IconButton></div>
        <div className="thumbnail-list">
          {info.pages.map((item, index) =>
            <button className={`page-thumb ${page === index ? "selected" : ""}`} key={item.key}
              aria-label={`Go to page ${index + 1}`} aria-current={page === index ? "page" : undefined}
              disabled={Boolean(busy)} onClick={() => setPage(index)}>
              <Thumbnail pdf={pdf} page={index} /><span className="thumbnail-number">{index + 1}</span>
            </button>)}
        </div>
        <button className="rail-organize" disabled={disabled} onClick={() => changeMode("organize")}><LayoutGrid size={14} />Organize pages</button>
      </aside>}
      <nav className="utility-rail" aria-label="Document panels">
        <IconButton label="Toggle page thumbnails" active={railOpen && mode !== "organize"} disabled={disabled} onClick={() => { setRailOpen((open) => !open); if (mode === "organize") setMode("read"); }}><FileText size={18} /></IconButton>
        <IconButton label="Show comments" active={panelOpen && panel === "comments"} disabled={disabled} onClick={() => showPanel("comments")}><MessageSquare size={18} />{Boolean(info?.annotations.length) && <span className="utility-dot" />}</IconButton>
        <IconButton label="Show bookmarks" active={panelOpen && panel === "bookmarks"} disabled={disabled} onClick={() => showPanel("bookmarks")}><Bookmark size={18} /></IconButton>
        <IconButton label="Search document" active={panelOpen && panel === "search"} disabled={disabled} onClick={() => showPanel("search")}><Search size={18} /></IconButton>
        <span className="utility-spacer" />
        <IconButton label="Rotate current page clockwise" disabled={editDisabled} onClick={() => edit({ type: "rotate", page, degrees: 90 }, "Rotating page...")}><RotateCw size={18} /></IconButton>
      </nav>
    </div>

    <footer className="statusbar">
      <div>{busy ? <><LoaderCircle size={12} className="spin" />{busy}</> : pendingDraft ? <><span className="status-dot dirty" />Uncommitted writing</> : info?.dirty ? <><span className="status-dot dirty" />Unsaved changes</> : <><Check size={12} className="status-check" />No unsaved changes</>}</div>
      <div className="document-status">{info && <><span>{info.pageCount} {info.pageCount === 1 ? "page" : "pages"}</span><span className="status-separator" /><span>{formatBytes(info.byteLength)}</span></>}</div>
      <div className="privacy-status"><ShieldCheck size={12} /><span>{runtimeIssue ? "Desktop runtime unavailable" : native ? "Local files. No uploads." : "Preview uses the real Rust engine"}</span></div>
    </footer>

    {dragActive && <div className="drop-overlay"><div><FolderOpen size={38} /><h2>Drop your PDF here.</h2><p>Open one document at a time.</p></div></div>}
    {error && !modal && <div className="notification error-notification" role="alert"><span><strong>That didn&apos;t work.</strong>{error}</span><IconButton label="Dismiss error" onClick={() => setError(null)}><X size={16} /></IconButton></div>}
    {toast && <div className="notification toast" role="status"><Check size={16} /><span>{toast}</span><IconButton label="Dismiss notification" onClick={() => setToast(null)}><X size={14} /></IconButton></div>}

    {(modal?.type === "create" || modal?.type === "blank") && <Dialog title={modal.type === "create" ? "Create a PDF" : "Insert a blank page"} error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Cancel</button>
        <button className="button primary" disabled={Boolean(busy)} onClick={() => {
          const size = PAGE_SIZES[modal.size];
          const width = modal.landscape ? size.height : size.width;
          const height = modal.landscape ? size.width : size.height;
          if (modal.type === "create") { setModal(null); requestAction({ type: "create", width, height }); }
          else edit({ type: "blank_page", at: page + 1, width, height }, "Inserting blank page...", () => { setModal(null); setPage(page + 1); });
        }}><FilePlus2 size={15} />{modal.type === "create" ? "Create PDF" : "Insert page"}</button></>}>
      <p>{modal.type === "create" ? "Start a new document, then add text, draw, or insert pages from another PDF." : `A new page will be inserted after page ${page + 1}.`}</p>
      <label className="setting-field">Page size<select data-autofocus value={modal.size} onChange={(event) => {
        if (event.target.value === "letter" || event.target.value === "a4") setModal({ ...modal, size: event.target.value });
      }}>{Object.entries(PAGE_SIZES).map(([id, size]) => <option key={id} value={id}>{size.label}</option>)}</select></label>
      <label className="checkbox-setting"><input type="checkbox" checked={modal.landscape} onChange={(event) => setModal({ ...modal, landscape: event.target.checked })} />Landscape orientation</label>
    </Dialog>}

    {modal?.type === "text" && <Dialog title={`Add text on page ${modal.page + 1}`} error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Cancel</button>
        <button className="button primary" disabled={Boolean(busy) || !modal.text.trim()} onClick={() => {
          try {
            edit({ type: "text", page: modal.page, rect: modal.rect, text: modal.text,
              size: parseWritingNumber(textSize, 6, 96, "Text size"), color: rgbFromHex(writingColor) },
              "Adding text...", () => setModal(null));
          } catch (error) { setError(errorMessage(error)); }
        }}><Type size={15} />Place text</button></>}>
      <label htmlFor="placed-text" className="note-label">Text to place</label>
      <textarea id="placed-text" value={modal.text} maxLength={4000} rows={5} placeholder="Write directly on your PDF."
        onChange={(event) => setModal({ ...modal, text: event.target.value })} />
      <label className="setting-field">Text size (pt)<input type="number" min="6" max="96" value={textSize} onChange={(event) => setTextSize(event.target.value)} /></label>
      <p className="muted">This creates editable PDF text, with a bundled font. It does not replace existing document text. Latin, Greek, and Cyrillic are supported.</p>
    </Dialog>}

    {modal?.type === "edit-annotation" && <Dialog title="Edit annotation text" error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Cancel</button>
        <button className="button primary" disabled={Boolean(busy) || !modal.text.trim()} onClick={() => edit({ type: "set_annotation_text", id: modal.id, text: modal.text }, "Updating annotation...", () => setModal(null))}>Save text</button></>}>
      <label htmlFor="annotation-text" className="note-label">Annotation text</label>
      <textarea id="annotation-text" maxLength={4000} rows={5} value={modal.text} onChange={(event) => setModal({ ...modal, text: event.target.value })} />
      <p className="muted">Placed text keeps its original size, color, and text box. If the replacement does not fit, shorten it.</p>
    </Dialog>}

    {modal?.type === "extract" && <Dialog title="Extract pages" error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Cancel</button>
        <button className="button primary" disabled={Boolean(busy) || !modal.range.trim()} onClick={() => {
          void run("Exporting pages...", async () => {
            const current = infoRef.current;
            if (!current) throw new Error("Open a PDF before extracting pages.");
            if (await bridge.extract(current, parsePageRange(modal.range, current.pageCount))) {
              setModal(null); setToast("Pages exported as a separate PDF. The working document is unchanged.");
            }
          });
        }}><FileOutput size={15} />Export pages</button></>}>
      <p>Create a separate PDF in the order you enter. The working document will not be changed.</p>
      <label className="setting-field">Pages<input data-autofocus type="text" value={modal.range} placeholder="1-3, 5" onChange={(event) => setModal({ ...modal, range: event.target.value })} /></label>
      <p className="muted">Use page numbers or ranges, such as 1-3, 5. This is not secure redaction or metadata sanitization.</p>
    </Dialog>}

    {modal?.type === "external" && <Dialog title="Open this link?" error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Cancel</button>
        <button className="button primary" disabled={Boolean(busy)} onClick={() => {
          void run("Opening link...", async () => { await bridge.external(modal.url); setModal(null); });
        }}><ExternalLink size={15} />Open link</button></>}>
      <p>This address is supplied by the PDF. It will open in your default browser or email application.</p>
      <p className="external-address">{modal.url}</p>
    </Dialog>}

    {modal?.type === "note" && <Dialog title={`A note on page ${modal.page + 1}`} error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Cancel</button>
        <button className="button primary" disabled={Boolean(busy) || !modal.text.trim()} onClick={() => {
          const change: Edit = { type: "note", page: modal.page, point: modal.point, text: modal.text };
          edit(change, "Adding note...", () => setModal(null));
        }}><MessageSquarePlus size={15} />Add note</button></>}>
      <label className="note-label" htmlFor="note-text">What would you like to remember?</label>
      <textarea id="note-text" value={modal.text} maxLength={4000} rows={5} placeholder="Leave a thought for your next review."
        onChange={(event) => setModal({ ...modal, text: event.target.value })} />
      <p className="note-privacy"><ShieldCheck size={12} />Saved inside the PDF, not sent to a server.</p>
    </Dialog>}

    {modal?.type === "delete" && <Dialog title={`Remove page ${modal.page + 1}?`} onClose={() => setModal(null)}
      actions={<><button className="button secondary" onClick={() => setModal(null)}>Keep page</button>
        <button className="button danger" onClick={() => { const target = modal.page; setModal(null); edit({ type: "delete", page: target }, "Removing page..."); }}><Trash2 size={15} />Remove page</button></>}>
      <p>The page will be removed from this working copy. You can undo the change before closing the document.</p>
    </Dialog>}

    {modal?.type === "unsaved" && <Dialog title="Keep your changes?" error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Cancel</button>
        <button className="button text-danger" disabled={Boolean(busy)} onClick={() => {
          const action = modal.action; setModal(null); void run("Opening PDF...", () => perform(action));
        }}>Discard changes</button>
        <button className="button primary" disabled={Boolean(busy)} onClick={() => {
          const action = modal.action;
          void run(native ? "Saving a copy..." : "Preparing download...", async () => {
            const current = infoRef.current;
            if (!current) throw new Error("The current document is unavailable.");
            const saved = await bridge.save(current, true);
            if (!saved) return;
            updateInfo(saved); setModal(null); await perform(action);
          });
        }}>{native ? "Save a copy" : "Download & continue"}</button></>}>
      <p>You have unsaved changes in <strong>{info?.name}</strong>. Save an edited copy before leaving this document.</p>
      {!native && <p className="muted">Browser preview downloads a copy instead of writing to your filesystem.</p>}
    </Dialog>}

    {modal?.type === "shortcuts" && <Dialog title="A few useful shortcuts" onClose={() => setModal(null)}>
      <dl className="shortcut-list">
        {[["New PDF", "Command + N"], ["Open PDF", "Command + O"], ["Save", "Command + S"], ["Save a copy", "Command + Shift + S"],
          ["Find in document", "Command + F"], ["Undo / redo", "Command + Z / Shift + Z"],
          ["Previous / next page", "Left / Right"], ["Zoom in / out", "Command + Plus / Minus"],
          ["Select / pan", "V / H"], ["Highlight / add note", "M / N"], ["Add text / pen", "T / D"]].map(([label, key]) =>
          <div key={label}><dt>{label}</dt><dd><kbd>{key}</kbd></dd></div>)}
      </dl><p className="muted">On Windows and Linux, use Control in place of Command.</p>
    </Dialog>}

    {modal?.type === "about" && <Dialog title="Meet Folio." onClose={() => setModal(null)}>
      <div className="about-brand"><img src="/folio.svg" width="52" height="52" alt="" /><div><h3>A little less paperwork.</h3><p>Version 0.1.0 - macOS-first MVP</p></div></div>
      <p>An independently built PDF workspace. Tauri connects this interface to a Rust document engine; PDF.js renders the pages.</p>
      <p>Create, read, search, add editable text or drawings, review, combine, and extract PDF pages. Existing-text reflow, interactive forms, OCR, Office conversion, encryption, and cryptographic signing are not included.</p>
      <p className="muted">Not affiliated with Adobe. No code, fonts, icons, or artwork have been extracted from Acrobat. Open-source dependencies carry their own licenses. Files stay on this device. This is an MVP, not a full Acrobat replacement.</p>
      {!native && <div className="preview-explanation"><FileText size={17} /><p><strong>You are in browser preview.</strong>The same Rust engine processes your edits through a loopback-only development bridge. Use the desktop app for native open/save dialogs.</p></div>}
    </Dialog>}
  </div>;
}
