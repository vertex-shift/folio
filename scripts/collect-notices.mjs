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
    sections.push(`\n${"=".repeat(72)}\n${title}\n${"=".repeat(72)}\n${readFileSync(path, "utf8")}`);
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
  const names = new Set(["lopdf", "tauri", "tauri-plugin-opener", "rfd", "tempfile", "serde", "serde_json", "thiserror", "ttf-parser", "unicode-normalization"]);
  for (const dependency of metadata.packages.filter((entry) => names.has(entry.name))) {
    const folder = dirname(dependency.manifest_path);
    const candidates = ["LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "LICENSE.md", "LICENSE.txt", "LICENSE.spdx", "COPYING"];
    const licenses = candidates.filter((name) => existsSync(join(folder, name)));
    if (!licenses.length) throw new Error(`No license file found for ${dependency.name} ${dependency.version}`);
    for (const name of licenses) append(`${dependency.name} ${dependency.version} / ${name}`, join(folder, name));
  }
  const text = sections.join("\n\n") + "\n";
  const path = join(root, "src-tauri/resources/THIRD_PARTY_NOTICES.txt");
  if (!existsSync(path) || readFileSync(path, "utf8") !== text) writeFileSync(path, text);
}
