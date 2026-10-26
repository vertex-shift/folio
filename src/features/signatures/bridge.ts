import { invoke } from "@tauri-apps/api/core";
import { downloadPdf, native, post, previewJson, previewResponse, revision } from "../../lib/bridge";
import type { DocumentInfo } from "../../lib/types";
import { signedCopyName, validateAnchorFile, validateIdentityFile } from "./types";
import type { SignatureReport, SignatureTrustStore, SigningIdentity, SigningIdentityFile } from "./types";
import { signatureTrustState } from "./trustState";

export const signatureBridge = {
  async inspect(info: DocumentInfo): Promise<SignatureReport> {
    const report = await (native ? invoke<SignatureReport>("inspect_signatures", revision(info))
      : previewJson<SignatureReport>("/signatures/inspect", post(revision(info))));
    if (!signatureTrustState.accept(report.trustStore.revision)) {
      throw new Error("The custom CA store changed during these checks. Refresh signature checks.");
    }
    return report;
  },
  chooseIdentity(info: DocumentInfo): Promise<SigningIdentityFile | null> {
    if (!native) throw new Error("Use the browser's local identity file picker.");
    return invoke("choose_signing_identity", revision(info));
  },
  async addAnchor(info: DocumentInfo, trustRevision: number, file?: File): Promise<SignatureTrustStore | null> {
    let store: SignatureTrustStore | null;
    if (native) {
      store = await invoke("choose_signature_anchor", { ...revision(info), trustRevision });
    } else {
      if (!file) return null;
      validateAnchorFile(file);
      const bytes = new Uint8Array(await file.arrayBuffer());
      try {
        store = await previewJson("/signatures/anchors/add", post({
          ...revision(info), trustRevision, certificate: Array.from(bytes),
        }));
      } finally { bytes.fill(0); }
    }
    if (store && !signatureTrustState.accept(store.revision)) throw new Error("The custom CA store changed. Refresh signature checks.");
    return store;
  },
  async removeAnchor(info: DocumentInfo, trustRevision: number, fingerprint: string): Promise<SignatureTrustStore> {
    const request = { ...revision(info), trustRevision, fingerprint };
    const store = await (native ? invoke<SignatureTrustStore>("remove_signature_anchor", request)
      : previewJson<SignatureTrustStore>("/signatures/anchors/remove", post(request)));
    if (!signatureTrustState.accept(store.revision)) throw new Error("The custom CA store changed. Refresh signature checks.");
    return store;
  },
  async export(info: DocumentInfo, identity: SigningIdentity, password: string, fieldId: string | null = null): Promise<string | null> {
    if (native) {
      if (identity.kind !== "native") throw new Error("Choose a local PKCS#12 identity in the desktop application.");
      return invoke("export_signed_copy", { ...revision(info), identityPath: identity.file.path, password, fieldId });
    }
    if (identity.kind !== "browser") throw new Error("Choose an identity file in this browser preview.");
    validateIdentityFile(identity.file);
    const bytes = new Uint8Array(await identity.file.arrayBuffer());
    try {
      const response = await previewResponse("/signatures/export", post({
        ...revision(info), identity: Array.from(bytes), password, fieldId,
      }));
      const name = signedCopyName(info.name);
      downloadPdf(new Uint8Array(await response.arrayBuffer()), name);
      return name;
    } finally {
      bytes.fill(0);
    }
  },
};
