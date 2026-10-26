import { invoke } from "@tauri-apps/api/core";
import { native, post, previewJson, revision } from "../../lib/bridge";
import { MAX_IMAGE_BYTES } from "../../lib/authoring";
import type { ImageAsset } from "../../lib/authoring";
import type { DocumentInfo } from "../../lib/types";
import type { ImageChange, ImagePage } from "./types";

export const imageEditingBridge = {
  inspect(info: DocumentInfo, page: number): Promise<ImagePage> {
    const input = { ...revision(info), page };
    return native ? invoke("page_image_occurrences", input) : previewJson("/images", post(input));
  },
  async stage(info: DocumentInfo, file: File): Promise<ImageAsset> {
    if (!file.size || file.size > MAX_IMAGE_BYTES) throw new Error("Choose a PNG or JPEG no larger than 16 MiB.");
    const bytes = new Uint8Array(await file.arrayBuffer());
    const headers = { "X-Document-Id": String(info.id), "X-Revision": String(info.revision) };
    return native ? invoke("stage_image_replacement", bytes, { headers })
      : previewJson("/images/replacement", { method: "POST", headers: { ...headers, "Content-Type": "application/octet-stream" }, body: bytes });
  },
  apply(info: DocumentInfo, change: ImageChange): Promise<DocumentInfo> {
    const input = { ...revision(info), change };
    return native ? invoke("edit_image_occurrence", input) : previewJson("/images/edit", post(input));
  },
  cancel(documentId: number, assetId: number): Promise<boolean> {
    const input = { documentId, assetId };
    return native ? invoke("cancel_image_replacement", input) : previewJson("/images/replacement/cancel", post(input));
  },
};
