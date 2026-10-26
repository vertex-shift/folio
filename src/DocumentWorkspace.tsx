import { useCallback, useEffect, useImperativeHandle, useRef, useState } from "react";
import type { ClipboardEvent, Ref } from "react";
import {
  ArrowLeft, ArrowRight, BookOpen, Check, ChevronLeft, ChevronRight,
  ClipboardList, Copy, FileText, FolderOpen, GripVertical, Hand, Highlighter,
  Keyboard, LayoutGrid, LoaderCircle, LockKeyhole, MessageSquare,
  MessageSquarePlus, Minus, MousePointer2, PanelLeft, PanelLeftClose,
  PanelRightClose, Plus, Redo2, RotateCcw, RotateCw, ScanLine, Search,
  ShieldCheck, Trash2, Undo2, X, FileKey2,
  Bookmark, FilePlus2, FileInput, FileOutput, Pencil, Type, SquarePen, ExternalLink, Eraser, ImagePlus, Hash, Stamp, Crop,
  Info, Paperclip,
  Printer, FileDown, Minimize2, Files,
} from "lucide-react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import { version as appVersion } from "../package.json";
import Dialog from "./components/Dialog";
import PasswordDialog from "./components/PasswordDialog";
import SelectionIndicator from "./components/SelectionIndicator";
import IconButton from "./components/IconButton";
import PdfPage, { Thumbnail } from "./components/PdfPage";
import ContinuousPages from "./components/ContinuousPages";
import BookmarksPanel from "./components/BookmarksPanel";
import ExistingTextPanel from "./components/ExistingTextPanel";
import RedactionPanel from "./components/RedactionPanel";
import SignaturesPanel from "./features/signatures/SignaturesPanel";
import SigningDialog from "./features/signatures/SigningDialog";
import { signatureBridge } from "./features/signatures/bridge";
import type { SignatureReport, SigningIdentity } from "./features/signatures/types";
import AuthoringPanel from "./components/AuthoringPanel";
import FormsPanel from "./components/FormsPanel";
import DocumentPropertiesPanel from "./components/DocumentPropertiesPanel";
import AttachmentsPanel from "./components/AttachmentsPanel";
import ProtectionCopyDialog from "./components/ProtectionCopyDialog";
import OcrPanel, { OcrJobProgress } from "./components/OcrPanel";
import OutputDialog from "./components/OutputDialog";
import { bridge, native, runtimeIssue } from "./lib/bridge";
import { createDraftSynchronizer, hasDocumentDrafts } from "./lib/draftProtection";
import { documentUtilities } from "./lib/documentUtilitiesBridge";
import { MAX_MANAGEMENT_SOURCE_BYTES, metadataChanged } from "./lib/documentUtilitiesModel";
import type { AttachmentInfo, BookmarkInfo, CopyProtection, MetadataDraft } from "./lib/documentUtilitiesModel";
import { errorMessage, formatBytes, pageFromInput } from "./lib/geometry";
import { loadPdf } from "./lib/pdf";
import { useSearch } from "./lib/useSearch";
import { useExistingText } from "./lib/useExistingText";
import { useAuthoring } from "./lib/useAuthoring";
import { useForms } from "./lib/useForms";
import FormDesignerPanel from "./features/formsDesigner/FormDesignerPanel";
import { useFormDesigner } from "./features/formsDesigner/useFormDesigner";
import { formDesignerBridge } from "./features/formsDesigner/bridge";
import { widgetPlacement } from "./features/formsDesigner/model";
import { decimal } from "./lib/authoring";
import type { ControlKind, FlattenInfo } from "./features/formsDesigner/types";
import type { FormWidget } from "./lib/formsTypes";
import { matchExistingText } from "./lib/existingText";
import { PAGE_SIZES, parsePageRange, parseWritingNumber, rgbFromHex } from "./lib/pages";
import { destinationPage, safeExternalUrl } from "./lib/navigation";
import { MAX_REDACTION_MARKS, renderRedactionPages } from "./lib/redaction";
import { MAX_OCR_PAGES, recognizeOcrPages } from "./lib/ocr";
import type { OcrOptions, OcrOutcome, OcrPlan, OcrProgress } from "./lib/ocrTypes";
import type { DocumentInfo, Edit, ExistingTextRun, Mode, OpenOutcome, Panel, PasswordChallenge, PdfRect, Point, Quad, RedactionMark, RedactionPlan, ReplacementFont, Tool, Zoom } from "./lib/types";
import "./authoring.css";
import "./document-utilities.css";
import "./features/formsDesigner/styles.css";
import type { OutputAction } from "./lib/output";
import type { SourceState } from "./features/comparison/model";

type Action = { type: "open" | "example" | "close" }
  | { type: "create"; width: number; height: number };
type Modal = { type: "shortcuts" | "about" }
  | { type: "password"; challenge: PasswordChallenge }
  | { type: "unsaved"; action: Action }
  | { type: "delete"; page: number }
  | { type: "note"; page: number; point: Point; text: string }
  | { type: "text"; page: number; rect: [number, number, number, number]; text: string }
  | { type: "edit-annotation"; id: string; text: string }
  | { type: "create" | "blank"; size: keyof typeof PAGE_SIZES; landscape: boolean }
  | { type: "extract"; range: string }
  | { type: "redaction-export"; dpi: 150 | 300; acknowledged: boolean }
  | { type: "redaction-clear" }
  | { type: "redaction-leave"; action: Action }
  | { type: "sign-copy"; documentId: number; revision: number; report: SignatureReport }
  | { type: "form-discard"; action?: Action }
  | { type: "static-form-copy"; documentId: number; revision: number; plan: FlattenInfo; acknowledged: boolean }
  | { type: "bookmark-create"; documentId: number; revision: number; text: string; page: string; parent: string | null; at: number; entries: BookmarkInfo[] }
  | { type: "bookmark-rename"; documentId: number; revision: number; id: string; text: string }
  | { type: "bookmark-delete"; entry: BookmarkInfo }
  | { type: "attachment-remove" | "attachment-export"; attachment: AttachmentInfo }
  | { type: "protection-copy" }
  | { type: "external"; url: string };

type SourceTextModal = {
  type: "edit-existing"; documentId: number; revision: number; page: number; path: number[];
  expected: string; text: string; originalFont: string; font: ReplacementFont;
};

const quickTools = [
  { tool: "select", icon: MousePointer2, label: "Select text (V)" },
  { tool: "hand", icon: Hand, label: "Pan document (H)" },
  { tool: "highlight", icon: Highlighter, label: "Highlight text (M)" },
  { tool: "note", icon: MessageSquarePlus, label: "Add a note (N)" },
  { tool: "text", icon: Type, label: "Add text (T)" },
  { tool: "ink", icon: Pencil, label: "Draw with pen (D)" },
  { tool: "edit-text", icon: SquarePen, label: "Edit existing text (E)" },
  { tool: "redact", icon: Eraser, label: "Redact areas (R)" },
] as const;
const panelLabels: Record<Panel, string> = {
  tools: "Document tools", search: "Find in document", comments: "Comments",
  bookmarks: "Bookmarks", write: "Writing tools", edit: "Edit existing text", redact: "Redact a PDF",
  signatures: "Digital signatures",
  authoring: "Page authoring",
  forms: "Fill form fields",
  "form-designer": "Design form fields",
  properties: "Document properties", attachments: "File attachments",
  ocr: "Recognize scanned text",
};

function typingTarget(target: EventTarget | null): boolean {
  return target instanceof Element && Boolean(target.closest("input, textarea, select, [contenteditable='true']"));
}

export interface DocumentControls {
  menu(id: string): void;
  close(): void;
  comparisonState(): SourceState;
  prepareComparison(): Promise<void>;
}

export interface DocumentActivity {
  busy: boolean;
  pendingDraft: boolean;
  modal: boolean;
  outputPending: boolean;
}

interface Props {
  document: DocumentInfo | null;
  active: boolean;
  blocked: boolean;
  startWriting?: boolean;
  onOpen(): void;
  onCompare(): void;
  onOpened(document: DocumentInfo): void;
  onCreated(document: DocumentInfo): void;
  onInfo(document: DocumentInfo): void;
  onClose(document: DocumentInfo): Promise<void>;
  onActivity(id: number, activity: DocumentActivity | null): void;
  ref?: Ref<DocumentControls>;
}

