import { PAGE_SIZES, parseWritingNumber } from "../../lib/pages.ts";

export type CreationKind = "images" | "text";
export const MAX_CREATION_FILES = 50;
export const MAX_CREATION_PAGES = 500;
export const MAX_CREATION_IMAGE_BYTES = 16 * 1024 * 1024;
export const MAX_CREATION_INPUT_BYTES = 64 * 1024 * 1024;
export const MAX_TEXT_FILE_BYTES = 1024 * 1024;
export const MAX_CREATION_TEXT_BYTES = 4 * 1024 * 1024;
export const MAX_DOCX_SOURCE_BYTES = 64 * 1024 * 1024;
export const MAX_DOCX_PAGES = 200;
export const MAX_DOCX_PAGE_TEXT_BYTES = 256 * 1024;
export const MAX_DOCX_TEXT_BYTES = 8 * 1024 * 1024;

export interface CreationSettings {
  width: number;
  height: number;
  margin: number;
  fontSize: number;
}

export interface CreationInput {
  index: number;
  name: string;
  byteLength: number;
}

export interface CreationPlan {
  jobId: number;
  kind: CreationKind;
  inputs: CreationInput[];
}

export interface ReadyCreationStatus {
  jobId: number;
  phase: "ready";
  pageCount: number;
  byteLength: number;
}

export type CreationStatus = ReadyCreationStatus
  | { jobId: number; phase: "preparing"; pageCount: null; byteLength: null };

export function creationSettings(size: keyof typeof PAGE_SIZES, landscape: boolean, margin: string, fontSize: string): CreationSettings {
  const page = PAGE_SIZES[size];
  const result = {
    width: landscape ? page.height : page.width,
    height: landscape ? page.width : page.height,
    margin: parseWritingNumber(margin, 0, 144, "Margin"),
    fontSize: parseWritingNumber(fontSize, 8, 36, "Text size"),
  };
  if (result.width - 2 * result.margin < 72 || result.height - 2 * result.margin < 72) {
    throw new Error("The margin must leave at least 72 points of content in each direction.");
  }
  return result;
}

export function validateCreationFiles(kind: CreationKind, files: readonly { name: string; size: number }[]) {
  if (!files.length || files.length > MAX_CREATION_FILES) throw new Error("Choose from 1 to 50 local files.");
  const limit = kind === "images" ? MAX_CREATION_IMAGE_BYTES : MAX_TEXT_FILE_BYTES;
  let total = 0;
  for (const file of files) {
    if (!file.name || file.name.length > 512 || /[/\\\u0000-\u001f\u007f]/u.test(file.name)) {
      throw new Error("A selected input has an invalid filename.");
    }
    if (kind === "images" && !/\.(png|jpe?g)$/i.test(file.name)) throw new Error(`${file.name}: choose a still PNG or JPEG.`);
    if (!Number.isSafeInteger(file.size) || file.size < 1 || file.size > limit) {
      throw new Error(`${file.name}: ${kind === "images" ? "each image must contain 1 byte to 16 MiB" : "each UTF-8 text file must contain 1 byte to 1 MiB"}.`);
    }
    total += file.size;
  }
  if (total > (kind === "images" ? MAX_CREATION_INPUT_BYTES : MAX_CREATION_TEXT_BYTES)) {
    throw new Error(kind === "images" ? "The selected images exceed 64 MiB." : "The selected text exceeds 4 MiB.");
  }
}

export function moveCreationInput(order: readonly number[], position: number, direction: -1 | 1): number[] {
  const target = position + direction;
  if (!Number.isInteger(position) || position < 0 || position >= order.length || target < 0 || target >= order.length) {
    throw new Error("That file cannot move farther in the page order.");
  }
  const next = [...order];
  [next[position], next[target]] = [next[target], next[position]];
  return next;
}

export function validateDocxText(text: string): void {
  for (const character of text) {
    const code = character.codePointAt(0)!;
    if (!(code === 9 || code === 10 || code === 13 || code >= 0x20 && code <= 0xd7ff
      || code >= 0xe000 && code <= 0xfffd || code >= 0x10000 && code <= 0x10ffff)) {
      throw new Error(`DOCX cannot represent extracted XML character U+${code.toString(16).toUpperCase().padStart(4, "0")}. No text was silently removed.`);
    }
  }
}

export function waitForCreation(
  jobId: number,
  status: (jobId: number) => Promise<CreationStatus>,
  signal: AbortSignal,
): Promise<ReadyCreationStatus> {
  return (async () => {
    for (;;) {
      signal.throwIfAborted();
      const result = await status(jobId);
      signal.throwIfAborted();
      if (result.phase === "ready") {
        if (!Number.isSafeInteger(result.pageCount) || result.pageCount < 1 || result.pageCount > MAX_CREATION_PAGES
          || !Number.isSafeInteger(result.byteLength) || result.byteLength < 1 || result.byteLength > 128 * 1024 * 1024) {
          throw new Error("The creation engine returned an invalid ready PDF size or page count. No document was opened.");
        }
        return result;
      }
      await new Promise<void>((resolve, reject) => {
        const abort = () => { clearTimeout(timeout); reject(signal.reason); };
        const timeout = setTimeout(() => {
          signal.removeEventListener("abort", abort); resolve();
        }, 100);
        signal.addEventListener("abort", abort, { once: true });
        if (signal.aborted) abort();
      });
    }
  })();
}
