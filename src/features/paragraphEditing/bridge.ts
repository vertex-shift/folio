import { invoke } from "@tauri-apps/api/core";
import { native, post, previewJson, revision } from "../../lib/bridge";
import type { DocumentInfo } from "../../lib/types";
import type { ParagraphChange, ParagraphPage, ParagraphPreview } from "./types";

export const paragraphBridge = {
  inspect(info: DocumentInfo, page: number): Promise<ParagraphPage> {
    const input = { ...revision(info), page };
    return native ? invoke("page_paragraph_blocks", input) : previewJson("/paragraphs", post(input));
  },
  preview(info: DocumentInfo, change: ParagraphChange): Promise<ParagraphPreview> {
    const input = { ...revision(info), change };
    return native ? invoke("preview_paragraph", input) : previewJson("/paragraphs/preview", post(input));
  },
  apply(info: DocumentInfo, change: ParagraphChange): Promise<DocumentInfo> {
    const input = { ...revision(info), change };
    return native ? invoke("edit_paragraph", input) : previewJson("/paragraphs/edit", post(input));
  },
};
