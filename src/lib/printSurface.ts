import { printPageRule } from "./output";
import type { PrintImage } from "./output";

export async function requestBrowserPrint(): Promise<void> {
  if (typeof window.print !== "function") {
    throw new Error("This browser does not support a print dialog. Use a supported desktop browser.");
  }
  let timer: number | undefined;
  let acknowledge = () => {};
  const started = new Promise<boolean>((resolve) => {
    acknowledge = () => resolve(true);
    timer = window.setTimeout(() => resolve(false), 2000);
  });
  window.addEventListener("beforeprint", acknowledge, { once: true });
  try {
    window.print();
    if (!await started) {
      throw new Error("This browser did not acknowledge a print dialog. Printing may be blocked or unsupported; open the preview in a supported desktop browser.");
    }
  } finally {
    window.removeEventListener("beforeprint", acknowledge);
    window.clearTimeout(timer);
  }
}

export class PreparedPrintSurface {
  private readonly root = document.createElement("div");
  private readonly style = document.createElement("style");
  private readonly urls: string[] = [];
  private readonly media = window.matchMedia("print");
  private readonly beginPrint = () => { this.printing = true; };
  private readonly endPrint = () => { this.printing = false; };
  private readonly mediaChange = (event: MediaQueryListEvent) => { this.printing = event.matches; };
  private pageCount = 0;
  printing = false;
  requested = false;

  constructor() {
    if (document.querySelector(".folio-print-surface")) {
      throw new Error("Another print snapshot is still open. Finish that print dialog first.");
    }
    this.root.className = "folio-print-surface";
    this.root.setAttribute("aria-hidden", "true");
    this.style.media = "print";
    this.root.append(this.style);
    document.body.append(this.root);
    window.addEventListener("beforeprint", this.beginPrint);
    window.addEventListener("afterprint", this.endPrint);
    this.media.addEventListener("change", this.mediaChange);
  }

  async append(image: PrintImage): Promise<void> {
    if (this.requested) throw new Error("A print snapshot cannot change after its dialog has opened.");
    const index = this.pageCount;
    const url = URL.createObjectURL(image.blob);
    this.urls.push(url);
    const sheet = document.createElement("section");
    sheet.className = "folio-print-sheet";
    sheet.style.page = `folio-output-${index}`;
    sheet.style.width = `${image.widthPoints}pt`;
    sheet.style.height = `${image.heightPoints}pt`;
    const img = document.createElement("img");
    img.alt = "";
    img.src = url;
    img.width = image.pixelWidth;
    img.height = image.pixelHeight;
    sheet.append(img);
    this.root.append(sheet);
    this.style.textContent += printPageRule(image, index);
    if (index === 0) {
      this.style.textContent += `@page { size: ${image.widthPoints}pt ${image.heightPoints}pt; margin: 0; }`;
    }
    await img.decode();
    this.pageCount += 1;
  }

  finish(expected: number): void {
    if (!this.pageCount || this.pageCount !== expected) throw new Error("The print snapshot is incomplete.");
    const rules = Array.from(this.style.sheet?.cssRules ?? []);
    const sizedPages = rules.filter((rule) => "style" in rule
      && rule instanceof CSSPageRule && Boolean(rule.style.getPropertyValue("size")));
    if (sizedPages.length < expected || !CSS.supports("page", "folio-output-0")) {
      throw new Error("This browser/webview cannot preserve PDF page sizes for printing. Export images or print from another PDF reader.");
    }
    this.root.dataset.ready = "true";
  }

  markRequested(): void {
    if (this.root.dataset.ready !== "true") throw new Error("Prepare the full print snapshot before opening the dialog.");
    this.requested = true;
  }

  dispose(confirmedFinished = false): void {
    if (this.media.matches || this.printing && !confirmedFinished) {
      throw new Error("Close or finish the platform print dialog before clearing its prepared pages.");
    }
    this.printing = false;
    window.removeEventListener("beforeprint", this.beginPrint);
    window.removeEventListener("afterprint", this.endPrint);
    this.media.removeEventListener("change", this.mediaChange);
    this.root.remove();
    for (const url of this.urls) URL.revokeObjectURL(url);
    this.urls.length = 0;
  }
}
