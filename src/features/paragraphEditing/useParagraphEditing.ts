import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "../../lib/geometry";
import type { DocumentInfo } from "../../lib/types";
import { paragraphBridge } from "./bridge";
import { assertParagraphDraft, newParagraphDraft, paragraphChange, paragraphDraftPending, paragraphPath } from "./model";
import type { ParagraphBlock, ParagraphDraft, ParagraphInspection, ParagraphPreviewState } from "./types";

const emptyInspection: ParagraphInspection = { loading: false, result: null, error: null };
const emptyPreview: ParagraphPreviewState = { loading: false, result: null, error: null };

export function useParagraphEditing(info: DocumentInfo | null, page: number, enabled: boolean, active: boolean) {
  const [draft, setDraft] = useState<ParagraphDraft | null>(null);
  const [inspections, setInspections] = useState<Record<string, ParagraphInspection>>({});
  const [preview, setPreview] = useState<ParagraphPreviewState & { key: string }>({ ...emptyPreview, key: "" });
  const requests = useRef(new Map<string, Promise<void>>());
  const cache = useRef<string[]>([]);
  const mounted = useRef(true);
  const latest = useRef({ info, draft });
  latest.current = { info, draft };
  const source = useRef({ id: info?.id, revision: info?.revision, generation: 0 });
  if (source.current.id !== info?.id || source.current.revision !== info?.revision) {
    source.current = { id: info?.id, revision: info?.revision, generation: source.current.generation + 1 };
  }
  const prefix = `${info?.id}:${info?.revision}:${source.current.generation}:`;
  const prefixRef = useRef(prefix);
  prefixRef.current = prefix;
  const update = useCallback((draft: ParagraphDraft | null) => {
    latest.current.draft = draft;
    setDraft(draft);
  }, []);
  const hasDraft = useCallback(() => paragraphDraftPending(latest.current.draft), []);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    requests.current.clear(); cache.current = []; setInspections({});
    if (!hasDraft()) update(null);
  }, [info?.id, info?.revision, hasDraft, update]);

  const inspectPage = useCallback((page: number, retry = false): Promise<void> => {
    const info = latest.current.info;
    if (!info) return Promise.resolve();
    const prefix = prefixRef.current, key = `${prefix}${page}`;
    cache.current = [...cache.current.filter((item) => item !== key), key].slice(-8);
    for (const previous of requests.current.keys()) {
      if (!cache.current.includes(previous)) requests.current.delete(previous);
    }
    const previous = requests.current.get(key);
    if (!retry && previous) return previous;
    setInspections((previous) => ({ ...Object.fromEntries(Object.entries(previous).filter(([key]) => cache.current.includes(key))),
      [key]: { loading: true, result: null, error: null } }));
    const request = paragraphBridge.inspect(info, page).then((result) => {
      if (mounted.current && prefixRef.current === prefix && requests.current.get(key) === request && cache.current.includes(key)) {
        setInspections((previous) => ({ ...previous, [key]: { loading: false, result, error: null } }));
      }
    }).catch((error: unknown) => {
      if (mounted.current && prefixRef.current === prefix && requests.current.get(key) === request && cache.current.includes(key)) {
        setInspections((previous) => ({ ...previous, [key]: { loading: false, result: null, error: errorMessage(error) } }));
      }
      if (requests.current.get(key) === request) requests.current.delete(key);
    });
    requests.current.set(key, request);
    return request;
  }, []);
  useEffect(() => { if (enabled && active) void inspectPage(page); }, [info?.id, info?.revision, page, enabled, active, inspectPage]);

  let previewKey = "", validationError: string | null = null;
  if (info && draft?.source.editable) {
    try { previewKey = JSON.stringify({ ...paragraphChange(info, draft), generation: source.current.generation }); }
    catch (error) { validationError = errorMessage(error); }
  }
  useEffect(() => {
    if (!active || !previewKey) return;
    let current = true;
    const { info, draft } = latest.current;
    if (!info || !draft) return;
    const timeout = window.setTimeout(() => {
      setPreview({ key: previewKey, loading: true, result: null, error: null });
      void paragraphBridge.preview(info, paragraphChange(info, draft)).then((result) => {
        if (current) setPreview({ key: previewKey, loading: false, result, error: null });
      }).catch((error: unknown) => {
        if (current) setPreview({ key: previewKey, loading: false, result: null, error: errorMessage(error) });
      });
    }, 180);
    return () => { current = false; window.clearTimeout(timeout); };
  }, [previewKey, active]);

  function select(target: number, block: ParagraphBlock) {
    const { info, draft } = latest.current;
    if (!info) throw new Error("Open a PDF first.");
    if (draft && paragraphDraftPending(draft)) {
      assertParagraphDraft(info, draft);
      if (draft.page === target && paragraphPath(draft.source.path) === paragraphPath(block.path)) return;
      throw new Error("Apply or discard the current paragraph draft before selecting another source block.");
    }
    const result = inspections[`${prefixRef.current}${target}`]?.result;
    if (!result) throw new Error("Wait for paragraph inspection to finish, then select the source block again.");
    update(newParagraphDraft(info, target, result.epoch, block));
  }
  const inspection = (target: number) => inspections[`${prefix}${target}`] ?? emptyInspection;
  const inspectedEpoch = draft ? inspection(draft.page).result?.epoch : undefined;
  const stale = Boolean(draft && (draft.documentId !== info?.id || draft.revision !== info?.revision
    || inspectedEpoch !== undefined && draft.epoch !== inspectedEpoch));
  return {
    draft, change: update, clear: () => update(null), select, hasDraft, pending: paragraphDraftPending(draft),
    inspectPage, inspection, retry: () => { void inspectPage(page, true); }, stale,
    preview: validationError ? { ...emptyPreview, error: validationError } : !previewKey ? emptyPreview
      : previewKey && preview.key !== previewKey ? { ...emptyPreview, loading: true } : preview,
  };
}
