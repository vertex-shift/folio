import { useCallback, useEffect, useRef, useState } from "react";
import { bridge } from "./bridge";
import { errorMessage } from "./geometry";
import { formChanges, updateFormDraft } from "./forms";
import type { FormDraft, FormSnapshot, FormValue } from "./formsTypes";
import type { DocumentInfo } from "./types";

export function useForms(info: DocumentInfo | null, enabled: boolean) {
  const [snapshot, setSnapshot] = useState<FormSnapshot | null>(null);
  const [draft, setDraft] = useState<FormDraft | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [reload, setReload] = useState(0);
  const draftRef = useRef(draft);
  const snapshotRef = useRef(snapshot);
  const inspectedReload = useRef(0);
  const current = useRef(info);
  current.current = info;
  snapshotRef.current = snapshot;
  const retry = useCallback(() => setReload((value) => value + 1), []);
  const pending = useCallback(() => draftRef.current !== null, []);
  const discard = useCallback(() => {
    draftRef.current = null;
    setDraft(null); setFieldErrors({}); setError(null);
  }, []);
  const change = useCallback((id: string, value: FormValue) => {
    const source = snapshotRef.current;
    try {
      if (!source || current.current?.id !== source.documentId || current.current.revision !== source.revision) {
        throw new Error("Wait for the current form to load before changing its values.");
      }
      const next = updateFormDraft(source, draftRef.current, id, value);
      draftRef.current = next;
      setDraft(next);
      setError(null);
      setFieldErrors((errors) => {
        if (!(id in errors)) return errors;
        const next = { ...errors }; delete next[id]; return next;
      });
    } catch (error) { setError(errorMessage(error)); }
  }, []);
  const changes = useCallback((info: DocumentInfo) => {
    const result = formChanges(snapshotRef.current, draftRef.current, info);
    setFieldErrors(result.errors);
    if (Object.keys(result.errors).length) throw new Error("Some form values need attention. Review the marked fields; your drafts are kept.");
    return result.changes;
  }, []);
  const count = draft ? Object.keys(draft.values).length : 0;
  useEffect(() => {
    if (!info || !enabled && !count) return;
    if (draftRef.current && (draftRef.current.documentId !== info.id || draftRef.current.revision !== info.revision)) {
      setLoading(false);
      return;
    }
    if (snapshotRef.current?.documentId === info.id && snapshotRef.current.revision === info.revision && inspectedReload.current === reload) return;
    let active = true;
    setLoading(true); setError(null);
    void bridge.forms(info).then((form) => {
      if (active) {
        inspectedReload.current = reload;
        setSnapshot({ documentId: info.id, revision: info.revision, form }); setLoading(false);
      }
    }).catch((error: unknown) => {
      if (active) { setError(errorMessage(error)); setLoading(false); }
    });
    return () => { active = false; };
  }, [info?.id, info?.revision, enabled, reload, count > 0]);
  const stale = Boolean(draft && (draft.documentId !== info?.id || draft.revision !== info.revision));
  const form = snapshot?.documentId === info?.id && (snapshot?.revision === info?.revision || stale) ? snapshot?.form ?? null : null;
  return { form, draft, count, loading, error, fieldErrors, stale, retry, pending, change, changes, discard };
}