export default function DocumentWorkspace(props: Props) {
  const [info, setInfo] = useState<DocumentInfo | null>(props.document);
  const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null);
  const [page, setPage] = useState(0);
  const [pageInput, setPageInput] = useState("1");
  const [mode, setMode] = useState<Mode>(props.startWriting ? "write" : "read");
  const [panel, setPanel] = useState<Panel>(props.startWriting ? "write" : "tools");
  const [panelOpen, setPanelOpen] = useState(window.innerWidth >= 1100);
  const [railOpen, setRailOpen] = useState(window.innerWidth >= 700);
  const [tool, setTool] = useState<Tool>("select");
  const [zoom, setZoom] = useState<Zoom>({ mode: "page" });
  const [actualScale, setActualScale] = useState(1);
  const [query, setQuery] = useState("");
  const [matchCase, setMatchCase] = useState(false);
  const [activeResult, setActiveResult] = useState(0);
  const [selectedAnnotation, setSelectedAnnotation] = useState<string | null>(null);
  const [designerWidget, setDesignerWidget] = useState(0);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [modal, setModal] = useState<Modal | SourceTextModal | null>(null);
  const [metadataDraft, setMetadataDraft] = useState<MetadataDraft | null>(null);
  const pendingMetadata = metadataChanged(metadataDraft);
  const [redactions, setRedactions] = useState<RedactionMark[]>([]);
  const [ocrRange, setOcrRange] = useState("");
  const [ocrDpi, setOcrDpi] = useState<OcrOptions["dpi"]>(300);
  const [ocrPolicy, setOcrPolicy] = useState<OcrOptions["existingText"]>("skip");
  const [ocrOutcome, setOcrOutcome] = useState<OcrOutcome | null>(null);
  const [ocrProgress, setOcrProgress] = useState<OcrProgress | null>(null);
  const [ocrCanceling, setOcrCanceling] = useState(false);
  const ocrJob = useRef<{ controller: AbortController; documentId: number; revision: number } | null>(null);
  const [outputMode, setOutputMode] = useState<OutputAction | null>(null);
  const gesturePages = useRef(new Set<number>());
  const [gestureDraft, setGestureDraft] = useState(false);
  const redactionsRef = useRef(redactions);
  redactionsRef.current = redactions;
  const redactionCounter = useRef(0);
  const [redactedCopyPrepared, setRedactedCopyPrepared] = useState(false);
  const exportController = useRef<AbortController | null>(null);
  const [redactionProgress, setRedactionProgress] = useState<{
    page: number; count: number; stage: "rendering" | "saving"; canceling: boolean;
  } | null>(null);
  const pendingWriting = modal?.type === "edit-existing"
    ? modal.text !== modal.expected || modal.font !== "original"
    : Boolean(modal && "text" in modal && modal.text.trim());
  const writingDraftRef = useRef(pendingWriting || pendingMetadata || gestureDraft);
  writingDraftRef.current = pendingWriting || pendingMetadata || gestureDraft;
  const [syncDraft] = useState(() => createDraftSynchronizer(bridge.draft));
  const [draggedPage, setDraggedPage] = useState<number | null>(null);
  const [overPage, setOverPage] = useState<number | null>(null);
  const [viewportSize, setViewportSize] = useState({ width: 800, height: 740 });
  const [textSize, setTextSize] = useState("14");
  const [penWidth, setPenWidth] = useState("2");
  const [writingColor, setWritingColor] = useState("#253b57");
  const viewerRef = useRef<HTMLDivElement>(null);
  const workspaceRef = useRef<HTMLDivElement>(null);
  const readingPosition = useRef<{ page: number; fraction: number } | undefined>(undefined);
  const activeRef = useRef(props.active);
  activeRef.current = props.active;
  const loaded = useRef(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const importRef = useRef<HTMLInputElement>(null);
  const imageRef = useRef<HTMLInputElement>(null);
  const attachmentRef = useRef<HTMLInputElement>(null);
  const attachmentPickerRevision = useRef<{ documentId: number; revision: number } | null>(null);
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
  const existingText = useExistingText(info, page, tool === "edit-text" || panelOpen && panel === "edit");
  const forms = useForms(info, panelOpen && panel === "forms");
  const designer = useFormDesigner(info, panel === "form-designer" || tool === "form-widget");
  const pendingDesign = Boolean(designer.draft?.dirty);
  const readOnly = Boolean(info?.readOnlyReason);
  const disabled = Boolean(busy) || !info;
  const authoring = useAuthoring({ info, page, current: () => infoRef.current,
    blocked: () => busyRef.current || redactionsRef.current.length > 0 || forms.pending() || designer.pending() || writingDraftRef.current,
    run, adopt, onError: setError, onToast: setToast,
    onSelect: (kind) => {
      setMode("write"); setPanel("authoring"); setPanelOpen(true);
      setTool(kind === "image" || kind === "crop" ? kind : "select");
    },
    onDraftCleared: async () => {
      const current = infoRef.current;
      if (current) await syncDraft(current.id, draftProjection());
    },
  });
  const pendingDraft = hasDocumentDrafts({ writing: pendingWriting || pendingMetadata || gestureDraft,
    formFilling: forms.count > 0, formDesign: pendingDesign, pageAuthoring: authoring.pending, redaction: redactions.length > 0 });
  const pendingDraftRef = useRef(pendingDraft);
  pendingDraftRef.current = pendingDraft;
  const authoringDisabled = disabled || readOnly || redactions.length > 0 || forms.count > 0 || pendingMetadata || pendingDesign;
  const editDisabled = authoringDisabled || authoring.pending;
  const redactDisabled = disabled || readOnly || authoring.pending || forms.count > 0 || pendingMetadata || pendingDesign;
  const managementTooLarge = Boolean(info && info.byteLength > MAX_MANAGEMENT_SOURCE_BYTES);
  const managementEditDisabled = editDisabled || managementTooLarge;
  const utilityPanel = panelOpen && ["properties", "attachments", "bookmarks"].includes(panel);
  const redactionIntent = tool === "redact" || redactions.length > 0;
  const saveLabel = redactionIntent ? "Export redacted copy" : native ? "Save a copy" : "Download PDF";

  const notifyError = useCallback((message: string) => setError(message), []);
  const onScale = useCallback((scale: number) => setActualScale(scale), []);
  const documentId = props.document?.id ?? 0;

  useImperativeHandle(props.ref, () => ({
    menu: (id) => actionRef.current.menu(id),
    close: () => actionRef.current.action({ type: "close" }),
    comparisonState: () => ({ document: infoRef.current, pending: draftProjection(), busy: busyRef.current }),
    prepareComparison: async () => {
      const current = infoRef.current;
      if (!current) throw new Error("This comparison source is no longer open.");
      await syncDraft(current.id, draftProjection());
      if (draftProjection()) throw new Error(`${current.name}: apply or discard pending writing, forms, metadata, authoring, redaction or gesture drafts before comparing.`);
      if (infoRef.current?.id !== current.id || infoRef.current.revision !== current.revision) {
        throw new Error(`${current.name}: the comparison source changed. Review the current PDF again.`);
      }
      if (busyRef.current) throw new Error(`${current.name}: finish the current action before comparing.`);
    },
  }), []);

  useEffect(() => {
    props.onActivity(documentId, { busy: Boolean(busy) || gestureDraft, pendingDraft, modal: Boolean(modal) || Boolean(outputMode), outputPending: Boolean(outputMode) });
  }, [documentId, busy, gestureDraft, pendingDraft, modal, outputMode, props.onActivity]);

  useEffect(() => () => props.onActivity(documentId, null), [documentId, props.onActivity]);

  useEffect(() => {
    const job = ocrJob.current;
    if (job && (!props.active || props.document?.id !== job.documentId || props.document.revision !== job.revision)) {
      job.controller.abort();
    }
  }, [props.active, props.document?.id, props.document?.revision]);

  useEffect(() => () => ocrJob.current?.controller.abort(), []);

  useEffect(() => {
    if (ocrOutcome && ocrOutcome.document.revision !== info?.revision) setOcrOutcome(null);
  }, [info?.revision, ocrOutcome]);

  function updateInfo(next: DocumentInfo) {
    infoRef.current = next;
    setInfo(next);
    props.onInfo(next);
  }

  function onGestureDraft(page: number, pending: boolean) {
    if (pending) gesturePages.current.add(page);
    else gesturePages.current.delete(page);
    const active = gesturePages.current.size > 0;
    writingDraftRef.current = pendingWriting || pendingMetadata || active;
    pendingDraftRef.current = draftProjection();
    setGestureDraft(active);
  }

  async function adopt(next: DocumentInfo) {
    const previous = infoRef.current;
    if (designer.pending() && (previous?.id !== next.id || previous.revision !== next.revision)) {
      throw new Error("Apply or explicitly discard the form-design draft before changing this PDF.");
    }
    if (forms.pending() && (previous?.id !== next.id || previous.revision !== next.revision)) {
      throw new Error("Apply or explicitly discard the pending form changes before changing this PDF.");
    }
    if (redactionsRef.current.length && (previous?.id !== next.id || previous.revision !== next.revision)) {
      throw new Error("Clear the pending redaction marks before changing the original PDF.");
    }
    const previousKey = previous?.pages[pageRef.current]?.key;
    const sameDocument = previous?.id === next.id;
    const matchingPage = sameDocument ? next.pages.findIndex((item) => item.key === previousKey) : 0;
    updateInfo(next);
    setPage(matchingPage >= 0 ? matchingPage : Math.min(pageRef.current, next.pageCount - 1));
    setSelectedAnnotation(null);
    if (!sameDocument) {
      updateRedactions([]);
      redactionCounter.current = 0;
      setModal(null);
      setMetadataDraft(null);
      setPage(0);
      setMode("read");
      setTool("select");
      setQuery("");
      setPanel("tools");
      setZoom({ mode: "page" });
    }
    setPdf(null);
    const bytes = await bridge.bytes(next, true);
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

  async function finishOpen(outcome: OpenOutcome) {
    if (outcome.status === "failed") throw new Error(`${outcome.name}: ${outcome.message}`);
    if (outcome.status === "password_required") {
      setModal({ type: "password", challenge: outcome });
      return;
    }
    await adopt(outcome.document);
    setModal(null);
    if (outcome.importedAt !== null) {
      setPage(outcome.importedAt);
      setMode("organize");
      setToast("PDF pages inserted. Undo is available.");
    } else if (outcome.document.passwordProtected) {
      setToast("Protected PDF opened. Saved copies keep its password protection.");
    }
  }

  function cancelPassword(requestId: number) {
    void run("Canceling PDF opening...", async () => {
      await bridge.cancelOpen(requestId);
      setModal(null);
    });
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
        props.onOpen();
        break;
      }
      case "example": props.onOpened(await bridge.example()); break;
      case "close": {
        const current = infoRef.current;
        if (!current) throw new Error("This PDF tab is no longer open.");
        await syncDraft(current.id, draftProjection());
        await props.onClose(current);
        break;
      }
      case "create":
        props.onCreated(await bridge.create(action.width, action.height));
        break;
    }
  }

  function requestAction(action: Action) {
    if (busyRef.current) {
      setToast("Wait for the current document action to finish.");
      return;
    }
    if (modal?.type === "password") {
      setToast("Unlock this PDF or cancel the password dialog first.");
      return;
    }
    if (writingDraftRef.current) {
      setError("Finish or discard the current document draft before leaving this tab.");
      return;
    }
    if (authoring.hasDraft() && action.type === "close") {
      setError("Apply or discard the Page authoring draft before closing this PDF tab.");
      setPanel("authoring"); setPanelOpen(true);
      return;
    }
    if (designer.pending() && action.type === "close") {
      showPanel("form-designer");
      setError("Apply or discard the form-design draft before closing this PDF tab.");
      return;
    }
    if (action.type !== "close") {
      void run("Opening PDF...", () => perform(action));
      return;
    }
    if (forms.pending()) {
      setError(null);
      setModal({ type: "form-discard", action });
      return;
    }
    if (redactionsRef.current.length) {
      setError(null);
      setModal({ type: "redaction-leave", action });
      return;
    }
    if (infoRef.current?.dirty) {
      setModal({ type: "unsaved", action });
    } else {
      void run(action.type === "close" ? "Closing..." : "Opening PDF...", () => perform(action));
    }
  }

  function open() {
    props.onOpen();
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
    if (next === "signatures") setTool("select");
    if (next === "forms") { setTool("select"); if (mode === "write") setMode("read"); }
    if (next === "form-designer") { setTool("select"); setMode("write"); }
    else if (tool === "form-widget") setTool("select");
  }

  function showOcr() {
    if (busyRef.current) return;
    if (!ocrRange.trim()) setOcrRange(String(pageRef.current + 1));
    setTool("select");
    setMode("write");
    showPanel("ocr");
  }

  function startOcr() {
    void run("Recognizing scanned text...", async () => {
      const current = infoRef.current;
      if (!current || !pdf) throw new Error("Wait for the PDF to finish loading before recognizing text.");
      if (hasPendingContentEdits() || redactionsRef.current.length) {
        throw new Error("Apply or discard pending edits and clear redaction marks before using OCR.");
      }
      await syncDraft(current.id, draftProjection());
      if (current.readOnlyReason) throw new Error(current.readOnlyReason);
      if (!current.copyAllowed) throw new Error("This PDF does not allow copying. Reopen with the owner password before using OCR.");
      const pages = parsePageRange(ocrRange, current.pageCount, MAX_OCR_PAGES);
      const controller = new AbortController();
      ocrJob.current = { controller, documentId: current.id, revision: current.revision };
      setOcrOutcome(null);
      setOcrCanceling(false);
      setOcrProgress({ stage: "preparing", page: pages[0], completed: 0, total: pages.length, fraction: 0 });
      let plan: OcrPlan | undefined;
      let outcome: OcrOutcome | undefined;
      let failure: unknown;
      let cleanupFailure: unknown;
      try {
        plan = await bridge.beginOcr(current, { pages, dpi: ocrDpi, language: "eng", existingText: ocrPolicy });
        controller.signal.throwIfAborted();
        await recognizeOcrPages(pdf, plan, bridge.stageOcrPage, (progress) => {
          setOcrProgress((previous) => previous?.stage === progress.stage && previous.page === progress.page
            && previous.completed === progress.completed && Math.floor(previous.fraction * 100) === Math.floor(progress.fraction * 100)
            ? previous : progress);
        }, controller.signal);
        controller.signal.throwIfAborted();
        if (!activeRef.current || infoRef.current?.id !== current.id || infoRef.current.revision !== current.revision
          || writingDraftRef.current || redactionsRef.current.length) {
          throw new Error("The document or pending drafts changed during OCR. No text was added.");
        }
        ocrJob.current = null;
        const total = plan.pages.filter((page) => !page.skipped).length;
        setOcrProgress({ stage: "committing", page: pages[pages.length - 1], completed: total, total, fraction: 1 });
        outcome = await bridge.commitOcr(plan.jobId);
      } catch (error) { failure = error; }
      if (plan) {
        try { await bridge.cancelOcr(plan.jobId); }
        catch (error) { cleanupFailure = error; }
      }
      ocrJob.current = null;
      setOcrProgress(null);
      setOcrCanceling(false);
      if (outcome) {
        if (outcome.document.revision !== current.revision) await adopt(outcome.document);
        setOcrOutcome(outcome);
        setToast(outcome.wordCount ? "Searchable text added. Review the result, then save a copy to keep it."
          : "OCR finished without adding text. See the recognition summary.");
        if (cleanupFailure) setError(`OCR completed, but staging cleanup failed: ${errorMessage(cleanupFailure)}`);
      } else if (cleanupFailure) {
        throw new Error(`No OCR text was committed. Staging cleanup failed: ${errorMessage(cleanupFailure)}`);
      } else if (controller.signal.aborted) {
        setToast("OCR canceled. The working PDF is unchanged.");
      } else if (failure) {
        throw failure;
      }
    });
  }

  function cancelOcr() {
    const job = ocrJob.current;
    if (!job) return;
    setOcrCanceling(true);
    job.controller.abort();
  }

  function chooseTool(next: Tool) {
    if (busyRef.current) { setToast("Wait for the current document action to finish."); return; }
    if (authoring.hasDraft() && !["select", "hand", "image", "crop"].includes(next)) {
      setError("Apply or discard the Page authoring draft before using another editing tool.");
      return;
    }
    if (forms.pending() && !["select", "hand"].includes(next)) {
      showPanel("forms");
      setError("Apply or discard your form changes before editing other PDF content.");
      return;
    }
    if (designer.pending() && !["select", "hand", "form-widget"].includes(next)) {
      showPanel("form-designer");
      setError("Apply or discard the form-design draft before editing other PDF content.");
      return;
    }
    if (pendingMetadata && !["select", "hand"].includes(next)) {
      setError("Apply or discard the metadata draft before editing this PDF.");
      return;
    }
    if (redactionsRef.current.length && !["select", "hand", "redact"].includes(next)) {
      setError("Clear the pending redaction marks before changing the original PDF.");
      return;
    }
    if (!["select", "hand"].includes(next) && readOnly) {
      setError(info?.readOnlyReason ?? "This PDF is read-only.");
      return;
    }
    setTool(next);
    if (mode === "organize") setMode("read");
    if (next === "highlight" || next === "note") setMode("comment");
    if (next === "text" || next === "ink") { setMode("write"); setPanel("write"); setPanelOpen(true); }
    if (next === "edit-text") { setMode("write"); setPanel("edit"); setPanelOpen(true); }
    if (next === "redact") { setMode("write"); setPanel("redact"); setPanelOpen(true); }
    if (next === "image" || next === "crop") authoring.choose(next);
  }

  function changeMode(next: Mode) {
    if (designer.pending() && (next === "write" || next === "organize")) {
      showPanel("form-designer"); setError("Apply or discard the form-design draft before changing editing modes."); return;
    }
    if (authoring.hasDraft() && next !== "read") {
      setError("Apply or discard the Page authoring draft before changing editing modes.");
      return;
    }
    if (forms.pending() && (next === "write" || next === "organize")) {
      showPanel("forms");
      setError("Apply or discard your form changes before writing or organizing pages.");
      return;
    }
    if (redactionsRef.current.length) {
      if (next === "organize") { setError("Clear the redaction marks before reorganizing pages."); return; }
      if (next === "write") { chooseTool("redact"); return; }
    }
    setMode(next);
    setTool("select");
    if (next === "comment") { setPanel("comments"); setPanelOpen(true); }
    if (next === "write") { setTool("edit-text"); setPanel("edit"); setPanelOpen(true); }
  }

  function edit(change: Edit, label: string, onSuccess?: () => void) {
    if (designer.pending()) { showPanel("form-designer"); setError("Apply or discard the form-design draft before changing this PDF."); return; }
    if (authoring.hasDraft()) { setError("Apply or discard the Page authoring draft before changing this PDF."); return; }
    if (forms.pending()) { showPanel("forms"); setError("Apply or discard the pending form changes before editing this PDF."); return; }
    if (redactionsRef.current.length) { setError("Clear pending redaction marks before editing the original PDF."); return; }
    if (pendingMetadata) { setError("Apply or discard the metadata draft before making other edits."); return; }
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
      } else if (change.type === "replace_text") {
        setToast("Existing PDF text replaced. Save a copy to keep your changes.");
      }
    });
  }

  function selectSourceText(run: ExistingTextRun, targetPage = pageRef.current) {
    if (designer.pending()) { showPanel("form-designer"); setError("Apply or discard the form-design draft before editing source text."); return; }
    if (authoring.hasDraft()) { setError("Apply or discard the Page authoring draft before editing source text."); return; }
    if (forms.pending()) { showPanel("forms"); setError("Apply or discard the pending form changes before editing PDF text."); return; }
    if (redactionsRef.current.length) { setError("Clear pending redaction marks before editing the original PDF."); return; }
    const current = infoRef.current;
    if (!current || busyRef.current) return;
    if (current.readOnlyReason) { setError(current.readOnlyReason); return; }
    if (!run.editable || run.text === null) {
      setError(run.reason ?? "This source text cannot be edited safely.");
      return;
    }
    setError(null);
    setModal({ type: "edit-existing", documentId: current.id, revision: current.revision,
      page: targetPage, path: run.path, expected: run.text, text: run.text, originalFont: run.font, font: "original" });
  }

  function hitSourceText(text: string, origin: Point, targetPage: number) {
    const current = infoRef.current;
    if (!current || busyRef.current) return;
    void bridge.textRuns(current, targetPage).then((runs) => {
      if (infoRef.current?.id !== current.id || infoRef.current.revision !== current.revision) {
        throw new Error("The document changed. Select the text again.");
      }
      if (busyRef.current || actionRef.current.hasModal) {
        setToast("Finish the current action, then select the text again.");
        return;
      }
      const match = matchExistingText(runs, text, origin);
      if (match) selectSourceText(match, targetPage);
      else {
        setPanel("edit"); setPanelOpen(true);
        setToast("This text is split or repeated in the PDF. Choose the exact source run from the list.");
      }
    }).catch((error: unknown) => setError(errorMessage(error)));
  }

  function importPages(file?: File) {
    if (designer.pending()) { showPanel("form-designer"); setError("Apply or discard the form-design draft before inserting pages."); return; }
    if (authoring.hasDraft()) { setError("Apply or discard the Page authoring draft before inserting pages."); return; }
    if (forms.pending()) { showPanel("forms"); setError("Apply or discard form changes before inserting pages."); return; }
    if (redactionsRef.current.length) { setError("Clear redaction marks before inserting pages."); return; }
    if (writingDraftRef.current) { setError("Finish or discard the current document draft before inserting pages."); return; }
    if (!native && !file) { importRef.current?.click(); return; }
    void run("Inserting PDF pages...", async () => {
      const current = infoRef.current;
      if (!current) throw new Error("Open or create a PDF before inserting pages.");
      const at = pageRef.current + 1;
      const next = file ? await bridge.importFile(current, file, at) : await bridge.import(current, at);
      if (!next) return;
      await finishOpen(next);
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

  function draw(points: Point[], targetPage = pageRef.current) {
    try {
      edit({ type: "ink", page: targetPage, points, color: rgbFromHex(writingColor),
        width: parseWritingNumber(penWidth, 0.5, 12, "Pen width") }, "Saving drawing...");
    } catch (error) { setError(errorMessage(error)); }
  }

  function highlight(quads: Quad[], text: string, targetPage = pageRef.current) {
    edit({ type: "highlight", page: targetPage, quads, color: [1, 0.86, 0.17], text }, "Adding highlight...");
  }

  function history(redo: boolean) {
    if (typingTarget(document.activeElement)) {
      document.execCommand(redo ? "redo" : "undo");
      return;
    }
    if (designer.pending()) { showPanel("form-designer"); setError("Apply or discard the form-design draft before undoing or redoing PDF changes."); return; }
    if (authoring.hasDraft()) { setError("Apply or discard the Page authoring draft before undoing or redoing PDF changes."); return; }
    if (forms.pending()) { showPanel("forms"); setError("Apply or discard the form changes before undoing or redoing document edits."); return; }
    if (redactionsRef.current.length) { setError("Clear redaction marks before undoing or redoing changes to the original."); return; }
    if (writingDraftRef.current) { setError("Apply or discard the current draft before undoing document changes."); return; }
    const current = infoRef.current;
    if (!current || !(redo ? current.canRedo : current.canUndo)) return;
    void run(redo ? "Redoing change..." : "Undoing change...", async () => {
      await adopt(await (redo ? bridge.redo(current) : bridge.undo(current)));
    });
  }

  function save(copy = true) {
    if (designer.pending()) {
      showPanel("form-designer"); setError("Apply or discard the form-design draft before saving. It is not yet in the PDF."); return;
    }
    if (authoring.hasDraft() || writingDraftRef.current) {
      setError("Apply or discard pending writing and Page authoring drafts before saving.");
      if (authoring.hasDraft()) { setPanel("authoring"); setPanelOpen(true); }
      return;
    }
    if (forms.pending()) {
      showPanel("forms");
      setError("Apply or explicitly discard your form changes before saving. Unapplied values are not yet in the PDF.");
      return;
    }
    if (redactionsRef.current.length || tool === "redact") { reviewRedactions(); return; }
    if (writingDraftRef.current) { setError("Apply or cancel the current writing draft before saving."); return; }
    void run(native ? "Saving PDF..." : "Preparing download...", async () => {
      const current = infoRef.current;
      if (!current) throw new Error("Open a PDF before saving.");
      await syncDraft(current.id, draftProjection());
      const saved = await bridge.save(current, copy);
      if (!saved) return;
      updateInfo(saved);
      setToast(native ? "PDF saved." : "PDF downloaded. Browser preview does not overwrite local files.");
    });
  }

  function reviewSigning(report: SignatureReport) {
    const current = infoRef.current;
    if (!current || busyRef.current) return;
    if (redactionsRef.current.length || hasPendingContentEdits()) {
      setError("Apply or discard pending edits and clear redaction marks before signing the current PDF.");
      return;
    }
    void run("Preparing certificate signing...", async () => {
      await syncDraft(current.id, draftProjection());
      if (draftProjection() || infoRef.current?.id !== report.documentId || infoRef.current.revision !== report.revision) {
        throw new Error("The document or its drafts changed. Refresh signature checks before signing.");
      }
      setModal({ type: "sign-copy", documentId: current.id, revision: current.revision, report });
    });
  }

  function exportSignedCopy(identity: SigningIdentity, password: string, fieldId: string | null) {
    const requested = modal;
    void run("Signing a separate PDF copy...", async () => {
      const current = infoRef.current;
      if (!current || requested?.type !== "sign-copy"
          || current.id !== requested.documentId || current.revision !== requested.revision) {
        throw new Error("The document changed. Close the signing dialog and review the current PDF again.");
      }
      if (redactionsRef.current.length || hasPendingContentEdits()) {
        throw new Error("Apply or discard pending edits and clear redaction marks before signing. The original has not been redacted.");
      }
      await syncDraft(current.id, draftProjection());
      if (draftProjection()) throw new Error("Apply or discard pending edits before signing.");
      const output = await signatureBridge.export(current, identity, password, fieldId);
      if (!output) {
        setToast("Signing canceled. Your original and unsaved work are unchanged.");
        return;
      }
      setModal(null);
      setToast("Signed copy exported. This window still shows the unchanged source; open the exported PDF to inspect all signatures.");
    });
  }

  function applyMetadata() {
    const draft = metadataDraft;
    if (!draft || !metadataChanged(draft)) return;
    void run("Applying descriptive metadata...", async () => {
      if (forms.pending() || designer.pending() || authoring.hasDraft() || pendingWriting) {
        throw new Error("Apply or discard the other pending document edits before applying metadata.");
      }
      const current = infoRef.current;
      if (!current || current.id !== draft.documentId || current.revision !== draft.revision) {
        throw new Error("The PDF changed while this metadata draft was open. Your draft is retained; discard it and review the current properties.");
      }
      if (redactionsRef.current.length) throw new Error("Clear pending redaction marks before editing metadata.");
      await adopt(await bridge.edit(current, { type: "metadata", fields: draft.fields }));
      setMetadataDraft(null);
      setToast("Metadata applied. Undo is available; save a PDF copy to keep it.");
    });
  }

  async function utilityExportSource() {
    const current = infoRef.current;
    if (!current) throw new Error("Open a PDF first.");
    if (pendingDraftRef.current) {
      throw new Error("Finish or discard pending writing, form, metadata, and redaction drafts before exporting. No copy was created.");
    }
    await syncDraft(current.id, draftProjection());
    if (infoRef.current?.id !== current.id || infoRef.current.revision !== current.revision) {
      throw new Error("The document changed. Review it and export again.");
    }
    return current;
  }

  function addAttachment(file?: File) {
    const current = infoRef.current;
    if (!current || busyRef.current) return;
    if (pendingDraftRef.current) { setError("Finish or discard pending drafts and redaction marks before adding attachments."); return; }
    if (!native && !file) {
      attachmentPickerRevision.current = { documentId: current.id, revision: current.revision };
      attachmentRef.current?.click();
      return;
    }
    const selectedAt = attachmentPickerRevision.current;
    attachmentPickerRevision.current = null;
    if (file && (!selectedAt || selectedAt.documentId !== current.id || selectedAt.revision !== current.revision)) {
      setError("The PDF changed while the attachment picker was open. Choose the file again.");
      return;
    }
    void run("Embedding attachment...", async () => {
      const source = await utilityExportSource();
      const next = file ? await documentUtilities.addAttachmentFile(source, file)
        : await documentUtilities.addAttachment(source);
      if (!next) return;
      await adopt(next);
      setToast("File embedded in the working PDF. Save a copy to keep it.");
    });
  }

  function exportAttachment(attachment: AttachmentInfo) {
    void run("Saving attachment...", async () => {
      if (await documentUtilities.exportAttachment(await utilityExportSource(), attachment)) {
        setModal(null);
        setToast("Attachment saved without opening or executing it.");
      }
    });
  }

  function exportProtectionCopy(protection: CopyProtection, authorizationPassword: string | null) {
    void run("Exporting protection copy...", async () => {
      if (await documentUtilities.exportProtectionCopy(await utilityExportSource(), protection, authorizationPassword)) {
        setModal(null);
        setToast("Separate PDF copy exported. This tab, its protection, and its unsaved work are unchanged.");
      }
    });
  }

  function addBookmark(entries: BookmarkInfo[], parent: string | null) {
    const current = infoRef.current;
    if (!current || managementEditDisabled) return;
    setError(null);
    setModal({ type: "bookmark-create", documentId: current.id, revision: current.revision,
      text: "", page: String(pageRef.current + 1), parent, entries,
      at: entries.filter((entry) => entry.parent === parent).length });
  }

  function requestOutput(kind: OutputAction) {
    if (busyRef.current) { setToast("Wait for the current document action to finish."); return; }
    if (redactionsRef.current.length || hasPendingContentEdits()) {
      setError("Apply or discard pending edits and clear redaction marks before printing or exporting.");
      return;
    }
    const current = infoRef.current;
    if (!current) { setError("Open a PDF before printing or exporting."); return; }
    if (kind === "print" && !current.permissions.printAllowed) {
      setError("This PDF does not allow printing. Reopen it with the owner password."); return;
    }
    if (kind === "export" && !current.permissions.copyAllowed) {
      setError("This PDF does not allow copying text or images. Reopen it with the owner password."); return;
    }
    if (kind === "optimize" && current.readOnlyReason) { setError(current.readOnlyReason); return; }
    void run("Preparing output controls...", async () => {
      await syncDraft(current.id, draftProjection());
      setOutputMode(kind);
    });
  }

  function updateRedactions(next: RedactionMark[]) {
    redactionsRef.current = next;
    setRedactions(next);
    setRedactedCopyPrepared(false);
  }

  function markRedaction(rect: PdfRect, targetPage = pageRef.current) {
    if (designer.pending()) { showPanel("form-designer"); setError("Apply or discard the form-design draft before marking redactions."); return; }
    if (authoring.hasDraft()) { setError("Apply or discard the Page authoring draft before marking redactions."); return; }
    if (forms.pending()) { showPanel("forms"); setError("Apply or discard form changes before marking redactions."); return; }
    const current = infoRef.current;
    if (!current || busyRef.current) return;
    if (current.readOnlyReason) { setError(current.readOnlyReason); return; }
    const marks = redactionsRef.current;
    if (marks.length >= MAX_REDACTION_MARKS) { setError("The limit is 500 redaction areas per export."); return; }
    if (marks.some((mark) => mark.documentId !== current.id || mark.revision !== current.revision)) {
      setError("The original changed. Clear the old marks and review this document again.");
      return;
    }
    updateRedactions([...marks, { id: ++redactionCounter.current, documentId: current.id,
      revision: current.revision, page: targetPage, rect }]);
    setError(null);
  }

  function applyForms() {
    if (designer.pending()) { showPanel("form-designer"); setError("Apply or discard the form-design draft before filling fields."); return; }
    if (authoring.hasDraft() || writingDraftRef.current) {
      setError("Apply or discard pending writing and Page authoring before applying form changes.");
      return;
    }
    if (redactionsRef.current.length) { setError("Clear pending redaction marks before applying form changes."); return; }
    void run("Applying PDF form changes...", async () => {
      const current = infoRef.current;
      if (!current) throw new Error("Open a PDF before filling its fields.");
      const changes = forms.changes(current);
      await syncDraft(current.id, true);
      const next = await bridge.fillForms(current, changes);
      forms.discard();
      await adopt(next);
      await syncDraft(next.id, draftProjection());
      setToast("Form values and appearances applied. Undo is available; save a copy to keep them.");
    });
  }

  function hasPendingContentEdits() {
    return writingDraftRef.current || forms.pending() || designer.pending() || authoring.hasDraft() || gesturePages.current.size > 0;
  }

  function draftProjection(exclude?: "formFilling" | "formDesign") {
    return hasDocumentDrafts({
      writing: writingDraftRef.current || gesturePages.current.size > 0,
      formFilling: exclude !== "formFilling" && forms.pending(),
      formDesign: exclude !== "formDesign" && designer.pending(),
      pageAuthoring: authoring.hasDraft(),
      redaction: redactionsRef.current.length > 0,
    });
  }

  async function discardForms(action?: Action) {
    let discarded = false;
    await run("Discarding unapplied form changes...", async () => {
      const current = infoRef.current;
      if (!current) throw new Error("This PDF tab is no longer open.");
      await syncDraft(current.id, draftProjection("formFilling"));
      forms.discard();
      setModal(null);
      discarded = true;
    });
    if (discarded && action) requestAction(action);
  }

  function locateFormWidget(widget: FormWidget) {
    setPage(widget.page);
    setMode("read"); setTool("select"); setSelectedAnnotation(null);
    workspaceRef.current?.querySelector<HTMLElement>(`[data-page-slot="${widget.page + 1}"]`)?.scrollIntoView({ block: "start", inline: "nearest" });
    if (window.matchMedia("(max-width: 899px)").matches) setPanelOpen(false);
  }

  function designerAvailable() {
    if (busyRef.current) return false;
    if (draftProjection("formDesign")) {
      setError("Apply or discard other writing, filling, metadata, authoring and redaction drafts before designing fields.");
      return false;
    }
    return true;
  }

  function createDesignerField(kind: ControlKind) {
    if (!designerAvailable()) return;
    setError(null); setToast(null); setDesignerWidget(0);
    designer.choose(kind, pageRef.current);
  }

  function selectDesignerField(id: string, widgetId?: string) {
    if (!designerAvailable()) return;
    if (designer.pending() && designer.draft?.fieldId !== id) {
      setError("Apply or discard the form-design draft before selecting another field.");
      return;
    }
    designer.select(id);
    const widgets = designer.designer?.fields.find(({ field }) => field.id === id)?.field.widgets;
    setDesignerWidget(widgetId ? Math.max(0, widgets?.findIndex((widget) => widget.id === widgetId) ?? 0) : 0);
    setError(null);
  }

  function placeDesignerWidget(targetPage: number, placement: [number, number, number, number]) {
    if (!designerAvailable()) return;
    const current = infoRef.current, draft = designer.draft;
    if (!current || !draft || draft.documentId !== current.id || draft.revision !== current.revision) {
      setError("The form-design draft is stale. It is kept; discard it before placing a widget in this revision."); return;
    }
    const index = Math.min(designerWidget, draft.widgets.length - 1), previous = draft.widgets[index];
    if (!previous || !placement.every(Number.isFinite)) { setError("Choose a valid widget and dimensions in the form designer."); return; }
    const next = { ...previous, page: String(targetPage + 1), x: decimal(placement[0]), y: decimal(placement[1]),
      width: decimal(placement[2]), height: decimal(placement[3]) };
    designer.change({ ...draft, widgets: draft.widgets.map((widget, at) => at === index ? next : widget) });
    try { widgetPlacement(next, current); setError(null); }
    catch (error) { setError(errorMessage(error)); }
  }

  function applyDesigner() {
    if (!designerAvailable()) return;
    void run("Applying form design...", async () => {
      const current = infoRef.current;
      if (!current) throw new Error("Open a PDF before designing fields.");
      const changes = designer.changes(current);
      await syncDraft(current.id, true);
      const next = await formDesignerBridge.apply(current, changes);
      designer.discard();
      setTool("select"); setDesignerWidget(0);
      await adopt(next);
      await syncDraft(next.id, draftProjection());
      setToast("Form design applied to the PDF. Undo is available; save a copy to keep it.");
    });
  }

  function discardDesigner() {
    void run("Discarding form-design draft...", async () => {
      const current = infoRef.current;
      if (!current) throw new Error("This PDF tab is no longer open.");
      await syncDraft(current.id, draftProjection("formDesign"));
      designer.discard(); setTool("select"); setDesignerWidget(0);
      setToast("Design draft discarded. Already applied PDF edits are unchanged.");
    });
  }

  function reviewStaticFormCopy() {
    if (busyRef.current) return;
    void run("Reviewing static form copy...", async () => {
      const source = await utilityExportSource();
      const plan = await formDesignerBridge.inspectStaticCopy(source);
      if (infoRef.current?.id !== source.id || infoRef.current.revision !== source.revision) {
        throw new Error("The PDF changed. Review the current form before exporting.");
      }
      setModal({ type: "static-form-copy", documentId: source.id, revision: source.revision, plan, acknowledged: false });
    });
  }

  function exportStaticFormCopy() {
    const requested = modal;
    void run("Exporting separate static form copy...", async () => {
      const source = await utilityExportSource();
      if (requested?.type !== "static-form-copy" || !requested.acknowledged
          || source.id !== requested.documentId || source.revision !== requested.revision) {
        throw new Error("The document changed. Close this dialog and review the static copy again.");
      }
      const output = await formDesignerBridge.exportStaticCopy(source);
      if (!output) { setToast("Static-copy export canceled. Your source and unsaved work are unchanged."); return; }
      setModal(null);
      setToast("Static form copy exported. This tab still shows the interactive original; its unsaved work and history are unchanged.");
    });
  }

  function reviewRedactions() {
    if (busyRef.current) { setToast("Wait for the current document action to finish."); return; }
    if (hasPendingContentEdits()) { setError("Apply or discard pending edits before preparing a redacted copy."); return; }
    if (!redactionsRef.current.length) { setToast("Mark at least one area before exporting a redacted copy."); return; }
    setError(null);
    setModal({ type: "redaction-export", dpi: 150, acknowledged: false });
  }

  function exportRedactions(dpi: 150 | 300) {
    if (hasPendingContentEdits()) { setError("Apply or discard pending edits before exporting a redacted copy."); return; }
    void run("Preparing redacted copy...", async () => {
      const current = infoRef.current;
      const marks = redactionsRef.current;
      if (!current || !pdf) throw new Error("Wait for the PDF to finish loading.");
      if (!marks.length || marks.some((mark) => mark.documentId !== current.id || mark.revision !== current.revision)) {
        throw new Error("These redaction marks are stale. Clear them and review the current PDF again.");
      }
      const controller = new AbortController();
      exportController.current = controller;
      setRedactionProgress({ page: 0, count: current.pageCount, stage: "rendering", canceling: false });
      let plan: RedactionPlan | undefined;
      let failure: unknown;
      let cleanupFailure: unknown;
      let output: string | null = null;
      try {
        plan = await bridge.beginRedaction(current, dpi, marks.map(({ page, rect }) => ({ page, rect })));
        await renderRedactionPages(pdf, plan, bridge.stageRedactionPage, (page, count) => {
          setRedactionProgress({ page, count, stage: "rendering", canceling: false });
        }, controller.signal);
        controller.signal.throwIfAborted();
        exportController.current = null;
        setRedactionProgress({ page: current.pageCount, count: current.pageCount, stage: "saving", canceling: false });
        output = await bridge.exportRedactedCopy(plan.jobId);
      } catch (error) { failure = error; }
      if (plan) {
        try { await bridge.cancelRedaction(plan.jobId); }
        catch (error) { cleanupFailure = error; }
      }
      exportController.current = null;
      setRedactionProgress(null);
      if (cleanupFailure) {
        throw new Error(`${failure ? `${errorMessage(failure)} ` : output ? "The copy was exported, but " : ""}Redaction cleanup failed: ${errorMessage(cleanupFailure)}`);
      }
      if (failure) {
        if (!controller.signal.aborted) throw failure;
        setToast("Redaction export canceled. Your original and pending marks are unchanged.");
        return;
      }
      if (!output) { setToast("Export canceled. Your redaction marks remain pending."); return; }
      setRedactedCopyPrepared(true);
      setModal(null);
      setToast(native ? "Redacted copy saved. This window still shows the original."
        : "Redacted copy download started. This window still shows the original.");
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
      case "print": requestOutput("print"); break;
      case "export": requestOutput("export"); break;
      case "optimize": requestOutput("optimize"); break;
      case "undo": history(false); break;
      case "redo": history(true); break;
      case "find": showPanel("search"); break;
      case "close-tab": requestAction({ type: "close" }); break;
      case "about": setModal({ type: "about" }); break;
    }
  }

  actionRef.current = {
    open, save, history, find: () => showPanel("search"),
    action: requestAction, page: changePage, zoom: changeZoom, tool: chooseTool,
    menu: handleMenu, hasModal: Boolean(modal) || Boolean(outputMode) || props.blocked,
    escape: () => { if (!busyRef.current) { setModal(null); setTool("select"); } },
  };

  useEffect(() => {
    if (!props.active || !props.document || loaded.current) return;
    loaded.current = true;
    void run("Opening PDF...", async () => adopt(props.document!));
  }, [props.active, props.document]);

  useEffect(() => () => {
    if (pdf) void pdf.loadingTask.destroy().catch((error: unknown) => console.error("Could not release PDF renderer resources:", error));
  }, [pdf]);

  useEffect(() => {
    const viewport = viewerRef.current;
    if (!viewport) return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry.contentRect.width > 0 && entry.contentRect.height > 0) {
        setViewportSize({ width: entry.contentRect.width, height: entry.contentRect.height });
      }
    });
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [mode, props.active]);

  useEffect(() => setPageInput(String(page + 1)), [page]);

  useEffect(() => {
    if (!info) return;
    let active = true;
    void syncDraft(info.id, pendingDraft).catch((error: unknown) => {
      if (active) setError(`Native draft protection could not be updated: ${errorMessage(error)}`);
    });
    return () => { active = false; };
  }, [info?.id, pendingDraft, syncDraft]);

  useEffect(() => {
    if (props.active && panelOpen && panel === "search") searchRef.current?.focus();
  }, [panel, panelOpen, props.active]);

  useEffect(() => {
    if (!search.results.length) return;
    setActiveResult(0);
    setPage(search.results[0].page);
  }, [search.results]);

  useEffect(() => {
    const selected = workspaceRef.current?.querySelector<HTMLElement>(".page-rail .page-thumb.selected");
    selected?.scrollIntoView({ block: "nearest" });
  }, [page]);

  useEffect(() => {
    if (!toast) return;
    const timeout = window.setTimeout(() => setToast(null), 5000);
    return () => window.clearTimeout(timeout);
  }, [toast]);

  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (!activeRef.current || event.defaultPrevented) return;
      const actions = actionRef.current;
      const key = event.key.toLowerCase();
      const mod = event.metaKey || event.ctrlKey;
      if (mod && key === "p") {
        event.preventDefault();
        if (!actions.hasModal || gesturePages.current.size > 0) actions.menu("print");
        return;
      }
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
        if (["PageDown", "PageUp"].includes(event.key)) {
          event.preventDefault();
          const viewport = viewerRef.current;
          if (viewport) viewport.scrollBy({ top: viewport.clientHeight * 0.9 * (event.key === "PageDown" ? 1 : -1) });
          else actions.page(event.key === "PageDown" ? 1 : -1);
        }
        if (event.key === "ArrowRight") { event.preventDefault(); actions.page(1); }
        if (event.key === "ArrowLeft") { event.preventDefault(); actions.page(-1); }
        if (key === "v") actions.tool("select");
        if (key === "h") actions.tool("hand");
        if (key === "m") actions.tool("highlight");
        if (key === "n") actions.tool("note");
        if (key === "t") actions.tool("text");
        if (key === "d") actions.tool("ink");
        if (key === "e") actions.tool("edit-text");
        if (key === "r") actions.tool("redact");
        if (key === "escape") actions.escape();
      }
    };
    window.addEventListener("keydown", keydown);
    return () => {
      window.removeEventListener("keydown", keydown);
    };
  }, []);

  useEffect(() => {
    let screenWidth = window.innerWidth;
    const collapseForResize = () => {
      // Print page widths must not change the document workspace's panel state.
      if (window.matchMedia("print").matches) return;
      const width = window.innerWidth;
      if (width < 1100 && screenWidth >= 1100) setPanelOpen(false);
      if (width < 700 && screenWidth >= 700) setRailOpen(false);
      screenWidth = width;
    };
    window.addEventListener("resize", collapseForResize);
    return () => window.removeEventListener("resize", collapseForResize);
  }, []);

  const currentMatch = search.results[activeResult];
  const zoomPercent = Math.round(actualScale * 75);
  const customPercent = zoom.mode === "custom" ? String(Math.round(zoom.scale * 75)) : zoom.mode;
  const zoomOptions = ["50", "75", "100", "125", "150", "200", "300", "400"];

  function protectCopy(event: ClipboardEvent<HTMLDivElement>) {
    const formValue = event.target instanceof Element && Boolean(event.target.closest(".form-control"));
    if (info?.copyAllowed === false && (!typingTarget(event.target) || formValue)) {
      event.preventDefault();
      setError("This PDF does not allow copying. Reopen it with the owner password.");
    }
  }

  return <div className="document-workspace" ref={workspaceRef} hidden={!props.active}
    id={`document-panel-${documentId}`} role={info ? "tabpanel" : "region"}
    aria-labelledby={info ? `document-tab-${documentId}` : undefined}
    aria-label={info ? undefined : "PDF workspace"}
    onCopyCapture={protectCopy} onCutCapture={protectCopy}
  >
    <input ref={importRef} className="file-input" type="file" accept=".pdf,application/pdf" aria-label="Choose PDF to insert"
      onChange={(event) => { const file = event.currentTarget.files?.[0]; if (file) importPages(file); event.currentTarget.value = ""; }} />
    <input ref={imageRef} className="file-input" type="file" accept=".png,.jpg,.jpeg,image/png,image/jpeg" aria-label="Choose image to insert"
      onChange={(event) => { const file = event.currentTarget.files?.[0]; if (file) authoring.stage(file); event.currentTarget.value = ""; }} />
    <input ref={attachmentRef} className="file-input" type="file" aria-label="Choose file to embed as an attachment"
      onChange={(event) => { const file = event.currentTarget.files?.[0]; if (file) addAttachment(file); event.currentTarget.value = ""; }} />

    <div className="commandbar">
      <button className={`all-tools ${panelOpen && panel === "tools" ? "selected" : ""}`} onClick={() => {
        if (panel === "tools") setPanelOpen((open) => !open);
        else { setPanel("tools"); setPanelOpen(true); }
      }} aria-expanded={panelOpen && panel === "tools"} aria-label="All tools" title="All tools">
        <PanelLeft size={17} /><span>All tools</span>
      </button>
      <nav className="mode-tabs" aria-label="Document workspace">
        <SelectionIndicator kind="modes" selectionKey={`${props.active}:${mode}`} />
        {([{ mode: "read", label: "Read", icon: BookOpen }, { mode: "write", label: "Write", icon: SquarePen }, { mode: "comment", label: "Comment", icon: MessageSquare },
          { mode: "organize", label: "Organize", icon: LayoutGrid }] as const).map((item) =>
          <button key={item.mode} className={mode === item.mode ? "selected" : ""}
            aria-label={item.label} title={item.label}
            aria-current={mode === item.mode ? "page" : undefined}
            disabled={disabled || (item.mode === "organize" || item.mode === "write") && forms.count > 0 || item.mode === "organize" && redactions.length > 0} onClick={() => changeMode(item.mode)}>
            <item.icon size={16} /><span>{item.label}</span>
          </button>)}
      </nav>
      <div className="command-actions">
        <IconButton label="Find in document (Command+F)" disabled={disabled} onClick={() => showPanel("search")}><Search size={18} /></IconButton>
        <IconButton label="Print PDF (Command+P)" disabled={disabled} onClick={() => requestOutput("print")}><Printer size={18} /></IconButton>
        <span className="toolbar-divider" />
        <IconButton label="Undo (Command+Z)" disabled={disabled || authoring.pending || pendingMetadata || redactions.length > 0 || forms.count > 0 || !info?.canUndo} onClick={() => history(false)}><Undo2 size={18} /></IconButton>
        <IconButton label="Redo (Command+Shift+Z)" disabled={disabled || authoring.pending || pendingMetadata || redactions.length > 0 || forms.count > 0 || !info?.canRedo} onClick={() => history(true)}><Redo2 size={18} /></IconButton>
        <button className="button primary save-button" aria-label={saveLabel} title={saveLabel} disabled={disabled || redactionIntent && !redactions.length} onClick={() => save(true)}>
          {redactionIntent ? <FileOutput size={15} /> : <Copy size={15} />}<span>{saveLabel}</span>
        </button>
      </div>
    </div>

    {info?.readOnlyReason && <div className="readonly-banner" role="status"><LockKeyhole size={15} />{info.readOnlyReason}</div>}
    {info?.passwordProtected && !info.readOnlyReason && <div className="protected-banner" role="status"><LockKeyhole size={15} />Password protected. Ordinary PDF saves keep this document's passwords and permissions.</div>}
    {redactions.length > 0 && <div className="readonly-banner redaction-source-banner" role="status"><Eraser size={15} />
      Original PDF: {redactions.length} pending {redactions.length === 1 ? "redaction" : "redactions"}. Only exported copies are redacted.</div>}
    {forms.count > 0 && <div className="protected-banner form-draft-banner" role="status"><ClipboardList size={15} />
      {forms.count} unapplied form {forms.count === 1 ? "change" : "changes"}. Save requires applying or discarding these values.
      <button className="text-button" onClick={() => showPanel("forms")}>Review fields</button></div>}
    {pendingDesign && <div className="protected-banner designer-draft-banner" role="status"><ClipboardList size={15} />
      Unapplied form-design draft. Save requires applying or discarding it.
      <button className="text-button" onClick={() => showPanel("form-designer")}>Review design</button></div>}
    {ocrProgress && <OcrJobProgress progress={ocrProgress} canceling={ocrCanceling} onCancel={cancelOcr} />}

    <div className="workspace">
      {panelOpen && <aside key={panel} className={`tools-panel panel-${panel}`} aria-label={panel === "tools" ? "PDF tools" : panelLabels[panel]}>
        <div className="panel-heading">
          <h2>{panelLabels[panel]}</h2>
          {panel === "comments" && <span className="count-badge">{info?.annotations.length ?? 0}</span>}
          <IconButton label="Close side panel" onClick={() => setPanelOpen(false)}><PanelLeftClose size={16} /></IconButton>
        </div>
        {utilityPanel && error && !modal && <div className="utility-inline-error" role="alert">
          <span>{error}</span><IconButton label="Dismiss error" onClick={() => setError(null)}><X size={15} /></IconButton>
        </div>}
        {utilityPanel && managementTooLarge && <p className="utility-capacity-notice" role="status">
          Document-management editing requires a current PDF within the 64 MiB undo-snapshot budget. Inspection and separate-copy exports remain available.
        </p>}
        {panel === "tools" && <>
          <div className="tools-list">
            <button className={mode === "read" && tool === "select" ? "chosen" : ""} disabled={disabled} onClick={() => { changeMode("read"); setTool("select"); }}>
              <span className="tool-icon blue"><BookOpen size={19} /></span><span>Read a PDF</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button className={tool === "edit-text" ? "chosen" : ""} disabled={editDisabled} onClick={() => chooseTool("edit-text")}>
              <span className="tool-icon blue"><SquarePen size={19} /></span><span>Edit existing text</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button disabled={disabled} onClick={() => showPanel("forms")}>
              <span className="tool-icon blue"><ClipboardList size={19} /></span><span>Fill form fields</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button disabled={disabled} onClick={() => showPanel("form-designer")}>
              <span className="tool-icon blue"><ClipboardList size={19} /></span><span>Design form fields</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button className={tool === "redact" ? "chosen" : ""} disabled={redactDisabled} onClick={() => chooseTool("redact")}>
              <span className="tool-icon neutral"><Eraser size={19} /></span><span>Redact a PDF</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button disabled={disabled} onClick={() => showPanel("signatures")}>
              <span className="tool-icon teal"><FileKey2 size={19} /></span><span>Digital signatures</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button disabled={disabled} onClick={props.onCompare}>
              <span className="tool-icon blue"><Files size={19} /></span><span>Compare PDFs</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button disabled={editDisabled || info?.copyAllowed === false} onClick={showOcr}>
              <span className="tool-icon blue"><ScanLine size={19} /></span><span>Recognize scanned text</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button className={tool === "highlight" ? "chosen" : ""} disabled={editDisabled} onClick={() => chooseTool("highlight")}>
              <span className="tool-icon amber"><Highlighter size={19} /></span><span>Highlight text</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button className={tool === "note" ? "chosen" : ""} disabled={editDisabled} onClick={() => chooseTool("note")}>
              <span className="tool-icon violet"><MessageSquarePlus size={19} /></span><span>Add a note</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button disabled={editDisabled} onClick={() => chooseTool("text")}><span className="tool-icon blue"><Type size={19} /></span><span>Add text</span><ChevronRight size={14} className="list-arrow" /></button>
            <button disabled={editDisabled} onClick={() => chooseTool("ink")}><span className="tool-icon violet"><Pencil size={19} /></span><span>Draw with pen</span><ChevronRight size={14} className="list-arrow" /></button>
            <button disabled={authoringDisabled} onClick={() => authoring.choose(authoring.draft?.kind ?? "image")}>
              <span className="tool-icon blue"><ImagePlus size={19} /></span><span>Page authoring</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button className={mode === "organize" ? "chosen" : ""} disabled={disabled || redactions.length > 0 || forms.count > 0 || authoring.pending} onClick={() => changeMode("organize")}>
              <span className="tool-icon teal"><LayoutGrid size={19} /></span><span>Organize pages</span><ChevronRight size={14} className="list-arrow" />
            </button>
            <button disabled={disabled} onClick={() => showPanel("properties")}><span className="tool-icon neutral"><Info size={18} /></span><span>Document properties</span><ChevronRight size={14} className="list-arrow" /></button>
            <button disabled={disabled} onClick={() => showPanel("bookmarks")}><span className="tool-icon neutral"><Bookmark size={18} /></span><span>Edit bookmarks</span><ChevronRight size={14} className="list-arrow" /></button>
            <button disabled={disabled} onClick={() => showPanel("attachments")}><span className="tool-icon neutral"><Paperclip size={18} /></span><span>File attachments</span><ChevronRight size={14} className="list-arrow" /></button>
            <button disabled={disabled || pendingDraft || Boolean(info?.readOnlyReason?.match(/signed|certif/i))}
              onClick={() => { setError(null); setModal({ type: "protection-copy" }); }}><span className="tool-icon neutral"><LockKeyhole size={18} /></span><span>Export protection copy</span><ChevronRight size={14} className="list-arrow" /></button>
            <div className="tool-list-divider" />
            <button disabled={disabled} onClick={() => requestOutput("print")}>
              <span className="tool-icon neutral"><Printer size={18} /></span><span>Print PDF</span><kbd>Cmd+P</kbd>
            </button>
            <button disabled={disabled} onClick={() => requestOutput("export")}>
              <span className="tool-icon neutral"><FileDown size={18} /></span><span>Export images or text</span>
            </button>
            <button disabled={disabled || readOnly} onClick={() => requestOutput("optimize")}>
              <span className="tool-icon neutral"><Minimize2 size={18} /></span><span>Optimize PDF copy</span>
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
        {panel === "signatures" && info && <SignaturesPanel info={info} active={props.active}
          disabled={disabled} pendingDraft={pendingDraft} onSign={reviewSigning} />}
        {panel === "authoring" && info && authoring.draft && info.pages[authoring.draft.page] && <AuthoringPanel
          draft={authoring.draft} geometry={info.pages[authoring.draft.page].geometry}
          currentPage={page} pageCount={info.pageCount} disabled={authoringDisabled}
          error={error} notice={toast} onDismissError={() => setError(null)}
          onChange={authoring.change} onKind={authoring.choose} onImage={() => imageRef.current?.click()}
          onApply={authoring.apply} onDiscard={authoring.discard} onDraw={() => {
            setTool(authoring.draft?.kind === "crop" ? "crop" : "image"); setPanelOpen(false);
          }} />}
        {panel === "forms" && <FormsPanel documentId={documentId} page={page} form={forms.form}
          draft={forms.draft} count={forms.count} loading={forms.loading} error={forms.error}
          actionError={error} notice={toast}
          fieldErrors={forms.fieldErrors} stale={forms.stale} retry={forms.retry}
          disabled={disabled || redactions.length > 0 || authoring.pending || pendingWriting || pendingMetadata || pendingDesign} onChange={forms.change} onApply={applyForms}
          onDiscard={() => { setError(null); setModal({ type: "form-discard" }); }} onLocate={locateFormWidget}
          onDesign={() => showPanel("form-designer")} onFlatten={reviewStaticFormCopy} />}
        {panel === "form-designer" && info && <FormDesignerPanel info={info} page={page} designer={designer.designer}
          draft={designer.draft} widgetIndex={designerWidget} loading={designer.loading} stale={designer.stale}
          error={error ?? designer.error} notice={toast}
          disabled={disabled || readOnly || forms.count > 0 || authoring.pending || pendingWriting || pendingMetadata || redactions.length > 0}
          retry={designer.retry} onNew={createDesignerField} onSelect={selectDesignerField} onChange={designer.change}
          onWidget={setDesignerWidget} onTarget={() => {
            if (!designerAvailable()) return;
            setTool("form-widget");
            setToast("Click to move the selected widget, or drag a new box to move and resize it. Apply the design when ready.");
            if (window.matchMedia("(max-width: 899px)").matches) setPanelOpen(false);
          }} onLocate={(page) => setPage(page)} onApply={applyDesigner} onDiscard={discardDesigner}
          onFlatten={reviewStaticFormCopy} onFill={() => showPanel("forms")} />}
        {panel === "bookmarks" && <BookmarksPanel info={info} page={page} disabled={disabled} editDisabled={managementEditDisabled}
          onNavigate={navigateLink} onAdd={addBookmark}
          onRename={(entry) => {
            if (!info) return;
            setError(null); setModal({ type: "bookmark-rename", documentId: info.id, revision: info.revision, id: entry.id, text: entry.title });
          }}
          onRemove={(entry) => { setError(null); setModal({ type: "bookmark-delete", entry }); }}
          onMove={(entry, parent, at) => edit({ type: "bookmark_move", id: entry.id, parent, at }, "Organizing bookmarks...", () => setToast("Bookmark moved. Its destination and children are unchanged."))} />}
        {panel === "properties" && <DocumentPropertiesPanel info={info} draft={metadataDraft}
          disabled={disabled || readOnly || managementTooLarge || redactions.length > 0 || forms.count > 0 || authoring.pending || pendingDesign} onDraft={(draft) => setMetadataDraft(metadataChanged(draft) ? draft : null)}
          onApply={applyMetadata} onDiscard={() => setMetadataDraft(null)} />}
        {panel === "attachments" && <AttachmentsPanel info={info} editDisabled={managementEditDisabled} exportDisabled={disabled || pendingDraft}
          onAdd={() => addAttachment()}
          onExport={(attachment) => { setError(null); setModal({ type: "attachment-export", attachment }); }}
          onRemove={(attachment) => { setError(null); setModal({ type: "attachment-remove", attachment }); }} />}
        {panel === "edit" && <ExistingTextPanel key={`${info?.id}:${info?.revision}:${page}`}
          runs={existingText.runs} loading={existingText.loading} error={existingText.error} retry={existingText.retry}
          page={page} disabled={editDisabled} onSelect={selectSourceText} />}
        {panel === "redact" && <RedactionPanel pdf={pdf} page={page} marks={redactions} disabled={redactDisabled}
          exported={redactedCopyPrepared} onMark={markRedaction} onPage={setPage}
          onRemove={(id) => updateRedactions(redactionsRef.current.filter((mark) => mark.id !== id))}
          onClear={() => { setError(null); setModal({ type: "redaction-clear" }); }} onExport={reviewRedactions}
          onDraw={() => setPanelOpen(false)} />}
        {panel === "ocr" && <OcrPanel range={ocrRange} dpi={ocrDpi} policy={ocrPolicy}
          pageCount={info?.pageCount ?? 0} disabled={disabled || !pdf}
          blockedReason={info?.readOnlyReason ?? (info?.copyAllowed === false ? "This PDF does not allow copying. Reopen it with the owner password."
            : pendingDraft ? "Finish or cancel writing/form drafts and clear pending redaction marks before using OCR." : null)}
          outcome={ocrOutcome} onRange={setOcrRange} onDpi={setOcrDpi} onPolicy={setOcrPolicy} onStart={startOcr} />}
        {panel === "write" && <div className="panel-scroll writing-settings">
          <p>Add text annotations or freehand marks. To replace original page text, choose Edit existing text.</p>
          <button className="button secondary existing-text-launch" disabled={editDisabled} onClick={() => chooseTool("edit-text")}><SquarePen size={15} />Edit existing text</button>
          <button className="button secondary existing-text-launch" disabled={authoringDisabled}
            onClick={() => authoring.choose(authoring.draft?.kind ?? "image")}><ImagePlus size={15} />Page authoring</button>
          <button className="button secondary existing-text-launch" disabled={disabled} onClick={() => showPanel("form-designer")}><ClipboardList size={15} />Design form fields</button>
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
                <p>{search.noText ? "Recognize scanned pages offline to add searchable text." : "Try a different word, or turn off Match case."}</p>
                {search.noText && <button className="button secondary" disabled={editDisabled || info?.copyAllowed === false} onClick={showOcr}>Recognize scanned text</button>}</div>}
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
                  {annotation.kind === "highlight" ? <Highlighter size={14} /> : annotation.kind === "text" ? <Type size={14} /> : annotation.kind === "ink" ? <Pencil size={14} />
                    : annotation.kind === "image" ? <ImagePlus size={14} /> : annotation.kind === "watermark" ? <Stamp size={14} /> : annotation.kind === "page_number" ? <Hash size={14} /> : <MessageSquare size={14} />}</span>
                  <strong>{annotation.author}</strong>
                  {annotation.editable && <IconButton label="Edit annotation text" disabled={editDisabled} onClick={() => setModal({ type: "edit-annotation", id: annotation.id, text: annotation.text })}><SquarePen size={13} /></IconButton>}
                  <IconButton label="Remove annotation" disabled={editDisabled || !annotation.deletable}
                    onClick={() => edit({ type: "remove_annotation", id: annotation.id }, "Removing annotation...")}><Trash2 size={13} /></IconButton>
                </div>
                <button className="comment-content" onClick={() => selectAnnotation(annotation.id)}>
                  <span className="comment-kind">{{ highlight: "Highlighted text", note: "Note", text: "Placed text", ink: "Pen drawing",
                    image: "Inserted image", watermark: "Text watermark", page_number: "Page number" }[annotation.kind]}</span>
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
            <button className="button secondary" disabled={authoringDisabled} onClick={() => authoring.choose("crop")}><Crop size={15} />Crop pages</button>
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
                  {props.active && <Thumbnail pdf={pdf} page={index} width={150} />}
                  <span className="organize-page-label"><GripVertical size={14} />Page {index + 1}{page === index ? <span className="selection-check"><Check size={11} /></span> : <span />}</span>
                </button>
              </div>)}
          </div>
          {readOnly && <p className="organize-readonly">This PDF is read-only and cannot be reorganized.</p>}
        </> : <>
          <div className={`page-scroll ${tool === "hand" ? "pannable" : ""}`} ref={viewerRef} tabIndex={0} aria-label="Continuous PDF page scroll area"
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
            {info ? props.active && <ContinuousPages pdf={pdf} documentId={info.id} page={page} zoom={zoom}
              initialPosition={readingPosition.current} onPosition={(position) => { readingPosition.current = position; }}
              width={viewportSize.width} height={viewportSize.height} container={viewerRef}
              onPage={setPage} onScale={onScale} onError={notifyError}
              renderPage={(index, frame) => pdf && <PdfPage pdf={pdf} page={index} initialSize={frame} zoom={zoom} width={viewportSize.width} height={viewportSize.height} copyAllowed={info.copyAllowed}
                tool={tool} disabled={tool === "image" || tool === "crop" ? authoringDisabled : tool === "redact" ? redactDisabled : editDisabled} annotations={info.annotations.filter((annotation) => annotation.page === index)}
                authoringDraft={authoring.draft} onAuthoringPoint={(point) => authoring.place(index, point)}
                onAuthoringCrop={(margins) => authoring.crop(index, margins)}
                formDesigner={panelOpen && panel === "form-designer" || tool === "form-widget" ? {
                  fields: designer.designer?.fields ?? [], draft: designer.draft, widgetIndex: designerWidget,
                  active: tool === "form-widget",
                  disabled: disabled || readOnly || forms.count > 0 || authoring.pending || pendingWriting || pendingMetadata || redactions.length > 0,
                  onSelect: selectDesignerField, onPlace: placeDesignerWidget,
                  onGesture: (pending) => onGestureDraft(index, pending),
                } : undefined}
                redactions={redactions.filter((mark) => mark.page === index)} onRedaction={(rect) => markRedaction(rect, index)}
                selectedAnnotation={selectedAnnotation}
                query={panel === "search" && panelOpen ? query : ""} matchCase={matchCase}
                activeMatch={currentMatch?.page === index ? currentMatch.index : -1}
                onScale={() => {}} onHighlight={(quads, text) => highlight(quads, text, index)}
                onNote={(point) => setModal({ type: "note", page: index, point, text: "" })}
                onText={(rect) => setModal({ type: "text", page: index, rect, text: "" })}
                onInk={(points) => draw(points, index)} onLink={navigateLink}
                onGestureDraft={(pending) => onGestureDraft(index, pending)}
                onEditExisting={(text, origin) => hitSourceText(text, origin, index)}
                penColor={writingColor} penWidth={Number.isFinite(Number(penWidth)) ? Math.max(0.5, Math.min(12, Number(penWidth))) : 2}
                onAnnotation={selectAnnotation} onError={notifyError}
              />} /> : <div className="page-stage"><div className="workspace-empty">
                {busy ? <><LoaderCircle size={30} className="spin" /><h2>{busy}</h2><p>Your document stays on this device.</p></>
                  : <><img src="/folio.svg" width="58" height="58" alt="" /><h1>A little less paperwork.</h1><p>Open a PDF to read, review, and make it your own.</p>
                    <button className="button primary" onClick={open}><FolderOpen size={16} />Open PDF</button>
                    <button className="text-button" onClick={() => requestAction({ type: "example" })}>Open the example document</button></>}
              </div></div>}
          </div>
          {info && <div className="quick-tools" role="toolbar" aria-label="Review tools">
            <SelectionIndicator kind="tools" selectionKey={`${props.active}:${tool}`} />
            {quickTools.map((item, index) => <span key={item.tool}>
              {index === 2 && <span className="quick-tool-divider" />}
              <IconButton label={item.label} active={tool === item.tool}
                disabled={disabled || readOnly && !["select", "hand"].includes(item.tool)
                  || redactions.length > 0 && !["select", "hand", "redact"].includes(item.tool)
                  || forms.count > 0 && !["select", "hand"].includes(item.tool)}
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
          {tool === "edit-text" && !disabled && <div className="tool-instruction" role="status"><SquarePen size={14} />Click existing text to replace it.</div>}
        </>}
        {busy && info && <div className="document-busy" role="status"><LoaderCircle size={14} className="spin" />{busy}</div>}
      </main>

      {props.active && railOpen && info && pdf && mode !== "organize" && <aside className="page-rail" aria-label="Page thumbnails">
        <div className="page-rail-heading"><h2>Pages</h2><span>{info.pageCount}</span><IconButton label="Hide page thumbnails" onClick={() => setRailOpen(false)}><PanelRightClose size={15} /></IconButton></div>
        <div className="thumbnail-list">
          {info.pages.map((item, index) =>
            <button className={`page-thumb ${page === index ? "selected" : ""}`} key={item.key}
              aria-label={`Go to page ${index + 1}`} aria-current={page === index ? "page" : undefined}
              disabled={Boolean(busy)} onClick={() => setPage(index)}>
              <Thumbnail pdf={pdf} page={index} /><span className="thumbnail-number">{index + 1}</span>
            </button>)}
        </div>
        <button className="rail-organize" disabled={disabled || redactions.length > 0 || forms.count > 0} onClick={() => changeMode("organize")}><LayoutGrid size={14} />Organize pages</button>
      </aside>}
      <nav className="utility-rail" aria-label="Document panels">
        <IconButton label="Toggle page thumbnails" active={railOpen && mode !== "organize"} disabled={disabled} onClick={() => { setRailOpen((open) => !open); if (mode === "organize") setMode("read"); }}><FileText size={18} /></IconButton>
        <IconButton label="Show comments" active={panelOpen && panel === "comments"} disabled={disabled} onClick={() => showPanel("comments")}><MessageSquare size={18} />{Boolean(info?.annotations.length) && <span className="utility-dot" />}</IconButton>
        <IconButton label="Show bookmarks" active={panelOpen && panel === "bookmarks"} disabled={disabled} onClick={() => showPanel("bookmarks")}><Bookmark size={18} /></IconButton>
        <IconButton label="Show digital signatures" active={panelOpen && panel === "signatures"} disabled={disabled} onClick={() => showPanel("signatures")}><FileKey2 size={18} /></IconButton>
        <IconButton label="Show form fields" active={panelOpen && panel === "forms"} disabled={disabled} onClick={() => showPanel("forms")}><ClipboardList size={18} /></IconButton>
        <IconButton label="Show file attachments" active={panelOpen && panel === "attachments"} disabled={disabled} onClick={() => showPanel("attachments")}><Paperclip size={18} /></IconButton>
        <IconButton label="Show document properties" active={panelOpen && panel === "properties"} disabled={disabled} onClick={() => showPanel("properties")}><Info size={18} /></IconButton>
        <IconButton label="Search document" active={panelOpen && panel === "search"} disabled={disabled} onClick={() => showPanel("search")}><Search size={18} /></IconButton>
        <span className="utility-spacer" />
        <IconButton label="Rotate current page clockwise" disabled={editDisabled} onClick={() => edit({ type: "rotate", page, degrees: 90 }, "Rotating page...")}><RotateCw size={18} /></IconButton>
      </nav>
    </div>

    <footer className="statusbar">
      <div>{busy ? <><LoaderCircle size={12} className="spin" />{busy}</> : redactions.length ? <><span className="status-dot dirty" />{redactions.length} pending {redactions.length === 1 ? "redaction" : "redactions"}</> : forms.count ? <><span className="status-dot dirty" />{forms.count} unapplied form {forms.count === 1 ? "field" : "fields"}</> : authoring.pending ? <><span className="status-dot dirty" />Unapplied page authoring</> : pendingMetadata ? <><span className="status-dot dirty" />Unapplied metadata draft</> : pendingWriting || gestureDraft ? <><span className="status-dot dirty" />Uncommitted writing or area</> : info?.dirty ? <><span className="status-dot dirty" />Unsaved changes</> : <><Check size={12} className="status-check" />No unsaved changes</>}</div>
      <div className="document-status">{info && <><span>{info.pageCount} {info.pageCount === 1 ? "page" : "pages"}</span><span className="status-separator" /><span>{formatBytes(info.byteLength)}</span></>}</div>
      <div className="privacy-status"><ShieldCheck size={12} /><span>{runtimeIssue ? "Desktop runtime unavailable" : native ? "Local files. No uploads." : "Preview uses the real Rust engine"}</span></div>
    </footer>

    {error && !modal && !outputMode && !utilityPanel && !(panelOpen && (panel === "forms" || panel === "authoring" && authoring.draft)) && <div className="notification error-notification" role="alert"><span><strong>That didn&apos;t work.</strong>{error}</span><IconButton label="Dismiss error" onClick={() => setError(null)}><X size={16} /></IconButton></div>}
    {toast && !(panelOpen && (panel === "forms" || panel === "authoring" && authoring.draft)) && <div className="notification toast" role="status"><Check size={16} /><span>{toast}</span><IconButton label="Dismiss notification" onClick={() => setToast(null)}><X size={14} /></IconButton></div>}

    {modal?.type === "protection-copy" && info && <ProtectionCopyDialog info={info} busy={Boolean(busy)} error={error}
      onClose={() => setModal(null)} onSubmit={exportProtectionCopy} />}

    {modal?.type === "bookmark-create" && <Dialog title="Add a bookmark" error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Cancel</button>
        <button className="button primary" disabled={Boolean(busy) || !modal.text.trim()} onClick={() => {
          const current = infoRef.current;
          if (!current || current.id !== modal.documentId || current.revision !== modal.revision) {
            setError("The PDF changed. Your draft is retained; close this dialog and add the bookmark again."); return;
          }
          const target = pageFromInput(modal.page, current.pageCount);
          if (target === null) { setError("Enter a page number within this PDF."); return; }
          edit({ type: "bookmark_create", title: modal.text, page: target, parent: modal.parent, at: modal.at },
            "Adding bookmark...", () => { setModal(null); setToast("Bookmark added. Save a PDF copy to keep it."); });
        }}><Bookmark size={15} />Add bookmark</button></>}>
      <label className="utility-field">Bookmark title<input data-autofocus type="text" maxLength={4096} value={modal.text} disabled={Boolean(busy)} onChange={(event) => setModal({ ...modal, text: event.target.value })} /></label>
      <div className="bookmark-create-fields">
        <label className="utility-field">Parent<select value={modal.parent ?? ""} disabled={Boolean(busy)} onChange={(event) => {
          const parent = event.target.value || null;
          setModal({ ...modal, parent, at: modal.entries.filter((entry) => entry.parent === parent).length });
        }}><option value="">Document root</option>{modal.entries.map((entry) => <option key={entry.id} value={entry.id}>{"— ".repeat(Math.min(entry.depth, 4))}{entry.title}</option>)}</select></label>
        <label className="utility-field">Page<input type="text" inputMode="numeric" value={modal.page} disabled={Boolean(busy)} onChange={(event) => setModal({ ...modal, page: event.target.value })} /></label>
      </div>
      <p>The bookmark is appended to this group and points to a real PDF page. Use the bookmark controls to move or nest it afterward.</p>
    </Dialog>}

    {modal?.type === "bookmark-rename" && <Dialog title="Rename bookmark" error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Cancel</button>
        <button className="button primary" disabled={Boolean(busy) || !modal.text.trim()} onClick={() => {
          const current = infoRef.current;
          if (!current || current.id !== modal.documentId || current.revision !== modal.revision) {
            setError("The PDF changed. Close this dialog and select the bookmark again."); return;
          }
          edit({ type: "bookmark_rename", id: modal.id, title: modal.text }, "Renaming bookmark...", () => setModal(null));
        }}>Rename</button></>}>
      <label className="utility-field">Bookmark title<input data-autofocus type="text" maxLength={4096} value={modal.text} disabled={Boolean(busy)} onChange={(event) => setModal({ ...modal, text: event.target.value })} /></label>
      <p>The existing destination, action, hierarchy, and styling are kept.</p>
    </Dialog>}

    {modal?.type === "bookmark-delete" && <Dialog title="Delete this bookmark?" error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Keep bookmark</button>
        <button className="button danger" disabled={Boolean(busy)} onClick={() => edit({ type: "bookmark_delete", id: modal.entry.id }, "Deleting bookmark...", () => setModal(null))}>Delete {modal.entry.childCount ? "group" : "bookmark"}</button></>}>
      <p><strong>{modal.entry.title}</strong>{modal.entry.childCount ? " and all its descendant bookmarks will be removed from the outline." : " will be removed from the outline."}</p>
      <p>PDF pages and content are unchanged. Undo restores the entire bookmark group.</p>
    </Dialog>}

    {modal?.type === "attachment-remove" && <Dialog title="Remove this attachment?" error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Keep attachment</button>
        <button className="button danger" disabled={Boolean(busy)} onClick={() => edit({ type: "attachment_remove", id: modal.attachment.id }, "Removing attachment...", () => setModal(null))}>Remove from list</button></>}>
      <p><strong>{modal.attachment.name}</strong> will be removed from the document attachment list. Undo is available.</p>
      <p>This is <strong>not secure erasure</strong>. Unreferenced payloads, undo snapshots, page attachment annotations, or other associated-file references can remain. The source file is unchanged.</p>
    </Dialog>}

    {modal?.type === "attachment-export" && <Dialog title="Save an untrusted attachment?" error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Cancel</button>
        <button className="button primary" disabled={Boolean(busy)} onClick={() => exportAttachment(modal.attachment)}><FileOutput size={15} />Save attachment</button></>}>
      <p><strong>{modal.attachment.name}</strong> is data supplied by this PDF, not a trusted Folio resource.</p>
      <p>Choose a separate output location. Folio sanitizes the suggested filename and never opens or executes the file. Inspect it with trusted tools before using it.</p>
    </Dialog>}

    <OutputDialog info={info} mode={outputMode} busy={Boolean(busy)} pendingDraft={pendingDraft} error={error}
      current={() => infoRef.current} run={run} onClearError={() => setError(null)} onResourceError={notifyError}
      onNotice={setToast} onClose={() => { setOutputMode(null); setError(null); }} />

    {modal?.type === "password" && <PasswordDialog challenge={modal.challenge} busy={Boolean(busy)} error={error}
      onCancel={() => cancelPassword(modal.challenge.requestId)}
      onSubmit={(password) => {
        const requestId = modal.challenge.requestId;
        void run("Unlocking PDF...", async () => finishOpen(await bridge.unlock(requestId, password)));
      }} />}

    {modal?.type === "sign-copy" && info && <SigningDialog info={info} report={modal.report} busy={Boolean(busy)} error={error}
      onClose={() => setModal(null)} onExport={exportSignedCopy} />}
    {modal?.type === "static-form-copy" && <Dialog title="Export a separate static form copy?" error={error} busy={Boolean(busy)}
      onClose={() => setModal(null)} actions={<>
        <button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Keep interactive original</button>
        <button className="button primary" disabled={Boolean(busy) || !modal.acknowledged} onClick={exportStaticFormCopy}><FileOutput size={15} />Export static copy</button>
      </>}>
      <p><strong>{modal.plan.fieldCount} fields / {modal.plan.widgetCount} widgets</strong> will become persistent page content in a separate PDF.
        Interactive controls and their field actions are removed from the copy.</p>
      {modal.plan.unsignedSignatureCount > 0 && <p>{modal.plan.unsignedSignatureCount} empty signature slot(s) will become unsigned static outlines, not digital signatures.</p>}
      {modal.plan.warnings.map((warning) => <p key={warning}>{warning}</p>)}
      <label className="checkbox-setting"><input type="checkbox" checked={modal.acknowledged} disabled={Boolean(busy)}
        onChange={(event) => setModal({ ...modal, acknowledged: event.target.checked })} />
        I understand this is a static copy, not secure sanitization, and will inspect it before sharing.</label>
    </Dialog>}
    {modal?.type === "form-discard" && <Dialog title="Discard unapplied form changes?" error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Keep filling</button>
        <button className="button danger" disabled={Boolean(busy)} onClick={() => void discardForms(modal.action)}>
          Discard form changes{modal.action ? " & continue" : ""}</button></>}>
      <p>{forms.count} changed {forms.count === 1 ? "field has" : "fields have"} not been applied to the PDF.
        Discarding removes these drafts, not already applied document edits.</p>
      <p>Keep filling, then choose Apply form changes and save a copy to preserve the new values.</p>
    </Dialog>}

    {modal?.type === "redaction-export" && <Dialog title="Export a permanently redacted copy?" error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={redactionProgress ? <button className="button secondary"
        disabled={redactionProgress.stage === "saving" || redactionProgress.canceling}
        onClick={() => {
          if (!exportController.current) return;
          exportController.current.abort();
          setRedactionProgress((progress) => progress && { ...progress, canceling: true });
        }}>{redactionProgress.canceling ? "Canceling..." : redactionProgress.stage === "saving" ? "Finishing export..." : "Cancel export"}</button>
        : <><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Keep reviewing</button>
          <button className="button primary" disabled={Boolean(busy) || !modal.acknowledged}
            onClick={() => exportRedactions(modal.dpi)}><FileOutput size={15} />Export redacted copy</button></>}>
      <p><strong>{redactions.length} marked {redactions.length === 1 ? "area" : "areas"}</strong> will become opaque black pixels.
        All <strong>{info?.pageCount} pages</strong> become images, without the original text layer, comments, forms, bookmarks, attachments, or metadata.</p>
      <p>The copy loses search, selection, accessibility structure, and vector quality.
        Check every occurrence and inspect the exported PDF before sharing.</p>
      {info?.passwordProtected && <p className="muted">This copy keeps the PDF's passwords and permissions.
        Older encryption also requires retaining its file identifier; encryption parameters are not removed.</p>}
      <label className="setting-field">Image resolution<select data-autofocus value={modal.dpi} disabled={Boolean(busy)}
        onChange={(event) => {
          const value = Number(event.target.value);
          if (value === 150 || value === 300) setModal({ ...modal, dpi: value });
        }}><option value={150}>150 DPI · smaller</option><option value={300}>300 DPI · sharper</option></select></label>
      <label className="checkbox-setting"><input type="checkbox" checked={modal.acknowledged} disabled={Boolean(busy)}
        onChange={(event) => setModal({ ...modal, acknowledged: event.target.checked })} />
        I understand the copy is image-only and will inspect it before sharing.</label>
      <p className="muted">Your original and unsaved work stay unchanged. This window still shows the original, not the exported copy.</p>
      {redactionProgress && <div className="redaction-progress" role="status" aria-live="polite">
        <div className="redaction-progress-label"><LoaderCircle size={15} className="spin" />
          {redactionProgress.canceling ? "Canceling; your marks will be kept..."
            : redactionProgress.stage === "saving" ? native ? "Choose a filename in the Save dialog..." : "Preparing the download..."
              : `Rendering page ${Math.max(1, redactionProgress.page)} of ${redactionProgress.count}...`}</div>
        <progress max={redactionProgress.count} value={redactionProgress.page} aria-label="Redaction export progress" />
      </div>}
    </Dialog>}

    {modal?.type === "redaction-clear" && <Dialog title="Clear pending redaction marks?" onClose={() => setModal(null)}
      actions={<><button className="button secondary" onClick={() => setModal(null)}>Keep marks</button>
        <button className="button danger" onClick={() => {
          updateRedactions([]); redactionCounter.current = 0; setModal(null);
        }}>Clear marks</button></>}>
      <p>This removes the pending marks, not the original content. Ordinary save/download will export the <strong>unredacted original</strong> again.</p>
      <p>A previously exported redacted PDF is a separate file and is not changed.</p>
    </Dialog>}

    {modal?.type === "redaction-leave" && <Dialog title="Leave your pending redactions?" onClose={() => setModal(null)}
      actions={<><button className="button secondary" onClick={() => setModal(null)}>Keep reviewing</button>
        <button className="button danger" onClick={() => {
          const action = modal.action;
          updateRedactions([]); redactionCounter.current = 0; setModal(null); requestAction(action);
        }}>Discard marks & continue</button></>}>
      <p>You have {redactions.length} pending redaction {redactions.length === 1 ? "area" : "areas"}. These marks are not saved inside your original PDF.</p>
      <p>Keep reviewing to export a redacted copy first, or explicitly discard the marks.
        Unsaved changes to the original document will be checked separately.</p>
    </Dialog>}

    {modal?.type === "edit-existing" && <Dialog title={`Edit existing text · Page ${modal.page + 1}`} error={error} busy={Boolean(busy)} onClose={() => setModal(null)}
      actions={<><button className="button secondary" disabled={Boolean(busy)} onClick={() => setModal(null)}>Cancel</button>
        <button className="button primary" disabled={Boolean(busy) || modal.text === modal.expected && modal.font === "original"} onClick={() => {
          const current = infoRef.current;
          if (!current || current.id !== modal.documentId || current.revision !== modal.revision) {
            setError("The document changed. Close this editor and select the text again.");
            return;
          }
          edit({ type: "replace_text", page: modal.page, path: modal.path, expected: modal.expected,
            replacement: modal.text, font: modal.font }, "Replacing PDF text...", () => setModal(null));
        }}><SquarePen size={15} />Replace text</button></>}>
      <label htmlFor="existing-text-value" className="note-label">Replacement text</label>
      <textarea id="existing-text-value" rows={4} maxLength={4000} value={modal.text} disabled={Boolean(busy)}
        onChange={(event) => setModal({ ...modal, text: event.target.value })} />
      <label className="setting-field">Font<select value={modal.font} disabled={Boolean(busy)}
        onChange={(event) => {
          const font = event.target.value;
          if (font === "original" || font === "noto") setModal({ ...modal, font });
        }}>
        <option value="original">{modal.originalFont.replace(/^[A-Z]{6}\+/, "")} (original)</option>
        <option value="noto">Noto Sans (embedded)</option>
      </select></label>
      <p className="muted">This replaces the selected drawing operation in the PDF, not an annotation.
        Font size, position, and color are retained. Longer text does not reflow neighboring lines.</p>
      <p className="muted">{modal.font === "noto"
        ? "Noto Sans embeds new Latin, Greek, or Cyrillic glyphs and changes this run's typeface."
        : "The original font preserves its typeface but may not include every replacement character."}</p>
      <p className="muted">Clear the field to remove this run. This is not secure redaction.</p>
    </Dialog>}

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
      <p>You have unsaved changes in <strong>{info?.name}</strong>. Save an edited copy before closing this tab. Other PDFs stay open.</p>
      {!native && <p className="muted">Browser preview downloads a copy instead of writing to your filesystem.</p>}
    </Dialog>}

    {modal?.type === "shortcuts" && <Dialog title="A few useful shortcuts" onClose={() => setModal(null)}>
      <dl className="shortcut-list">
        {[["New PDF", "Command + N"], ["Open PDFs", "Command + O"], ["Close PDF tab", "Command + W"], ["Next / previous tab", "Control + Tab / Shift + Tab"], ["Save", "Command + S"], ["Save a copy", "Command + Shift + S"],
          ["Find in document", "Command + F"], ["Print PDF", "Command + P"], ["Undo / redo", "Command + Z / Shift + Z"],
          ["Previous / next page", "Left / Right"], ["Zoom in / out", "Command + Plus / Minus"],
          ["Select / pan", "V / H"], ["Highlight / add note", "M / N"], ["Add text / pen", "T / D"], ["Edit existing text", "E"], ["Mark redaction areas", "R"]].map(([label, key]) =>
          <div key={label}><dt>{label}</dt><dd><kbd>{key}</kbd></dd></div>)}
      </dl><p className="muted">On Windows and Linux, use Control in place of Command.</p>
    </Dialog>}

    {modal?.type === "about" && <Dialog title="Meet Folio." onClose={() => setModal(null)}>
      <div className="about-brand"><img src="/folio.svg" width="52" height="52" alt="" /><div><h3>A little less paperwork.</h3><p>Version {appVersion} - macOS-first MVP</p></div></div>
      <p>An independently built PDF workspace. Tauri connects this interface to a Rust document engine; PDF.js renders the pages.</p>
      <p>Create, read, search, replace supported existing text runs, add text or drawings, redact to sanitized image-only copies, review, combine, and extract PDF pages. Open password-protected PDFs and retain their protection when saving. Inspect supported CMS signatures, explicitly evaluate chains against local custom CA anchors, and export incremental signed/co-signed approval copies with a local PKCS#12 identity. Revocation and trusted time are not checked. Paragraph reflow, interactive forms, OCR, Office conversion, and setting or changing passwords are not included.</p>
      <p className="muted">Not affiliated with Adobe. No code, fonts, icons, or artwork have been extracted from Acrobat. Open-source dependencies carry their own licenses. Files stay on this device. This is an MVP, not a full Acrobat replacement.</p>
      {!native && <div className="preview-explanation"><FileText size={17} /><p><strong>You are in browser preview.</strong>The same Rust engine processes your edits through a loopback-only development bridge. Use the desktop app for native open/save dialogs.</p></div>}
    </Dialog>}
  </div>;
}
