import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const catalog = JSON.parse(readFileSync(fileURLToPath(new URL("../src-tauri/resources/ocr/catalog.json", import.meta.url)), "utf8"));
export { catalog as ocrCatalog };

export const englishModel = {
  source: "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/87416418657359cb625c412a48b6e1d6d41c29bd/eng.traineddata",
  sha256: "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2",
};

export function prepareOcr(root) {
  const destination = join(root, "public/vendor/ocr");
  mkdirSync(destination, { recursive: true });
  const files = [];
  const copy = (source, name, expected) => {
    const bytes = readFileSync(source);
    const sha256 = createHash("sha256").update(bytes).digest("hex");
    if (Object.hasOwn(catalog.runtime, name) && sha256 !== catalog.runtime[name]) {
      throw new Error(`The bundled OCR asset ${name} failed SHA-256 verification. Restore the committed npm lockfile.`);
    }
    if (expected && (sha256 !== expected.sha256 || bytes.length !== expected.bytes)) {
      throw new Error(`The bundled OCR resource ${name} failed SHA-256 or byte-size verification. Restore the committed asset; no unverified substitution is allowed.`);
    }
    const target = join(destination, name);
    mkdirSync(join(target, ".."), { recursive: true });
    if (!existsSync(target) || !readFileSync(target).equals(bytes)) copyFileSync(source, target);
    files.push({ name, sha256, bytes: bytes.length });
  };
  for (const [name, version] of [["tesseract.js", "6.0.1"], ["tesseract.js-core", "6.1.2"]]) {
    const metadata = JSON.parse(readFileSync(join(root, "node_modules", name, "package.json"), "utf8"));
    if (metadata.version !== version) throw new Error(`Offline OCR requires ${name} ${version}; restore the committed npm lockfile.`);
    copy(join(root, "node_modules", name, name === "tesseract.js" ? "LICENSE.md" : "LICENSE"), `${name}-LICENSE`);
  }
  const resources = join(root, "src-tauri/resources/ocr");
  for (const language of catalog.languages) {
    copy(join(resources, `${language.id}.traineddata`), `${language.id}.traineddata`, language);
  }
  for (const font of catalog.fonts) {
    const name = font.file.split("/").at(-1);
    copy(join(resources, font.file), `fonts/${name}`, font);
    copy(join(resources, font.license), `fonts/${font.id}-OFL.txt`);
  }
  copy(join(resources, "LICENSE"), "tessdata-LICENSE");
  copy(join(resources, "catalog.json"), "catalog.json");
  copy(join(resources, "fonts/NOTICE"), "fonts/NOTICE");
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
  const manifest = JSON.stringify({ engine: catalog.engine, core: catalog.core, model: englishModel,
    models: catalog.languages, fonts: catalog.fonts, files }, null, 2) + "\n";
  const path = join(destination, "manifest.json");
  if (!existsSync(path) || readFileSync(path, "utf8") !== manifest) writeFileSync(path, manifest);
}
