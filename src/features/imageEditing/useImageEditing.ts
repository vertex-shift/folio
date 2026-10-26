import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "../../lib/geometry";
import type { DocumentInfo } from "../../lib/types";
import { imageEditingBridge } from "./bridge";
import { assertImageDraft, imageDraftPending, imagePath, newImageDraft } from "./model";
import type { ImageEditDraft, ImageInspection, ImageOccurrence } from "./types";

const emptyInspection: ImageInspection = { loading: false, result: null, error: null };

export function useImageEditing(info: DocumentInfo | null, page: number, enabled: boolean) {
  const [draft, setDraft] = useState<ImageEditDraft | null>(null);
  const [inspections, setInspections] = useState<Record<string, ImageInspection>>({});
  const requests = useRef(new Map<string, Promise<void>>());
  const picker = useRef<ImageEditDraft | null>(null);
  const latest = useRef({ info, draft });
  latest.current = { info, draft };
  const mounted = useRef(true);
  const source = useRef({ id: info?.id, revision: info?.revision, epoch: 0 });
  if (source.current.id !== info?.id || source.current.revision !== info?.revision) {
    source.current = { id: info?.id, revision: info?.revision, epoch: source.current.epoch + 1 };
  }
  const prefix = `${info?.id}:${info?.revision}:${source.current.epoch}:`;
  const prefixRef = useRef(prefix);
  prefixRef.current = prefix;
  const update = useCallback((value: ImageEditDraft | null) => {
    latest.current.draft = value;
    setDraft(value);
  }, []);
  const pending = useCallback(() => imageDraftPending(latest.current.draft), []);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    setInspections({});
    if (!pending()) update(null);
  }, [info?.id, info?.revision, pending, update]);
  const previewUrl = draft?.replacement?.previewUrl;
  useEffect(() => () => { if (previewUrl) URL.revokeObjectURL(previewUrl); }, [previewUrl]);

  const inspectPage = useCallback((page: number, retry = false): Promise<void> => {
    const info = latest.current.info;
    if (!info) return Promise.resolve();
    const prefix = prefixRef.current, key = `${prefix}${page}`;
    const existing = requests.current.get(key);
    if (!retry && existing) return existing;
    setInspections((previous) => ({ ...previous, [key]: { loading: true, result: null, error: null } }));
    const request = imageEditingBridge.inspect(info, page).then((result) => {
      if (mounted.current && prefixRef.current === prefix) {
        setInspections((previous) => ({ ...previous, [key]: { loading: false, result, error: null } }));
      }
    }).catch((error: unknown) => {
      if (mounted.current && prefixRef.current === prefix) {
        setInspections((previous) => ({ ...previous, [key]: { loading: false, result: null, error: errorMessage(error) } }));
      }
      requests.current.delete(key);
    });
    requests.current.set(key, request);
    return request;
  }, []);
  useEffect(() => { if (enabled) void inspectPage(page); }, [info?.id, info?.revision, page, enabled, inspectPage]);

  function select(page: number, image: ImageOccurrence) {
    const { info, draft } = latest.current;
    if (!info) throw new Error("Open a PDF first.");
    if (draft && imageDraftPending(draft)) {
      assertImageDraft(info, draft);
      if (draft.page === page && imagePath(draft.source.path) === imagePath(image.path)) return;
      throw new Error("Apply or discard the current image draft before selecting another occurrence.");
    }
    update(newImageDraft(info, page, image));
  }
  function prepareReplacement() {
    const { info, draft } = latest.current;
    if (!info) throw new Error("Open a PDF first.");
    assertImageDraft(info, draft);
    if (!draft.source.editable) throw new Error(draft.source.reason ?? "This image is read-only.");
    if (draft.remove) throw new Error("Choose Keep image before selecting a replacement.");
    picker.current = draft;
  }
  async function stage(file: File) {
    const requested = picker.current;
    picker.current = null;
    const { info, draft } = latest.current;
    if (!info || !requested || !draft || requested.page !== draft.page || imagePath(requested.source.path) !== imagePath(draft.source.path)) {
      throw new Error("The selected image changed while the picker was open. Choose a replacement again.");
    }
    assertImageDraft(info, requested);
    const asset = await imageEditingBridge.stage(info, file);
    const current = latest.current;
    if (!current.info || current.info.id !== info.id || current.info.revision !== info.revision
      || current.draft !== draft) {
      await imageEditingBridge.cancel(info.id, asset.assetId);
      throw new Error("The image selection changed during preparation. No source pixels were changed; choose the replacement again.");
    }
    const previewUrl = URL.createObjectURL(new Blob([new Uint8Array(asset.previewPng)], { type: "image/png" }));
    update({ ...draft, replacement: { assetId: asset.assetId, width: asset.width, height: asset.height, name: file.name, previewUrl } });
  }
  async function discard() {
    const draft = latest.current.draft;
    if (draft?.replacement) await imageEditingBridge.cancel(draft.documentId, draft.replacement.assetId);
    update(null);
  }
  return {
    draft, pending: imageDraftPending(draft), hasDraft: pending, select, change: update,
    prepareReplacement, stage, discard, clear: () => update(null), inspectPage,
    inspection: (target: number) => inspections[`${prefix}${target}`] ?? emptyInspection,
    retry: () => { void inspectPage(page, true); },
    stale: Boolean(draft && (draft.documentId !== info?.id || draft.revision !== info?.revision)),
  };
}
