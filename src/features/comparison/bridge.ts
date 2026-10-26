import { invoke } from "@tauri-apps/api/core";
import { native, post, previewJson, previewResponse } from "../../lib/bridge";
import { LIMITS, encodeResults } from "./model";
import type { ComparisonPlan, ComparisonSettings, PageResult, SourceRef } from "./model";

export const comparisonBridge = {
  begin(before: SourceRef, after: SourceRef, settings: ComparisonSettings): Promise<ComparisonPlan> {
    const request = { before, after, settings };
    return native ? invoke("begin_comparison", { request }) : previewJson("/comparison/begin", post(request));
  },
  check(comparisonId: number): Promise<ComparisonPlan> {
    return native ? invoke("check_comparison", { comparisonId }) : previewJson("/comparison/check", post({ comparisonId }));
  },
  async bytes(comparisonId: number, side: "before" | "after", signal: AbortSignal): Promise<Uint8Array> {
    signal.throwIfAborted();
    const buffer = native
      ? await invoke<ArrayBuffer>("comparison_render_bytes", { comparisonId, side })
      : await (await previewResponse("/comparison/render-bytes", { ...post({ comparisonId, side }), signal })).arrayBuffer();
    signal.throwIfAborted();
    if (buffer.byteLength > LIMITS.sourceBytes) throw new Error("The comparison snapshot exceeds 64 MiB.");
    return new Uint8Array(buffer);
  },
  async finish(comparisonId: number, pages: PageResult[], signal: AbortSignal): Promise<void> {
    const bytes = encodeResults(pages);
    const headers = { "X-Folio-Comparison-Id": String(comparisonId) };
    signal.throwIfAborted();
    if (native) return invoke("finish_comparison", bytes, { headers });
    await previewJson("/comparison/finish", { method: "POST", headers, body: new Uint8Array(bytes), signal });
  },
  async save(plan: ComparisonPlan, allowUnencrypted: boolean, signal: AbortSignal, check: () => void): Promise<string | null> {
    signal.throwIfAborted();
    check();
    if (native) return invoke("save_comparison_report", { comparisonId: plan.comparisonId, allowUnencrypted });
    const response = await previewResponse("/comparison/report", { ...post({ comparisonId: plan.comparisonId, allowUnencrypted }), signal });
    const blob = await response.blob();
    signal.throwIfAborted();
    check();
    if (blob.size > LIMITS.reportBytes) throw new Error("The comparison report exceeds 4 MiB.");
    await comparisonBridge.check(plan.comparisonId);
    signal.throwIfAborted();
    check();
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    try {
      link.href = url; link.download = plan.suggestedName;
      document.body.append(link);
      link.click();
      await new Promise((resolve) => window.setTimeout(resolve, 1000));
    } finally {
      link.remove();
      URL.revokeObjectURL(url);
    }
    return plan.suggestedName;
  },
  cancel(comparisonId: number): Promise<boolean> {
    return native ? invoke("cancel_comparison", { comparisonId }) : previewJson("/comparison/cancel", post({ comparisonId }));
  },
};
