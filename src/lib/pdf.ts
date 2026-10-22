import { GlobalWorkerOptions, getDocument } from "pdfjs-dist/legacy/build/pdf.mjs";
import workerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";

GlobalWorkerOptions.workerSrc = workerUrl;

export function loadPdf(bytes: Uint8Array) {
  return getDocument({
    data: bytes,
    cMapUrl: new URL("/vendor/pdfjs/cmaps/", window.location.href).href,
    cMapPacked: true,
    standardFontDataUrl: new URL("/vendor/pdfjs/standard_fonts/", window.location.href).href,
    iccUrl: new URL("/vendor/pdfjs/iccs/", window.location.href).href,
    wasmUrl: new URL("/vendor/pdfjs/wasm/", window.location.href).href,
    enableXfa: false,
    useSystemFonts: true,
  });
}
