import { ocrLanguageResources, selectedOcrLanguages } from "./ocrLanguages.ts";
import type { OcrLanguage } from "./ocrLanguages.ts";
import { sha256 as digestSha256 } from "@noble/hashes/sha2.js";

export type OcrWorkerProgress = (fraction: number) => void;

interface Pending {
  id: string;
  resolve(data: unknown): void;
  reject(error: Error): void;
  timeout: ReturnType<typeof setTimeout>;
}

function localAsset(name: string): string {
  return new URL(`/vendor/ocr/${name}`, window.location.href).href;
}

export async function verifiedOcrAsset(name: string, size: number, sha256: string, signal: AbortSignal): Promise<ArrayBuffer> {
  signal.throwIfAborted();
  const response = await fetch(localAsset(name), { signal, cache: "no-store", credentials: "same-origin" });
  if (!response.ok || !response.body) throw new Error(`The bundled OCR resource ${name} is missing. Restore the app assets; no text was added.`);
  const reader = response.body.getReader();
  const data = new Uint8Array(size);
  let length = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      signal.throwIfAborted();
      if (done) break;
      length += value.byteLength;
      if (length > size) throw new Error(`The bundled OCR resource ${name} exceeds its verified size. No text was added.`);
      data.set(value, length - value.byteLength);
    }
  } finally {
    await reader.cancel();
    reader.releaseLock();
  }
  if (length !== size) throw new Error(`The bundled OCR resource ${name} has the wrong size. Restore the app assets; no text was added.`);
  const digest = digestSha256(data);
  signal.throwIfAborted();
  if (Array.from(digest, (value) => value.toString(16).padStart(2, "0")).join("") !== sha256) {
    throw new Error(`The bundled OCR resource ${name} failed SHA-256 verification. Restore the app assets; no text was added.`);
  }
  return data.buffer;
}

// Use the pinned worker protocol directly so the raw worker is owned before
// initialization begins. createWorker() exposes it only after loading completes.
export class LocalOcrWorker {
  private readonly worker: Worker;
  private pending: Pending | null = null;
  private nextId = 0;
  private closed = false;
  private readonly abort: () => void;
  private readonly signal: AbortSignal;
  private readonly sourceSignal: AbortSignal;
  private readonly controller = new AbortController();
  private readonly progress: OcrWorkerProgress;

  constructor(signal: AbortSignal, progress: OcrWorkerProgress) {
    signal.throwIfAborted();
    this.sourceSignal = signal;
    this.signal = this.controller.signal;
    this.progress = progress;
    this.worker = new Worker(localAsset("worker.min.js"));
    this.abort = () => this.close(signal.reason instanceof Error ? signal.reason : new Error("OCR canceled."));
    signal.addEventListener("abort", this.abort, { once: true });
    this.worker.onmessage = ({ data }: MessageEvent<unknown>) => {
      if (typeof data !== "object" || data === null || !("jobId" in data) || data.jobId !== this.pending?.id) return;
      if (!("status" in data)) return;
      if (data.status === "progress" && "data" in data && typeof data.data === "object" && data.data !== null
        && "status" in data.data && data.data.status === "recognizing text"
        && "progress" in data.data && typeof data.data.progress === "number" && Number.isFinite(data.data.progress)) {
        this.progress(Math.max(0, Math.min(1, data.data.progress)));
      } else if (data.status === "resolve") {
        const pending = this.pending;
        this.pending = null;
        if (pending) { clearTimeout(pending.timeout); pending.resolve("data" in data ? data.data : undefined); }
      } else if (data.status === "reject") {
        this.close(new Error("The bundled OCR engine could not process this page. No text was added. Check the scan quality and try a smaller page range."));
      }
    };
    this.worker.onerror = (event) => {
      event.preventDefault();
      this.close(new Error("The local OCR worker failed to load or run. Restore the bundled OCR assets and try again; no text was added."));
    };
    this.worker.onmessageerror = () => this.close(new Error("The local OCR worker returned unreadable data. No text was added."));
  }

  private request(action: string, payload: object, transfer: Transferable[] = []): Promise<unknown> {
    this.signal.throwIfAborted();
    if (this.closed || this.pending) return Promise.reject(new Error("The OCR worker is unavailable or already processing a page."));
    const id = `folio-ocr-${++this.nextId}`;
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => this.close(new Error("An OCR step exceeded its two-minute limit. No text was added; try fewer or smaller pages.")), 120_000);
      this.pending = { id, resolve, reject, timeout };
      try {
        this.worker.postMessage({ workerId: "folio-local-ocr", jobId: id, action, payload }, transfer);
      } catch {
        this.close(new Error("The page could not be sent to the local OCR worker. No text was added."));
      }
    });
  }

  async initialize(dpi: number, languages: readonly OcrLanguage[] = ["eng"], segmentation: "auto" | "single_column" = "auto"): Promise<void> {
    const timeout = setTimeout(() => this.close(new Error("OCR initialization exceeded its two-minute limit. No text was added.")), 120_000);
    try {
      const selected = selectedOcrLanguages(languages);
      if (dpi !== 150 && dpi !== 300) throw new Error("Choose 150 or 300 DPI for OCR.");
      if (segmentation !== "auto" && segmentation !== "single_column") throw new Error("Choose automatic horizontal text or single-column printed text.");
      const langs = [];
      for (const model of ocrLanguageResources(selected)) {
        const data = await verifiedOcrAsset(model.name, model.bytes, model.sha256, this.signal);
        if (this.closed) throw new Error("OCR initialization was canceled or expired. No text was added.");
        langs.push({ code: model.id, data: new Uint8Array(data) });
      }
      await this.request("load", { options: { corePath: localAsset(""), lstmOnly: true, logging: false } });
      await this.request("loadLanguage", { langs, options: {
        langPath: localAsset(""), gzip: false, cacheMethod: "none", lstmOnly: true,
      } }, langs.map((language) => language.data.buffer));
      await this.request("initialize", { langs: selected.join("+"), oem: 1,
        config: "debug_file /dev/null\ntessedit_load_sublangs\ntextord_tabfind_vertical_text 0\ntextord_tabfind_force_vertical_text 0" });
      await this.request("setParameters", { params: {
        tessedit_pageseg_mode: segmentation === "auto" ? "3" : "6", preserve_interword_spaces: "1", user_defined_dpi: String(dpi),
        textord_tabfind_vertical_text: "0", textord_tabfind_force_vertical_text: "0",
      } });
    } catch (error) {
      this.close(error instanceof Error ? error : new Error("The selected OCR models could not initialize. No text was added."));
      throw error;
    } finally { clearTimeout(timeout); }
  }

  recognize(png: ArrayBuffer): Promise<unknown> {
    return this.request("recognize", { image: new Uint8Array(png),
      options: { rotateAuto: false, rotateRadians: 0 },
      output: { text: true, blocks: true, debug: false, pdf: false, hocr: false, tsv: false },
    }, [png]);
  }

  close(reason = new Error("The OCR worker was released.")): void {
    if (this.closed) return;
    this.closed = true;
    this.sourceSignal.removeEventListener("abort", this.abort);
    this.controller.abort(reason);
    this.worker.terminate();
    const pending = this.pending;
    this.pending = null;
    if (pending) { clearTimeout(pending.timeout); pending.reject(reason); }
  }
}
