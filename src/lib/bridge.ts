import { invoke, isTauri } from "@tauri-apps/api/core";
import { getIdentifier } from "@tauri-apps/api/app";
import type { DocumentInfo, Edit } from "./types";
import { resolveRuntime } from "./runtime";

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

async function previewResponse(route: string, options: RequestInit = {}): Promise<Response> {
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

async function previewJson<T>(route: string, options: RequestInit = {}): Promise<T> {
  const response = await previewResponse(route, options);
  return response.json() as Promise<T>;
}

function revision(info: DocumentInfo) {
  return { documentId: info.id, revision: info.revision };
}

function post(value: object): RequestInit {
  return { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(value) };
}

export const bridge = {
  bootstrap(): Promise<DocumentInfo> {
    if (runtimeIssue) return Promise.reject(new Error(runtimeIssue));
    return native ? invoke("bootstrap") : previewJson("/bootstrap");
  },
  open(discardChanges: boolean): Promise<DocumentInfo | null> {
    if (!native) throw new Error("Use the browser file picker in browser preview.");
    return invoke("open_document", { discardChanges });
  },
  openPath(path: string, discardChanges: boolean): Promise<DocumentInfo> {
    if (!native) throw new Error("File paths can only be opened in the desktop application.");
    return invoke("open_document_path", { path, discardChanges });
  },
  async openFile(file: File, discardChanges: boolean): Promise<DocumentInfo> {
    if (native) throw new Error("Use the native file picker in the desktop application.");
    if (file.size > 128 * 1024 * 1024) throw new Error("This PDF is larger than the 128 MB MVP limit.");
    return previewJson("/open", {
      method: "POST",
      headers: {
        "Content-Type": "application/pdf",
        "X-File-Name": encodeURIComponent(file.name),
        "X-Discard-Changes": String(discardChanges),
      },
      body: file,
    });
  },
  example(discardChanges: boolean): Promise<DocumentInfo> {
    return native
      ? invoke("open_example", { discardChanges })
      : previewJson("/demo", { method: "POST", headers: { "X-Discard-Changes": String(discardChanges) } });
  },
  async bytes(info: DocumentInfo): Promise<Uint8Array> {
    if (native) {
      const buffer = await invoke<ArrayBuffer>("document_bytes", revision(info));
      return new Uint8Array(buffer);
    }
    const response = await previewResponse("/bytes", {
      headers: { "X-Document-Id": String(info.id), "X-Revision": String(info.revision) },
    });
    return new Uint8Array(await response.arrayBuffer());
  },
  edit(info: DocumentInfo, edit: Edit): Promise<DocumentInfo> {
    return native
      ? invoke("edit_document", { ...revision(info), edit })
      : previewJson("/edit", post({ ...revision(info), edit }));
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
    const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "application/pdf" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = info.name.replace(/\.pdf$/i, "") + "-edited.pdf";
    document.body.append(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 30_000);
    return info;
  },
  close(): Promise<void> {
    if (!native) throw new Error("Close the browser tab to leave the browser preview.");
    return invoke("close_application");
  },
};
