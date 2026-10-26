import { invoke } from "@tauri-apps/api/core";
import { downloadPdf, native, post, previewJson, previewResponse, revision } from "../../lib/bridge";
import type { DocumentInfo } from "../../lib/types";
import type { DesignChange, DesignerInfo, FlattenInfo } from "./types";

export const formDesignerBridge = {
  inspect(info: DocumentInfo): Promise<DesignerInfo> {
    return native ? invoke("form_designer", revision(info)) : previewJson("/forms/designer", post(revision(info)));
  },
  apply(info: DocumentInfo, changes: DesignChange[]): Promise<DocumentInfo> {
    const input = { ...revision(info), changes };
    return native ? invoke("design_form_fields", input) : previewJson("/forms/design", post(input));
  },
  inspectStaticCopy(info: DocumentInfo): Promise<FlattenInfo> {
    return native ? invoke("inspect_flattened_form_copy", revision(info))
      : previewJson("/forms/flatten/inspect", post(revision(info)));
  },
  async exportStaticCopy(info: DocumentInfo): Promise<string | null> {
    if (native) return invoke("export_flattened_form_copy", revision(info));
    const response = await previewResponse("/forms/flatten", post(revision(info)));
    const name = info.name.replace(/\.pdf$/i, "") + "-static.pdf";
    downloadPdf(new Uint8Array(await response.arrayBuffer()), name);
    return name;
  },
};
