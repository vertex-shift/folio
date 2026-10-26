import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const englishModel = {
  source: "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/87416418657359cb625c412a48b6e1d6d41c29bd/eng.traineddata",
  sha256: "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2",
};

const runtimeAssets = {
  "worker.min.js": "38645599043239c0eb6db08a6504a92dcdc292200535f3e9339cd77c4443b842",
  "tesseract-core.wasm": "740f7e466a2f026e1f1e54a2e0d9957e386d8166110d1290224abd3d08986106",
  "tesseract-core.wasm.js": "e66872f6a76f5ad414d73d21512245df0de3060ad4871a97c47efceaab27b955",
  "tesseract-core-lstm.wasm": "220e2e87551edccb85519796a170469f8ab2a8055216789e3b8b1ada18b7bc2b",
  "tesseract-core-lstm.wasm.js": "775a35df6f2ae100e02609443e6bd5cafcd07983dd6175454ca4a432a7730687",
  "tesseract-core-simd.wasm": "ca11ee92a02023e4f5083eefd0966fb83b297416da54e997b9f6ce445f719c9b",
  "tesseract-core-simd.wasm.js": "3b0678c47a8dea6abb931b214171c08b742a5b9a9fcbbb1a028a08d5de6e9d4c",
  "tesseract-core-simd-lstm.wasm": "187d76742dfc0d8929f0b49a619f145bb6370730776c7bd0d3e20c6b2098808d",
  "tesseract-core-simd-lstm.wasm.js": "9d7c43fb206dc9f48475228b46bf35f888fa9e6259da2e67d5a75c77049f2dc7",
};

export function prepareOcr(root) {
  const destination = join(root, "public/vendor/ocr");
  mkdirSync(destination, { recursive: true });
  const files = [];
  const copy = (source, name) => {
    const bytes = readFileSync(source);
    const sha256 = createHash("sha256").update(bytes).digest("hex");
    if (Object.hasOwn(runtimeAssets, name) && sha256 !== runtimeAssets[name]) {
      throw new Error(`The bundled OCR asset ${name} failed SHA-256 verification. Restore the committed npm lockfile.`);
    }
    const target = join(destination, name);
    if (!existsSync(target) || !readFileSync(target).equals(bytes)) copyFileSync(source, target);
    files.push({ name, sha256 });
  };
  for (const [name, version] of [["tesseract.js", "6.0.1"], ["tesseract.js-core", "6.1.2"]]) {
    const metadata = JSON.parse(readFileSync(join(root, "node_modules", name, "package.json"), "utf8"));
    if (metadata.version !== version) throw new Error(`Offline OCR requires ${name} ${version}; restore the committed npm lockfile.`);
    copy(join(root, "node_modules", name, name === "tesseract.js" ? "LICENSE.md" : "LICENSE"), `${name}-LICENSE`);
  }
  const model = join(root, "src-tauri/resources/ocr/eng.traineddata");
  if (createHash("sha256").update(readFileSync(model)).digest("hex") !== englishModel.sha256) {
    throw new Error("The bundled English OCR model failed SHA-256 verification. Restore the committed model; do not substitute unverified data.");
  }
  copy(model, "eng.traineddata");
  copy(join(root, "src-tauri/resources/ocr/LICENSE"), "eng-LICENSE");
  copy(join(root, "src-tauri/resources/ocr/NOTICE"), "NOTICE");
  for (const name of ["tesseract", "leptonica", "giflib", "libjpeg", "libpng", "libtiff", "libwebp", "libwebp-patents", "openlibm", "zlib"]) {
    copy(join(root, "src-tauri/resources/ocr/notices", `${name}.txt`), `${name}-NOTICE.txt`);
  }
  copy(join(root, "node_modules/tesseract.js/dist/worker.min.js"), "worker.min.js");
  for (const variant of ["", "-lstm", "-simd", "-simd-lstm"]) {
    for (const extension of [".wasm", ".wasm.js"]) {
      const name = `tesseract-core${variant}${extension}`;
      copy(join(root, "node_modules/tesseract.js-core", name), name);
    }
  }
  const manifest = JSON.stringify({ engine: "6.0.1", core: "6.1.2", model: englishModel, files }, null, 2) + "\n";
  const path = join(destination, "manifest.json");
  if (!existsSync(path) || readFileSync(path, "utf8") !== manifest) writeFileSync(path, manifest);
}
