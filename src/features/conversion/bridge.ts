import { invoke } from "@tauri-apps/api/core";
import { native, post, previewJson } from "../../lib/bridge";
import type { DocumentInfo } from "../../lib/types";
import type { CreationKind, CreationPlan, CreationSettings, CreationStatus } from "./model";

export const conversionBridge = {
  begin(kind: CreationKind): Promise<CreationPlan> {
    return native ? invoke("begin_creation", { kind }) : previewJson("/creation/begin", post({ kind }));
  },
  pick(jobId: number): Promise<CreationPlan | null> {
    if (!native) throw new Error("Use the browser file picker for preview creation.");
    return invoke("choose_creation_files", { jobId });
  },
  stage(jobId: number, file: File, signal: AbortSignal): Promise<CreationPlan> {
    if (native) throw new Error("Native creation accepts explicit local file-picker selections only.");
    return previewJson("/creation/file", {
      method: "POST", signal,
      headers: { "X-Folio-Job-Id": String(jobId), "X-File-Name": encodeURIComponent(file.name), "Content-Type": "application/octet-stream" },
      body: file,
    });
  },
  prepare(jobId: number, settings: CreationSettings, order: number[]): Promise<CreationStatus> {
    const input = { jobId, settings, order };
    return native ? invoke("prepare_creation", input) : previewJson("/creation/prepare", post(input));
  },
  status(jobId: number): Promise<CreationStatus> {
    return native ? invoke("creation_status", { jobId }) : previewJson("/creation/status", post({ jobId }));
  },
  commit(jobId: number): Promise<DocumentInfo> {
    return native ? invoke("commit_creation", { jobId }) : previewJson("/creation/commit", post({ jobId }));
  },
  cancel(jobId: number): Promise<boolean> {
    return native ? invoke("cancel_creation", { jobId }) : previewJson("/creation/cancel", post({ jobId }));
  },
};
