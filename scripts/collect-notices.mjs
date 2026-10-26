import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

export function collectNotices(root) {
  const sections = [
    "FOLIO THIRD-PARTY NOTICES",
    "Folio is independently implemented. No resources were extracted from Acrobat.",
    "The following are dependency notices, not a licensing audit or a redistribution certification.",
  ];
  const append = (title, path) => {
    if (!existsSync(path)) throw new Error(`Required dependency notice is missing: ${path}`);
    sections.push(`\n${"=".repeat(72)}\n${title}\n${"=".repeat(72)}\n${readFileSync(path, "utf8").replace(/\r\n/g, "\n")}`);
  };
  for (const [name, license] of [
    ["react", "LICENSE"], ["react-dom", "LICENSE"], ["scheduler", "LICENSE"],
    ["lucide-react", "LICENSE"], ["@tauri-apps/api", "LICENSE-MIT"],
    ["@tauri-apps/plugin-opener", "LICENSE.spdx"],
    ["pdfjs-dist", "LICENSE"],
  ]) {
    append(name, join(root, "node_modules", name, license));
  }
  append("Noto Sans embedded PDF font", join(root, "src-tauri/resources/fonts/OFL.txt"));
  for (const [title, folder] of [
    ["PDF.js CMap data", "cmaps"], ["PDF.js ICC profiles", "iccs"],
  ]) {
    append(title, join(root, "node_modules/pdfjs-dist", folder, "LICENSE"));
  }
  const metadata = JSON.parse(execFileSync("cargo", [
    "metadata", "--manifest-path", join(root, "src-tauri/Cargo.toml"),
    "--format-version", "1", "--locked", "--all-features",
  ], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 }));
  const names = new Set(["lopdf", "tauri", "tauri-plugin-opener", "rfd", "tempfile", "serde", "serde_json", "thiserror", "ttf-parser", "unicode-normalization", "png",
    "openssl", "openssl-sys", "openssl-src", "openssl-macros", "foreign-types", "foreign-types-shared",
    "cms", "der", "der_derive", "const-oid", "spki", "x509-cert", "flagset", "zeroize", "zeroize_derive",
    "base64ct", "pem-rfc7468", "tls_codec", "tls_codec_derive",
    "image", "zune-jpeg", "zune-core", "byteorder-lite", "bytemuck", "moxcms", "pxfm",
    "getrandom", "quick-xml"]);
  for (const dependency of metadata.packages.filter((entry) => names.has(entry.name))) {
    const folder = dirname(dependency.manifest_path);
    const candidates = ["LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "LICENSE-MIT.md", "LICENSE.md", "LICENSE.txt", "LICENSE.spdx", "COPYING"];
    const licenses = candidates.filter((name) => existsSync(join(folder, name)));
    if (!licenses.length && dependency.name === "cms" && dependency.version === "0.2.3"
        && dependency.license === "Apache-2.0 OR MIT") {
      // This published crate contains the upstream license grant in README.md,
      // but omits standalone license files. Select its Apache-2.0 option.
      const readme = readFileSync(join(folder, "README.md"), "utf8");
      const grant = readme.match(/## License\n([\s\S]*?)(?=\n\[\/\/\])/);
      if (!grant) throw new Error("The cms 0.2.3 upstream license grant is missing.");
      sections.push(`\n${"=".repeat(72)}\ncms 0.2.3 / upstream license grant\n${"=".repeat(72)}\n`
        + `RustCrypto CMS: ${dependency.repository}\nDistributed under the Apache-2.0 option.\n${grant[1].trim()}`);
      append("cms 0.2.3 / Apache License 2.0 (full terms)", join(root, "LICENSE"));
    } else if (!licenses.length) {
      throw new Error(`No license file found for ${dependency.name} ${dependency.version}`);
    }
    for (const name of licenses) append(`${dependency.name} ${dependency.version} / ${name}`, join(folder, name));
    if (dependency.name === "openssl-src") {
      sections.push("This product includes OpenSSL software developed by The OpenSSL Project Authors "
        + "(https://www.openssl.org/). The bundled OpenSSL library is licensed under Apache-2.0; "
        + "its license is separate from the openssl-src wrapper crate's license.");
      append(`Vendored OpenSSL library (${dependency.version}) / LICENSE.txt`, join(folder, "openssl", "LICENSE.txt"));
      for (const notice of ["NOTICE", "AUTHORS.md"]) {
        if (existsSync(join(folder, "openssl", notice))) {
          append(`Vendored OpenSSL library / ${notice}`, join(folder, "openssl", notice));
        }
      }
    }
  }
  const text = sections.join("\n\n") + "\n";
  const path = join(root, "src-tauri/resources/THIRD_PARTY_NOTICES.txt");
  if (!existsSync(path) || readFileSync(path, "utf8") !== text) writeFileSync(path, text);
}
