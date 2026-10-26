import { invoke, isTauri } from "@tauri-apps/api/core";
import { native, post, previewJson, previewResponse } from "./bridge";
import type { OptimizationReport, OutputPlan, OutputRequest } from "./output";
import { MAX_OUTPUT_BYTES } from "./output";
import { requestBrowserPrint } from "./printSurface";

export const outputBridge = {
  begin(request: OutputRequest): Promise<OutputPlan> {
    return native ? invoke("begin_document_output", { request })
      : previewJson("/output/begin", post(request));
  },
  async renderBytes(jobId: number, signal: AbortSignal): Promise<Uint8Array> {
    signal.throwIfAborted();
    if (native) {
      const bytes = await invoke<ArrayBuffer>("output_render_bytes", { jobId });
      signal.throwIfAborted();
      return new Uint8Array(bytes);
    }
    const response = await previewResponse("/output/render-bytes", { ...post({ jobId }), signal });
    return new Uint8Array(await response.arrayBuffer());
  },
  async stage(jobId: number, index: number, bytes: Uint8Array, signal?: AbortSignal): Promise<void> {
    signal?.throwIfAborted();
    const headers = { "X-Folio-Job-Id": String(jobId), "X-Folio-Output-Index": String(index) };
    if (native) await invoke("stage_document_output", new Uint8Array(bytes), { headers });
    else await previewJson("/output/page", { method: "POST", headers, body: new Uint8Array(bytes), signal });
    signal?.throwIfAborted();
  },
  optimize(jobId: number): Promise<OptimizationReport> {
    return native ? invoke("prepare_optimized_output", { jobId })
      : previewJson("/output/optimize", post({ jobId }));
  },
  check(jobId: number): Promise<OutputPlan> {
    return native ? invoke("check_document_output", { jobId })
      : previewJson("/output/check", post({ jobId }));
  },
  async save(plan: OutputPlan, signal: AbortSignal): Promise<string | null> {
    signal.throwIfAborted();
    if (native) return invoke("save_document_output", { jobId: plan.jobId });
    const response = await previewResponse("/output/file", { ...post({ jobId: plan.jobId }), signal });
    const blob = await response.blob();
    signal.throwIfAborted();
    if (blob.size > MAX_OUTPUT_BYTES) throw new Error("The output exceeds the 128 MiB file limit.");
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    try {
      link.href = url;
      link.download = plan.suggestedName;
      document.body.append(link);
      link.click();
    } finally {
      link.remove();
      window.setTimeout(() => URL.revokeObjectURL(url), 30_000);
    }
    return plan.suggestedName;
  },
  async print(jobId: number): Promise<void> {
    if (native) return invoke("print_document_output", { jobId });
    await outputBridge.check(jobId);
    if (isTauri()) {
      throw new Error("This embedded preview does not provide Folio's native print operation. Open the loopback preview URL in a desktop browser to print.");
    }
    await requestBrowserPrint();
  },
  cancel(jobId: number): Promise<boolean> {
    return native ? invoke("cancel_document_output", { jobId })
      : previewJson("/output/cancel", post({ jobId }));
  },
};
