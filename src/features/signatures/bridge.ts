import { invoke } from "@tauri-apps/api/core";
import { downloadPdf, native, post, previewJson, previewResponse, revision } from "../../lib/bridge";
import type { DocumentInfo } from "../../lib/types";
import { signedCopyName, validateIdentityFile } from "./types";
import type { SignatureReport, SigningIdentity, SigningIdentityFile } from "./types";

export const signatureBridge = {
  inspect(info: DocumentInfo): Promise<SignatureReport> {
    return native ? invoke("inspect_signatures", revision(info))
      : previewJson("/signatures/inspect", post(revision(info)));
  },
  chooseIdentity(info: DocumentInfo): Promise<SigningIdentityFile | null> {
    if (!native) throw new Error("Use the browser's local identity file picker.");
    return invoke("choose_signing_identity", revision(info));
  },
  async export(info: DocumentInfo, identity: SigningIdentity, password: string): Promise<string | null> {
    if (native) {
      if (identity.kind !== "native") throw new Error("Choose a local PKCS#12 identity in the desktop application.");
      return invoke("export_signed_copy", { ...revision(info), identityPath: identity.file.path, password });
    }
    if (identity.kind !== "browser") throw new Error("Choose an identity file in this browser preview.");
    validateIdentityFile(identity.file);
    const bytes = new Uint8Array(await identity.file.arrayBuffer());
    try {
      const response = await previewResponse("/signatures/export", post({
        ...revision(info), identity: Array.from(bytes), password,
      }));
      const name = signedCopyName(info.name);
      downloadPdf(new Uint8Array(await response.arrayBuffer()), name);
      return name;
    } finally {
      bytes.fill(0);
    }
  },
};
