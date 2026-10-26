import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "../../lib/geometry";
import type { DocumentInfo } from "../../lib/types";
import { formDesignerBridge } from "./bridge";
import { assertSnapshot, designChanges, editDesignerDraft, newDesignerDraft } from "./model";
import type { ControlKind, DesignerDraft, DesignerSnapshot } from "./types";

export function useFormDesigner(info: DocumentInfo | null, enabled: boolean) {
  const [snapshot, setSnapshot] = useState<DesignerSnapshot | null>(null);
  const [draft, setDraft] = useState<DesignerDraft | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const latest = useRef({ info, snapshot, draft });
  latest.current = { info, snapshot, draft };
  const inspectedReload = useRef(0);
  const update = useCallback((next: DesignerDraft | null) => {
    latest.current.draft = next;
    setDraft(next);
  }, []);
  const retry = useCallback(() => setReload((value) => value + 1), []);
  const pending = useCallback(() => Boolean(latest.current.draft?.dirty), []);
  const discard = useCallback(() => { update(null); setError(null); }, [update]);
  const choose = useCallback((kind: ControlKind, page: number) => {
    try {
      const { info, snapshot, draft } = latest.current;
      if (!info) throw new Error("Open a PDF first.");
      assertSnapshot(info, snapshot);
      if (!snapshot.designer.authorable) throw new Error(snapshot.designer.reason ?? "General editing is not authorized.");
      if (draft?.dirty) throw new Error("Apply or discard this form-design draft before creating another field.");
      update(newDesignerDraft(info, page, kind, snapshot));
      setError(null);
    } catch (error) { setError(errorMessage(error)); }
  }, [update]);
  const select = useCallback((id: string) => {
    try {
      const { info, snapshot, draft } = latest.current;
      if (draft?.fieldId === id) return;
      if (draft?.dirty) throw new Error("Apply or discard this form-design draft before selecting another logical field.");
      if (!info || !snapshot) throw new Error("Wait for the designer inspection.");
      update(editDesignerDraft(info, snapshot, id));
      setError(null);
    } catch (error) { setError(errorMessage(error)); }
  }, [update]);
  const changes = useCallback((info: DocumentInfo) => designChanges(info, latest.current.snapshot, latest.current.draft), []);
  useEffect(() => {
    if (!info || !enabled && !draft?.dirty) return;
    if (latest.current.draft?.dirty && (latest.current.draft.documentId !== info.id || latest.current.draft.revision !== info.revision)) return;
    if (snapshot?.documentId === info.id && snapshot.revision === info.revision && inspectedReload.current === reload) return;
    let active = true;
    setLoading(true); setError(null);
    void formDesignerBridge.inspect(info).then((designer) => {
      if (!active) return;
      inspectedReload.current = reload;
      setSnapshot({ documentId: info.id, revision: info.revision, designer });
      setLoading(false);
      if (latest.current.draft && !latest.current.draft.dirty) update(null);
    }).catch((error: unknown) => {
      if (active) { setError(errorMessage(error)); setLoading(false); }
    });
    return () => { active = false; };
  }, [info?.id, info?.revision, enabled, Boolean(draft?.dirty), reload, update]);
  const stale = Boolean(draft && (draft.documentId !== info?.id || draft.revision !== info?.revision));
  const designer = snapshot && snapshot.documentId === info?.id && (snapshot.revision === info?.revision || stale) ? snapshot.designer : null;
  return { designer, draft, loading, error, stale, retry, pending, discard, choose, select, changes,
    change: (next: DesignerDraft) => update({ ...next, dirty: true }) };
}
