import { useEffect, useRef, useState } from "react";
import { bridge } from "./bridge";
import { errorMessage } from "./geometry";
import {
  assertAuthoringRevision, authoringEdit, authoringPending, createAuthoringDraft,
  decimal, fittedImage, imagePlacement,
} from "./authoring";
import type { AuthoringAction, AuthoringDraft, AuthoringKind } from "./authoring";
import type { DocumentInfo, Point } from "./types";

interface Options {
  info: DocumentInfo | null;
  page: number;
  current(): DocumentInfo | null;
  blocked(): boolean;
  run(label: string, action: () => Promise<void>): Promise<void>;
  adopt(info: DocumentInfo): Promise<void>;
  onError(message: string): void;
  onToast(message: string): void;
  onSelect(kind: AuthoringKind): void;
  onDraftCleared(): Promise<void>;
}

export function useAuthoring(options: Options) {
  const callbacks = useRef(options);
  callbacks.current = options;
  const [draft, setDraft] = useState<AuthoringDraft | null>(null);
  const draftRef = useRef(draft);
  draftRef.current = draft;
  function update(next: AuthoringDraft | null) {
    draftRef.current = next;
    setDraft(next);
  }

  useEffect(() => {
    const previous = draftRef.current, info = callbacks.current.info;
    if (previous && info && !authoringPending(previous)) {
      update(createAuthoringDraft(previous.kind, info, callbacks.current.page));
    }
  }, [options.info?.id, options.info?.revision, options.page]);

  const previewUrl = draft?.kind === "image" ? draft.previewUrl : null;
  useEffect(() => () => { if (previewUrl) URL.revokeObjectURL(previewUrl); }, [previewUrl]);

  function document(): DocumentInfo {
    const info = callbacks.current.current();
    if (!info) throw new Error("Open a PDF before using Page authoring.");
    if (info.readOnlyReason) throw new Error(info.readOnlyReason);
    if (callbacks.current.blocked()) throw new Error("Finish the current action and clear pending redaction marks before page authoring.");
    return info;
  }

  function author(label: string, action: (info: DocumentInfo) => Promise<void>) {
    try {
      const info = document();
      void callbacks.current.run(label, () => action(info));
    } catch (error) { callbacks.current.onError(errorMessage(error)); }
  }

  function choose(kind: AuthoringKind) {
    try {
      const info = document(), previous = draftRef.current;
      if (previous && authoringPending(previous)) {
        assertAuthoringRevision(previous, info);
        if (previous.kind !== kind) throw new Error("Apply or discard this authoring draft before switching authoring tools.");
      } else update(createAuthoringDraft(kind, info, callbacks.current.page));
      callbacks.current.onSelect(kind);
    } catch (error) { callbacks.current.onError(errorMessage(error)); }
  }

  function stage(file: File) {
    author("Preparing selected image...", async (info) => {
      const previous = draftRef.current;
      if (previous) assertAuthoringRevision(previous, info);
      if (previous && previous.kind !== "image" && authoringPending(previous)) {
        throw new Error("Apply or discard the current authoring draft before choosing an image.");
      }
      const asset = await bridge.stageImage(info, file);
      const base = previous?.kind === "image" ? previous : createAuthoringDraft("image", info, callbacks.current.page);
      if (base.kind !== "image") throw new Error("The image authoring tool is unavailable.");
      const previewUrl = URL.createObjectURL(new Blob([new Uint8Array(asset.previewPng)], { type: "image/png" }));
      update(fittedImage({ ...base, asset: { assetId: asset.assetId, width: asset.width, height: asset.height },
        name: file.name, previewUrl }, info.pages[base.page].geometry));
      callbacks.current.onSelect("image");
      callbacks.current.onToast("Image prepared. Position it, then choose Insert image.");
    });
  }

  function apply(action: AuthoringAction = "apply") {
    author("Applying page authoring...", async (info) => {
      const current = draftRef.current;
      if (!current) throw new Error("Choose a page-authoring tool first.");
      assertAuthoringRevision(current, info);
      let next: DocumentInfo;
      if (current.kind === "image") {
        if (!current.asset) throw new Error("Choose a PNG or JPEG before inserting an image.");
        next = await bridge.insertImage(info, current.asset.assetId, imagePlacement(current, info.pages[current.page].geometry));
      } else next = await bridge.edit(info, authoringEdit(current, info, action));
      update(createAuthoringDraft(current.kind, next, current.page));
      await callbacks.current.onDraftCleared();
      await callbacks.current.adopt(next);
      callbacks.current.onToast("Page authoring applied to the PDF. Undo is available; save a copy to keep it.");
    });
  }

  function discard() {
    void callbacks.current.run("Discarding authoring draft...", async () => {
      const info = callbacks.current.current(), current = draftRef.current;
      if (!info || !current) throw new Error("This authoring draft is no longer available.");
      if (current.kind === "image" && current.asset) await bridge.cancelImage(current.documentId, current.asset.assetId);
      update(createAuthoringDraft(current.kind, info, callbacks.current.page));
      await callbacks.current.onDraftCleared();
      callbacks.current.onToast("Authoring draft discarded. The PDF was not changed.");
    });
  }

  function place(page: number, point: Point) {
    try {
      const info = document(), current = draftRef.current;
      if (!current || current.kind !== "image" || !current.asset) throw new Error("Choose an image first.");
      assertAuthoringRevision(current, info);
      const geometry = info.pages[page]?.geometry;
      if (!geometry) throw new Error("The selected page is no longer available.");
      const width = Number(current.width), height = Number(current.height);
      if (!Number.isFinite(width) || !Number.isFinite(height) || width < 0.1 || height < 0.1
        || width > geometry.widthPoints || height > geometry.heightPoints) {
        throw new Error("Reduce the image dimensions to fit this visible page before placing it.");
      }
      update({ ...current, page, dirty: true,
        x: decimal(Math.max(0, Math.min(point[0], geometry.widthPoints - width))),
        y: decimal(Math.max(0, Math.min(point[1], geometry.heightPoints - height))) });
    } catch (error) { callbacks.current.onError(errorMessage(error)); }
  }

  function crop(page: number, margins: [number, number, number, number]) {
    try {
      const info = document(), current = draftRef.current;
      if (!current || current.kind !== "crop") throw new Error("Choose Crop in Page authoring first.");
      assertAuthoringRevision(current, info);
      update({ ...current, page, dirty: true,
        margins: [decimal(margins[0]), decimal(margins[1]), decimal(margins[2]), decimal(margins[3])] });
    } catch (error) { callbacks.current.onError(errorMessage(error)); }
  }

  return { draft, pending: authoringPending(draft), hasDraft: () => authoringPending(draftRef.current),
    change: update, choose, stage, apply, discard, place, crop };
}
