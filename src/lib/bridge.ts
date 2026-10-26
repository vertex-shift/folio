import { invoke, isTauri } from "@tauri-apps/api/core";
import { getIdentifier } from "@tauri-apps/api/app";
import type { DocumentInfo, Edit, ExistingTextRun, OpenOutcome, RedactionPlan, RedactionRegion, WorkspaceInfo } from "./types";
import { openUrl } from "@tauri-apps/plugin-opener";
import { safeExternalUrl } from "./navigation";
import { resolveRuntime } from "./runtime";
import { MAX_IMAGE_BYTES } from "./authoring";
import type { ImageAsset, ImagePlacement } from "./authoring";
import type { FormChange, FormInfo } from "./formsTypes";
import type { OcrOptions, OcrOutcome, OcrPageResult, OcrPlan } from "./ocrTypes";

const runtime = await resolveRuntime({
  hasNativeBridge: isTauri(),
  identify: getIdentifier,
  developmentPreview: import.meta.env.DEV
    && ["http:", "https:"].includes(location.protocol)
    && ["localhost", "127.0.0.1"].includes(location.hostname),
  reportPreview: (message) => console.info(message),
});
export const native = runtime.native;
export const runtimeIssue = runtime.issue;

export async function previewResponse(route: string, options: RequestInit = {}): Promise<Response> {
  if (runtimeIssue) throw new Error(runtimeIssue);
  if (!import.meta.env.DEV) {
    throw new Error("Native document operations require the Folio desktop application.");
  }
  let response: Response;
  try {
    response = await fetch(`/__folio${route}`, {
      ...options,
      headers: { "X-Folio-Preview": "1", ...options.headers },
    });
  } catch (error) {
    throw new Error(
      "The local Rust preview engine is not running. Start npm run preview:backend.",
      { cause: error },
    );
  }
  if (!response.ok) {
    const text = await response.text();
    let details: unknown;
    try {
      details = JSON.parse(text);
    } catch {
      throw new Error(`The local Rust preview engine returned HTTP ${response.status}. Start npm run preview:backend.`);
    }
    if (typeof details === "object" && details !== null && "error" in details && typeof details.error === "string") {
      throw new Error(details.error);
    }
    throw new Error(`The local preview request failed (HTTP ${response.status}).`);
  }
  return response;
}

export async function previewJson<T>(route: string, options: RequestInit = {}): Promise<T> {
  const response = await previewResponse(route, options);
  return response.json() as Promise<T>;
}

export function revision(info: DocumentInfo) {
  return { documentId: info.id, revision: info.revision };
}

export function post(value: object): RequestInit {
  return { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(value) };
}

export function downloadPdf(bytes: Uint8Array, name: string) {
  const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "application/pdf" }));
  const link = document.createElement("a");
  link.href = url; link.download = name;
  document.body.append(link); link.click(); link.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 30_000);
}

