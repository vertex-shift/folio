import { invoke } from "@tauri-apps/api/core";
import { native, post, previewJson, previewResponse } from "../../lib/bridge";
import { ARCHIVE_LIMITS } from "./model";
import type { ArchivePlan, ArchiveRequest, SourceRef, TechnicalReport } from "./model";

async function download(blob: Blob, name: string, signal: AbortSignal, check: () => void) {
  signal.throwIfAborted();
  check();
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  try {
    link.href = url; link.download = name;
    document.body.append(link);
    link.click();
    await new Promise((resolve) => window.setTimeout(resolve, 1000));
  } finally { link.remove(); URL.revokeObjectURL(url); }
}

export const preflightBridge = {
  inspect(source: SourceRef): Promise<TechnicalReport> {
    return native ? invoke("inspect_technical", { source }) : previewJson("/preflight/inspect", post(source));
  },
  check(inspectionId: number): Promise<TechnicalReport> {
    return native ? invoke("check_inspection", { inspectionId }) : previewJson("/preflight/check", post({ inspectionId }));
  },
  cancel(inspectionId: number): Promise<boolean> {
    return native ? invoke("cancel_inspection", { inspectionId }) : previewJson("/preflight/cancel", post({ inspectionId }));
  },
  async save(report: TechnicalReport, allowUnencrypted: boolean, signal: AbortSignal, check: () => void): Promise<string | null> {
    signal.throwIfAborted(); check();
    if (native) return invoke("save_inspection_report", { inspectionId: report.inspectionId, allowUnencrypted });
    const response = await previewResponse("/preflight/report", { ...post({ inspectionId: report.inspectionId, allowUnencrypted }), signal });
    const blob = await response.blob();
    if (blob.size > ARCHIVE_LIMITS.reportBytes) throw new Error("The technical report exceeds 2 MiB.");
    signal.throwIfAborted(); check();
    await preflightBridge.check(report.inspectionId);
    await download(blob, report.suggestedName, signal, check);
    return report.suggestedName;
  },
};

export const archiveBridge = {
  begin(request: ArchiveRequest): Promise<ArchivePlan> {
    return native ? invoke("begin_archive", { request }) : previewJson("/archive/begin", post(request));
  },
  check(archiveId: number): Promise<ArchivePlan> {
    return native ? invoke("check_archive", { archiveId }) : previewJson("/archive/check", post({ archiveId }));
  },
  cancel(archiveId: number): Promise<boolean> {
    return native ? invoke("cancel_archive", { archiveId }) : previewJson("/archive/cancel", post({ archiveId }));
  },
  async bytes(archiveId: number, signal: AbortSignal): Promise<Uint8Array> {
    signal.throwIfAborted();
    const buffer = native ? await invoke<ArrayBuffer>("archive_render_bytes", { archiveId })
      : await (await previewResponse("/archive/render-bytes", { ...post({ archiveId }), signal })).arrayBuffer();
    signal.throwIfAborted();
    if (buffer.byteLength > ARCHIVE_LIMITS.outputBytes) throw new Error("The renderer-only archival snapshot exceeds 128 MiB.");
    return new Uint8Array(buffer);
  },
  async stage(archiveId: number, index: number, bytes: Uint8Array, signal: AbortSignal): Promise<void> {
    if (bytes.byteLength > ARCHIVE_LIMITS.pngBytes) throw new Error("The archival PNG exceeds 64 MiB.");
    const headers = { "X-Folio-Archive-Id": String(archiveId), "X-Folio-Archive-Index": String(index) };
    signal.throwIfAborted();
    if (native) await invoke("stage_archive_page", bytes, { headers });
    else await previewJson("/archive/page", { method: "POST", headers, body: new Uint8Array(bytes), signal });
    signal.throwIfAborted();
  },
  async save(plan: ArchivePlan, signal: AbortSignal, check: () => void): Promise<string | null> {
    signal.throwIfAborted(); check();
    if (native) return invoke("save_archive_copy", { archiveId: plan.archiveId });
    const response = await previewResponse("/archive/file", { ...post({ archiveId: plan.archiveId }), signal });
    const blob = await response.blob();
    if (blob.size > ARCHIVE_LIMITS.outputBytes) throw new Error("The archival copy exceeds 128 MiB.");
    signal.throwIfAborted(); check();
    await archiveBridge.check(plan.archiveId);
    await download(blob, plan.suggestedName, signal, check);
    return plan.suggestedName;
  },
};
