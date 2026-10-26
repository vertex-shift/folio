import { invoke } from "@tauri-apps/api/core";
import { native, post, previewJson, previewResponse, revision } from "./bridge";
import type { DocumentInfo } from "./types";
import type { AttachmentInfo, BookmarkInfo, CopyProtection, MetadataInfo } from "./documentUtilitiesModel";
import { suggestedAttachmentName, validateCopyProtection } from "./documentUtilitiesModel";

function download(bytes: ArrayBuffer, name: string, type: string) {
  const url = URL.createObjectURL(new Blob([bytes], { type }));
  const link = document.createElement("a");
  link.href = url;
  link.download = suggestedAttachmentName(name);
  document.body.append(link);
  link.click();
  link.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 30_000);
}

export const documentUtilities = {
  metadata(info: DocumentInfo): Promise<MetadataInfo> {
    return native ? invoke("document_metadata", revision(info)) : previewJson("/metadata", post(revision(info)));
  },
  bookmarks(info: DocumentInfo): Promise<BookmarkInfo[]> {
    return native ? invoke("document_bookmarks", revision(info)) : previewJson("/bookmarks", post(revision(info)));
  },
  attachments(info: DocumentInfo): Promise<AttachmentInfo[]> {
    return native ? invoke("document_attachments", revision(info)) : previewJson("/attachments", post(revision(info)));
  },
  addAttachment(info: DocumentInfo): Promise<DocumentInfo | null> {
    if (!native) throw new Error("Use the browser attachment file picker.");
    return invoke("add_document_attachment", revision(info));
  },
  addAttachmentFile(info: DocumentInfo, file: File): Promise<DocumentInfo> {
    if (native) throw new Error("Use the native attachment file picker.");
    if (file.size > 16 * 1024 * 1024) throw new Error("The attachment exceeds 16 MiB.");
    return previewJson("/attachments/add", { method: "POST", headers: {
      "X-Document-Id": String(info.id), "X-Revision": String(info.revision),
      "X-File-Name": encodeURIComponent(file.name), "Content-Type": "application/octet-stream",
    }, body: file });
  },
  async exportAttachment(info: DocumentInfo, attachment: AttachmentInfo): Promise<boolean> {
    const input = { ...revision(info), attachmentId: attachment.id };
    if (native) return (await invoke<string | null>("export_document_attachment", input)) !== null;
    const response = await previewResponse("/attachments/export", post(input));
    download(await response.arrayBuffer(), attachment.suggestedName, "application/octet-stream");
    return true;
  },
  async exportProtectionCopy(info: DocumentInfo, protection: CopyProtection, authorizationPassword: string | null): Promise<boolean> {
    validateCopyProtection(protection);
    const input = { ...revision(info), protection, authorizationPassword };
    if (native) return (await invoke<string | null>("export_protection_copy", input)) !== null;
    const response = await previewResponse("/protection/export", post(input));
    const suffix = protection.mode === "unprotect" ? "unprotected" : "protected";
    download(await response.arrayBuffer(), `${info.name.replace(/\.pdf$/i, "")}-${suffix}.pdf`, "application/pdf");
    return true;
  },
};
