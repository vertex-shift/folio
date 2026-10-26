export const MAX_MANAGEMENT_SOURCE_BYTES = 64 * 1024 * 1024;

export interface MetadataFields {
  title: string;
  author: string;
  subject: string;
  keywords: string;
}

export interface MetadataInfo {
  fields: MetadataFields;
  xmpPresent: boolean;
  editable: boolean;
  reason: string | null;
}

export interface MetadataDraft {
  documentId: number;
  revision: number;
  original: MetadataFields;
  fields: MetadataFields;
}

export interface BookmarkInfo {
  id: string;
  title: string;
  parent: string | null;
  depth: number;
  position: number;
  childCount: number;
  page: number | null;
  namedDestination: string | null;
  url: string | null;
  targetDescription: string;
}

export interface AttachmentInfo {
  id: string;
  name: string;
  description: string;
  byteLength: number;
  suggestedName: string;
}

export interface ProtectionAccess {
  ownerAuthenticated: boolean;
  printAllowed: boolean;
  highQualityPrintAllowed: boolean;
  copyAllowed: boolean;
  modifyAllowed: boolean;
  annotateAllowed: boolean;
  fillFormsAllowed: boolean;
  assembleAllowed: boolean;
}

export interface CopyPermissions {
  print: "none" | "low" | "high";
  copy: boolean;
  edit: boolean;
  annotate: boolean;
  fillForms: boolean;
  assemble: boolean;
}

export type CopyProtection = {
  mode: "protect";
  openingPassword: string;
  ownerPassword: string;
  permissions: CopyPermissions;
} | { mode: "unprotect" };

export function metadataChanged(draft: MetadataDraft | null): boolean {
  return draft !== null && (Object.keys(draft.fields) as (keyof MetadataFields)[])
    .some((key) => draft.fields[key] !== draft.original[key]);
}

export function suggestedAttachmentName(value: string): string {
  const leaf = value.split(/[/\\]/).at(-1) ?? "";
  const characters = Array.from(leaf, (character) =>
    /[\u0000-\u001f\u007f-\u009f<>:"/\\|?*]/u.test(character) ? "_" : character,
  ).slice(0, 120);
  let length = 0;
  const encoder = new TextEncoder();
  let name = characters.filter((character) => {
    length += encoder.encode(character).length;
    return length <= 240;
  }).join("").replace(/^[ .]+|[ .]+$/g, "");
  if (/^(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)/i.test(name)) name = `_${name}`;
  return name || "attachment.bin";
}

export function validateCopyProtection(protection: CopyProtection): void {
  if (protection.mode === "unprotect") return;
  if (!protection.ownerPassword || protection.ownerPassword === protection.openingPassword) {
    throw new Error("Set a nonempty owner password different from the opening password.");
  }
  const encoder = new TextEncoder();
  if ([protection.openingPassword, protection.ownerPassword].some((value) => encoder.encode(value).length > 127)) {
    throw new Error("New passwords must fit within 127 UTF-8 bytes. Unicode characters can use more than one byte.");
  }
}

export function bookmarkMove(
  entries: BookmarkInfo[], entry: BookmarkInfo, direction: "up" | "down" | "in" | "out",
): { parent: string | null; at: number } | null {
  const siblings = entries.filter((candidate) => candidate.parent === entry.parent);
  const index = siblings.findIndex((candidate) => candidate.id === entry.id);
  if (index < 0) return null;
  if (direction === "up") return index > 0 ? { parent: entry.parent, at: index - 1 } : null;
  if (direction === "down") return index + 1 < siblings.length ? { parent: entry.parent, at: index + 1 } : null;
  if (direction === "in") {
    const previous = siblings[index - 1];
    return previous ? { parent: previous.id, at: previous.childCount } : null;
  }
  const parent = entries.find((candidate) => candidate.id === entry.parent);
  return parent ? { parent: parent.parent, at: parent.position + 1 } : null;
}
