import { GlobalWorkerOptions, getDocument } from "pdfjs-dist/legacy/build/pdf.mjs";
GlobalWorkerOptions.workerSrc = new URL("/vendor/pdfjs/folio-worker.mjs", window.location.href).href;

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
