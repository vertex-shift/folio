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

// Use the pinned worker protocol directly so the raw worker is owned before
// initialization begins. createWorker() exposes it only after loading completes.
export class LocalOcrWorker {
  private readonly worker: Worker;
  private pending: Pending | null = null;
  private nextId = 0;
  private closed = false;
  private readonly abort: () => void;
  private readonly signal: AbortSignal;
  private readonly progress: OcrWorkerProgress;

  constructor(signal: AbortSignal, progress: OcrWorkerProgress) {
    signal.throwIfAborted();
    this.signal = signal;
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

  async initialize(dpi: number): Promise<void> {
    await this.request("load", { options: { corePath: localAsset(""), lstmOnly: true, logging: false } });
    await this.request("loadLanguage", { langs: "eng", options: {
      langPath: localAsset(""), gzip: false, cacheMethod: "none", lstmOnly: true,
    } });
    await this.request("initialize", { langs: "eng", oem: 1, config: { debug_file: "/dev/null" } });
    await this.request("setParameters", { params: {
      tessedit_pageseg_mode: "3", preserve_interword_spaces: "1", user_defined_dpi: String(dpi),
    } });
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
    this.signal.removeEventListener("abort", this.abort);
    this.worker.terminate();
    const pending = this.pending;
    this.pending = null;
    if (pending) { clearTimeout(pending.timeout); pending.reject(reason); }
  }
}
