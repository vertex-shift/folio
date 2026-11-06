import assert from "node:assert/strict";
import { createHash, createPublicKey, verify } from "node:crypto";
import { copyFile, mkdir, readFile, readdir, stat, writeFile } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { blake2b } from "@noble/hashes/blake2.js";

const MAX_UPDATE_BYTES = 512 * 1024 * 1024;
const specifications = [
  ["folio-windows-x64", ".exe", "windows_x64_setup.exe", "windows-x86_64"],
  ["folio-macos-arm64", ".dmg", "macos_arm64.dmg", null],
  ["folio-linux-x64", ".deb", "linux_x64.deb", null],
  ["folio-linux-x64", ".AppImage", "linux_x64.AppImage", "linux-x86_64"],
  ["folio-macos-arm64", ".app.tar.gz", "macos_arm64.app.tar.gz", "darwin-aarch64"],
];

function decode(value) {
  const text = value.trim();
  assert.match(text, /^[A-Za-z0-9+/]+={0,2}$/, "Invalid update signature/key encoding");
  const bytes = Buffer.from(text, "base64");
  assert.equal(bytes.toString("base64"), text, "Noncanonical update signature/key encoding");
  return bytes;
}

// Verify the same prehashed Minisign format and trusted version used by Tauri.
export function verifyUpdate(bytes, encodedSignature, encodedKey, version) {
  const publicLines = decode(encodedKey).toString("utf8").trimEnd().split(/\r?\n/);
  const publicKey = decode(publicLines[1] ?? "");
  const lines = decode(encodedSignature).toString("utf8").trimEnd().split(/\r?\n/);
  assert.equal(lines.length, 4, "Expected a complete Minisign signature");
  const signature = decode(lines[1]);
  const global = decode(lines[3]);
  assert.equal(publicKey.length, 42, "Invalid Minisign public key");
  assert.equal(publicKey.subarray(0, 2).toString(), "Ed", "Unsupported Minisign key algorithm");
  assert.equal(signature.length, 74, "Invalid Minisign signature");
  assert.equal(signature.subarray(0, 2).toString(), "ED", "Expected a prehashed Minisign signature");
  assert.ok(publicKey.subarray(2, 10).equals(signature.subarray(2, 10)), "Wrong update signing key");
  assert.match(lines[2], /^trusted comment: /, "Missing signed update comment");
  const comment = lines[2].slice(17);
  const key = createPublicKey({
    key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), publicKey.subarray(10)]),
    format: "der", type: "spki",
  });
  assert.ok(verify(null, blake2b(bytes), key, signature.subarray(10)), "Invalid update package signature");
  assert.ok(verify(null, Buffer.concat([signature.subarray(10), Buffer.from(comment)]), key, global),
    "Invalid update version signature");
  const versions = comment.split("\t").filter((field) => field.startsWith("version:"));
  assert.deepEqual(versions, [`version:${version}`], "Signed version does not match release");
}

async function filesUnder(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  return (await Promise.all(entries.map(async (entry) => {
    const path = join(directory, entry.name);
    assert.ok(!entry.isSymbolicLink(), "Release artifacts must not be symbolic links");
    return entry.isDirectory() ? filesUnder(path) : [path];
  }))).flat();
}

export async function prepareRelease({ artifacts, output, version, tag, publicKey, date = new Date().toISOString() }) {
  assert.equal(tag, `v${version}`, "The release tag must match the application version");
  assert.match(version, /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/);
  const all = await filesUnder(artifacts);
  const selected = new Set();
  const copies = [];
  const platforms = {};
  for (const [artifact, extension, suffix, platform] of specifications) {
    const matches = all.filter((path) => path.startsWith(join(artifacts, artifact) + (process.platform === "win32" ? "\\" : "/"))
      && path.endsWith(extension));
    assert.equal(matches.length, 1, `Expected one ${extension} in ${artifact}`);
    const source = matches[0];
    const size = (await stat(source)).size;
    assert.ok(size > 0 && size <= MAX_UPDATE_BYTES, `Invalid release asset size: ${basename(source)}`);
    selected.add(source);
    const name = `Folio_${version}_${suffix}`;
    copies.push([source, name]);
    if (platform) {
      const signaturePath = `${source}.sig`;
      assert.ok(all.includes(signaturePath), `Missing signature: ${basename(source)}`);
      const signature = (await readFile(signaturePath, "utf8")).trim();
      verifyUpdate(await readFile(source), signature, publicKey, version);
      selected.add(signaturePath);
      copies.push([signaturePath, `${name}.sig`]);
      platforms[platform] = {
        signature,
        url: `https://github.com/vertex-shift/folio/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`,
      };
    }
  }
  assert.equal(selected.size, all.length, "Unexpected files in release artifacts");
  // Validate everything before creating publishable output.
  await mkdir(output);
  const manifest = { version, notes: `Folio ${version}`, pub_date: date, platforms };
  await writeFile(join(output, "latest.json"), JSON.stringify(manifest, null, 2) + "\n");
  for (const [source, name] of copies) await copyFile(source, join(output, name));
  const names = [...copies.map(([, name]) => name), "latest.json"].sort();
  const checksums = await Promise.all(names.map(async (name) =>
    `${createHash("sha256").update(await readFile(join(output, name))).digest("hex")}  ${name}\n`));
  await writeFile(join(output, "SHA256SUMS"), checksums.join(""));
  return manifest;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const pkg = JSON.parse(await readFile("package.json", "utf8"));
  const config = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"));
  await prepareRelease({
    artifacts: resolve("artifacts"), output: resolve("release"),
    version: pkg.version, tag: process.env.RELEASE_TAG, publicKey: config.plugins.updater.pubkey,
  });
  console.log("Prepared verified installers, signed update packages, latest.json and SHA256SUMS.");
}
