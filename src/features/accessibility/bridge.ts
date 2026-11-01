import { invoke } from "@tauri-apps/api/core";
import { native, post, previewJson, previewResponse } from "../../lib/bridge";
import type { SemanticCopyPlan, SemanticCopyRequest, SemanticInspection, SourceRef } from "./types";

export const accessibilityBridge = {
  inspect(source: SourceRef, signal: AbortSignal): Promise<SemanticInspection> {
    signal.throwIfAborted();
    // Retain the returned ID after cancellation so the hook can release server state.
    return native ? invoke("inspect_accessibility", { source })
      : previewJson("/accessibility/inspect", post(source));
  },
  checkInspection(inspectionId: number): Promise<SemanticInspection> {
    return native ? invoke("check_accessibility_inspection", { inspectionId })
      : previewJson("/accessibility/inspection/check", post({ inspectionId }));
  },
  cancelInspection(inspectionId: number): Promise<boolean> {
    return native ? invoke("cancel_accessibility_inspection", { inspectionId })
      : previewJson("/accessibility/inspection/cancel", post({ inspectionId }));
  },
  begin(request: SemanticCopyRequest, signal: AbortSignal): Promise<SemanticCopyPlan> {
    signal.throwIfAborted();
    return native ? invoke("begin_accessibility_copy", { request })
      : previewJson("/accessibility/begin", post(request));
  },
  check(jobId: number): Promise<SemanticCopyPlan> {
    return native ? invoke("check_accessibility_copy", { jobId })
      : previewJson("/accessibility/check", post({ jobId }));
  },
  cancel(jobId: number): Promise<boolean> {
    return native ? invoke("cancel_accessibility_copy", { jobId })
      : previewJson("/accessibility/cancel", post({ jobId }));
  },
  async save(plan: SemanticCopyPlan, signal: AbortSignal, check: () => void): Promise<string | null> {
    signal.throwIfAborted(); check();
    if (native) return invoke("save_accessibility_copy", { jobId: plan.jobId });
    const response = await previewResponse("/accessibility/file", { ...post({ jobId: plan.jobId }), signal });
    const blob = await response.blob();
    if (blob.size > 128 * 1024 * 1024) throw new Error("The tagged copy exceeds 128 MiB.");
    signal.throwIfAborted(); check();
    await accessibilityBridge.check(plan.jobId);
    signal.throwIfAborted(); check();
    const url = URL.createObjectURL(blob), link = document.createElement("a");
    try {
      link.href = url; link.download = plan.suggestedName;
      document.body.append(link); link.click();
      await new Promise((resolve) => window.setTimeout(resolve, 1000));
    } finally { link.remove(); URL.revokeObjectURL(url); }
    return plan.suggestedName;
  },
};