export const bridge = {
  bootstrap(): Promise<WorkspaceInfo> {
    if (runtimeIssue) return Promise.reject(new Error(runtimeIssue));
    return native ? invoke("bootstrap") : previewJson("/bootstrap");
  },
  open(): Promise<OpenOutcome[]> {
    if (!native) throw new Error("Use the browser file picker in browser preview.");
    return invoke("open_document");
  },
  openPaths(paths: string[]): Promise<OpenOutcome[]> {
    if (!native) throw new Error("File paths can only be opened in the desktop application.");
    return invoke("open_document_paths", { paths });
  },
  async openFile(file: File): Promise<OpenOutcome> {
    if (native) throw new Error("Use the native file picker in the desktop application.");
    if (file.size > 128 * 1024 * 1024) throw new Error("This PDF is larger than the 128 MB MVP limit.");
    return previewJson("/open", {
      method: "POST",
      headers: {
        "Content-Type": "application/pdf",
        "X-File-Name": encodeURIComponent(file.name),
      },
      body: file,
    });
  },
  unlock(requestId: number, password: string): Promise<OpenOutcome> {
    return native ? invoke("unlock_document", { requestId, password })
      : previewJson("/unlock", post({ requestId, password }));
  },
  cancelOpen(requestId: number): Promise<boolean> {
    return native ? invoke("cancel_open", { requestId })
      : previewJson("/open/cancel", post({ requestId }));
  },
  activate(documentId: number): Promise<DocumentInfo> {
    return native ? invoke("activate_document", { documentId })
      : previewJson("/activate", post({ documentId }));
  },
  closeDocument(info: DocumentInfo, discardChanges: boolean): Promise<WorkspaceInfo> {
    return native ? invoke("close_document", { ...revision(info), discardChanges })
      : previewJson("/close", post({ ...revision(info), discardChanges }));
  },
  example(): Promise<DocumentInfo> {
    return native
      ? invoke("open_example")
      : previewJson("/demo", { method: "POST" });
  },
  create(width: number, height: number): Promise<DocumentInfo> {
    return native ? invoke("create_document", { width, height })
      : previewJson("/create", post({ width, height }));
  },
  import(info: DocumentInfo, at: number): Promise<OpenOutcome | null> {
    if (!native) throw new Error("Use the preview's PDF import file picker.");
    return invoke("import_document_pages", { ...revision(info), at });
  },
  importFile(info: DocumentInfo, file: File, at: number): Promise<OpenOutcome> {
    if (file.size > 128 * 1024 * 1024) throw new Error("The imported PDF exceeds the 128 MiB limit.");
    return previewJson("/import", { method: "POST", headers: {
      "X-Document-Id": String(info.id), "X-Revision": String(info.revision), "X-Insert-At": String(at),
      "X-File-Name": encodeURIComponent(file.name),
      "Content-Type": "application/pdf",
    }, body: file });
  },
  async extract(info: DocumentInfo, pages: number[]): Promise<boolean> {
    if (native) return (await invoke<string | null>("extract_document_pages", { ...revision(info), pages })) !== null;
    const response = await previewResponse("/extract", post({ ...revision(info), pages }));
    downloadPdf(new Uint8Array(await response.arrayBuffer()), info.name.replace(/\.pdf$/i, "") + "-pages.pdf");
    return true;
  },
  async bytes(info: DocumentInfo, render = false): Promise<Uint8Array> {
    if (native) {
      const buffer = await invoke<ArrayBuffer>("document_bytes", { ...revision(info), render });
      return new Uint8Array(buffer);
    }
    const response = await previewResponse(render ? "/render-bytes" : "/bytes", {
      headers: { "X-Document-Id": String(info.id), "X-Revision": String(info.revision) },
    });
    return new Uint8Array(await response.arrayBuffer());
  },
  edit(info: DocumentInfo, edit: Edit): Promise<DocumentInfo> {
    return native
      ? invoke("edit_document", { ...revision(info), edit })
      : previewJson("/edit", post({ ...revision(info), edit }));
  },
  textRuns(info: DocumentInfo, page: number): Promise<ExistingTextRun[]> {
    return native ? invoke("page_text_runs", { ...revision(info), page })
      : previewJson("/text-runs", post({ ...revision(info), page }));
  },
  async stageImage(info: DocumentInfo, file: File): Promise<ImageAsset> {
    if (!file.size || file.size > MAX_IMAGE_BYTES) throw new Error("Choose a PNG or JPEG no larger than 16 MiB.");
    const bytes = new Uint8Array(await file.arrayBuffer());
    const headers = { "X-Document-Id": String(info.id), "X-Revision": String(info.revision) };
    return native ? invoke("stage_authoring_image", bytes, { headers })
      : previewJson("/authoring/image", { method: "POST", headers: { ...headers, "Content-Type": "application/octet-stream" }, body: bytes });
  },
  insertImage(info: DocumentInfo, assetId: number, placement: ImagePlacement): Promise<DocumentInfo> {
    const input = { ...revision(info), assetId, placement };
    return native ? invoke("insert_authoring_image", input) : previewJson("/authoring/image/insert", post(input));
  },
  cancelImage(documentId: number, assetId: number): Promise<boolean> {
    return native ? invoke("cancel_authoring_image", { documentId, assetId })
      : previewJson("/authoring/image/cancel", post({ documentId, assetId }));
  },
  forms(info: DocumentInfo): Promise<FormInfo> {
    return native ? invoke("form_fields", revision(info)) : previewJson("/forms", post(revision(info)));
  },
  fillForms(info: DocumentInfo, changes: FormChange[]): Promise<DocumentInfo> {
    const input = { ...revision(info), changes };
    return native ? invoke("fill_form_fields", input) : previewJson("/forms/fill", post(input));
  },
  beginOcr(info: DocumentInfo, options: OcrOptions): Promise<OcrPlan> {
    const input = { ...revision(info), options };
    return native ? invoke("begin_ocr", input) : previewJson("/ocr/begin", post(input));
  },
  stageOcrPage(jobId: number, result: OcrPageResult, signal?: AbortSignal): Promise<void> {
    return native ? invoke("stage_ocr_page", { jobId, result }) : previewJson("/ocr/page", { ...post({ jobId, result }), signal });
  },
  commitOcr(jobId: number): Promise<OcrOutcome> {
    return native ? invoke("commit_ocr", { jobId }) : previewJson("/ocr/commit", post({ jobId }));
  },
  cancelOcr(jobId: number): Promise<boolean> {
    return native ? invoke("cancel_ocr", { jobId }) : previewJson("/ocr/cancel", post({ jobId }));
  },
  beginRedaction(info: DocumentInfo, dpi: 150 | 300, regions: RedactionRegion[]): Promise<RedactionPlan> {
    const input = { ...revision(info), dpi, regions };
    return native ? invoke("begin_redaction", input) : previewJson("/redaction/begin", post(input));
  },
  async stageRedactionPage(jobId: number, page: number, png: Uint8Array): Promise<void> {
    const headers = { "X-Folio-Job-Id": String(jobId), "X-Folio-Page": String(page) };
    if (native) return invoke("stage_redaction_page", png, { headers });
    await previewJson("/redaction/page", {
      method: "POST", headers: { ...headers, "Content-Type": "image/png" }, body: new Uint8Array(png),
    });
  },
  async exportRedactedCopy(jobId: number): Promise<string | null> {
    if (native) return invoke("export_redacted_copy", { jobId });
    const response = await previewResponse("/redaction/export", post({ jobId }));
    downloadPdf(new Uint8Array(await response.arrayBuffer()), "Redacted.pdf");
    return "Redacted.pdf";
  },
  cancelRedaction(jobId: number): Promise<boolean> {
    return native ? invoke("cancel_redaction", { jobId }) : previewJson("/redaction/cancel", post({ jobId }));
  },
  undo(info: DocumentInfo): Promise<DocumentInfo> {
    return native ? invoke("undo_document", revision(info)) : previewJson("/undo", post(revision(info)));
  },
  redo(info: DocumentInfo): Promise<DocumentInfo> {
    return native ? invoke("redo_document", revision(info)) : previewJson("/redo", post(revision(info)));
  },
  async save(info: DocumentInfo, saveAs = true): Promise<DocumentInfo | null> {
    if (native) return invoke("save_document", { ...revision(info), saveAs });
    const bytes = await bridge.bytes(info);
    downloadPdf(bytes, info.name.replace(/\.pdf$/i, "") + "-edited.pdf");
    return info;
  },
  async external(url: string): Promise<void> {
    const address = safeExternalUrl(url);
    if (native) return openUrl(address);
    const opened = window.open(address, "_blank");
    if (!opened) throw new Error("The browser blocked this link. Allow pop-ups for the local preview and try again.");
    opened.opener = null;
  },
  draft(documentId: number, pending: boolean): Promise<void> {
    return native ? invoke("set_document_draft", { documentId, pending })
      : previewJson("/draft", post({ documentId, pending }));
  },
  close(discardChanges: boolean): Promise<void> {
    if (!native) throw new Error("Close the browser tab to leave the browser preview.");
    return invoke("close_application", { discardChanges });
  },
};
