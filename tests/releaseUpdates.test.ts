import test from "node:test";
import assert from "node:assert/strict";
import { generateKeyPairSync, randomBytes, sign } from "node:crypto";
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { blake2b } from "@noble/hashes/blake2.js";
import { prepareRelease, verifyUpdate } from "../scripts/prepare-release.mjs";

function signer() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const id = randomBytes(8);
  const publicBytes = publicKey.export({ format: "der", type: "spki" }).subarray(-32);
  const key = Buffer.from(`untrusted comment: synthetic test key\n${Buffer.concat([Buffer.from("Ed"), id, publicBytes]).toString("base64")}\n`).toString("base64");
  return {
    key,
    signature(bytes: Buffer, version = "0.2.0") {
      const signature = sign(null, blake2b(bytes), privateKey);
      const comment = `timestamp:1\tfile:synthetic\tversion:${version}`;
      const global = sign(null, Buffer.concat([signature, Buffer.from(comment)]), privateKey);
      return Buffer.from(`untrusted comment: synthetic\n${Buffer.concat([Buffer.from("ED"), id, signature]).toString("base64")}\ntrusted comment: ${comment}\n${global.toString("base64")}\n`).toString("base64");
    },
  };
}

test("release verification rejects tampered packages, keys, comments and unsigned version substitution", () => {
  const identity = signer();
  const bytes = Buffer.from("synthetic installer bytes");
  const signature = identity.signature(bytes);
  verifyUpdate(bytes, signature, identity.key, "0.2.0");
  assert.throws(() => verifyUpdate(Buffer.from("tampered"), signature, identity.key, "0.2.0"), /signature/);
  assert.throws(() => verifyUpdate(bytes, signature, signer().key, "0.2.0"), /key/);
  assert.throws(() => verifyUpdate(bytes, signature, identity.key, "9.0.0"), /version/);
  const alteredComment = Buffer.from(Buffer.from(signature, "base64").toString().replace("version:0.2.0", "version:9.0.0")).toString("base64");
  assert.throws(() => verifyUpdate(bytes, alteredComment, identity.key, "9.0.0"), /signature/);
  assert.throws(() => verifyUpdate(bytes, "not a signature", identity.key, "0.2.0"), /encoding/);
});

test("a complete release has exact stable platform URLs, detached signatures and checksums", async () => {
  const root = await mkdtemp(join(tmpdir(), "folio-release-test-"));
  try {
    const identity = signer();
    const artifacts = join(root, "artifacts");
    for (const [directory, name, signed] of [
      ["folio-windows-x64", "setup.exe", true],
      ["folio-macos-arm64", "Folio.dmg", false],
      ["folio-macos-arm64", "Folio.app.tar.gz", true],
      ["folio-linux-x64", "folio.deb", false],
      ["folio-linux-x64", "folio.AppImage", true],
    ] as const) {
      await mkdir(join(artifacts, directory), { recursive: true });
      const bytes = Buffer.from(`synthetic ${name}`);
      await writeFile(join(artifacts, directory, name), bytes);
      if (signed) await writeFile(join(artifacts, directory, `${name}.sig`), identity.signature(bytes));
    }
    const options = { artifacts, output: join(root, "release"), version: "0.2.0", tag: "v0.2.0", publicKey: identity.key };
    const manifest = await prepareRelease(options);
    assert.deepEqual(Object.keys(manifest.platforms).sort(), ["darwin-aarch64", "linux-x86_64", "windows-x86_64"]);
    assert.equal(manifest.platforms["windows-x86_64"].url,
      "https://github.com/vertex-shift/folio/releases/download/v0.2.0/Folio_0.2.0_windows_x64_setup.exe");
    const checksumLines = (await readFile(join(root, "release", "SHA256SUMS"), "utf8")).trim().split("\n");
    assert.equal(checksumLines.length, 9);
    assert.equal((await readdir(join(root, "release"))).length, 10);
    await assert.rejects(prepareRelease({ ...options, output: join(root, "bad-tag"), tag: "v9.0.0" }), /tag/);
    await writeFile(join(artifacts, "folio-windows-x64", "extra.exe"), "unexpected");
    await assert.rejects(prepareRelease({ ...options, output: join(root, "duplicate") }), /Expected one/);
    await rm(join(artifacts, "folio-windows-x64", "extra.exe"));
    await writeFile(join(artifacts, "unexpected.txt"), "not publishable");
    await assert.rejects(prepareRelease({ ...options, output: join(root, "extra") }), /Unexpected files/);
    await rm(join(artifacts, "unexpected.txt"));
    await rm(join(artifacts, "folio-windows-x64", "setup.exe.sig"));
    await assert.rejects(prepareRelease({ ...options, output: join(root, "unsigned") }), /Missing signature/);
    await assert.rejects(readdir(join(root, "unsigned")), /ENOENT/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
