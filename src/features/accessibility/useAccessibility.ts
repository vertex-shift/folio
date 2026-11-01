import { useEffect, useRef, useState } from "react";
import type { DocumentInfo } from "../../lib/types";
import { errorMessage } from "../../lib/geometry";
import { accessibilityBridge } from "./bridge";
import { assertSemanticCurrent, newSemanticDraft, semanticDraftPending } from "./model";
import type { SemanticCopyPlan, SemanticCopyRequest, SemanticDraft, SemanticInspection } from "./types";

export function useAccessibility(info: DocumentInfo | null, active: boolean, onError: (message: string) => void) {
  const [inspection, setInspection] = useState<SemanticInspection | null>(null);
  const [draft, setDraft] = useState<SemanticDraft | null>(null);
  const [plan, setPlan] = useState<SemanticCopyPlan | null>(null);
  const [readerOpen, setReaderOpen] = useState(false);
  const [working, setWorking] = useState(false);
  const latest = useRef({ info, active, onError, inspection, draft, plan });
  latest.current = { info, active, onError, inspection, draft, plan };
  const controller = useRef<AbortController | null>(null);
  const generation = useRef(0);

  function updateDraft(value: SemanticDraft | null) {
    latest.current.draft = value; setDraft(value);
  }
  function updatePlan(value: SemanticCopyPlan | null) {
    latest.current.plan = value; setPlan(value);
  }
  function cleanupFailure(error: unknown) {
    latest.current.onError(`Accessibility job cleanup failed: ${errorMessage(error)}. Source content is unchanged.`);
  }
  function cancelWork() {
    generation.current += 1;
    controller.current?.abort(new Error("Accessibility operation canceled. Source content is unchanged."));
  }
  useEffect(() => {
    cancelWork();
    const old = latest.current.inspection;
    latest.current.inspection = null; setInspection(null); setReaderOpen(false);
    if (old) void accessibilityBridge.cancelInspection(old.inspectionId).catch(cleanupFailure);
    const job = latest.current.plan;
    updatePlan(null);
    if (job) void accessibilityBridge.cancel(job.jobId).catch(cleanupFailure);
    if (latest.current.draft?.prepared) updateDraft(null);
    return cancelWork;
  }, [info?.id, info?.revision]);
  useEffect(() => { if (!active && controller.current) cancelWork(); }, [active]);
  useEffect(() => () => {
    cancelWork();
    const { inspection, plan } = latest.current;
    if (inspection) void accessibilityBridge.cancelInspection(inspection.inspectionId).catch(cleanupFailure);
    if (plan) void accessibilityBridge.cancel(plan.jobId).catch(cleanupFailure);
  }, []);

  function startWork() {
    cancelWork();
    const abort = new AbortController(); controller.current = abort; setWorking(true);
    const token = generation.current;
    const check = () => {
      abort.signal.throwIfAborted();
      if (token !== generation.current || !latest.current.active) throw new Error("This accessibility operation was canceled or its tab is no longer active.");
    };
    return { abort, check };
  }

  async function inspect(source: DocumentInfo, pending: () => boolean) {
    const { abort, check } = startWork();
    let result: SemanticInspection | undefined;
    try {
      result = await accessibilityBridge.inspect({ documentId: source.id, revision: source.revision }, abort.signal);
      check(); assertSemanticCurrent(result, latest.current.info, pending());
      const old = latest.current.inspection;
      if (old) await accessibilityBridge.cancelInspection(old.inspectionId);
      latest.current.inspection = result; setInspection(result);
    } catch (error) {
      if (result) await accessibilityBridge.cancelInspection(result.inspectionId);
      throw error;
    } finally { controller.current = null; setWorking(false); }
  }

  async function startDraft(pending: () => boolean) {
    const report = latest.current.inspection;
    if (!report) throw new Error("Inspect this PDF before reviewing its source candidates.");
    const { check } = startWork();
    try {
      check(); assertSemanticCurrent(report, latest.current.info, pending());
      await accessibilityBridge.checkInspection(report.inspectionId);
      check(); assertSemanticCurrent(report, latest.current.info, pending());
      updateDraft(newSemanticDraft(report));
      setReaderOpen(false);
    } finally { controller.current = null; setWorking(false); }
  }

  async function begin(request: SemanticCopyRequest, checkSource: () => void) {
    const { abort, check } = startWork();
    const original = latest.current.draft;
    let next: SemanticCopyPlan | undefined;
    try {
      checkSource();
      next = await accessibilityBridge.begin(request, abort.signal);
      check(); checkSource();
      if (latest.current.draft !== original || !original) throw new Error("The reviewed semantic recipe changed. Prepare it again.");
      updateDraft({ ...original, prepared: true });
      updatePlan(next);
    } catch (error) {
      if (next) await accessibilityBridge.cancel(next.jobId);
      throw error;
    } finally { controller.current = null; setWorking(false); }
  }

  async function save(pending: () => boolean): Promise<string | null> {
    const current = latest.current.plan;
    if (!current) throw new Error("Prepare a reviewed tagged copy before saving.");
    const { abort, check } = startWork();
    const checkSource = () => {
      check(); assertSemanticCurrent(current, latest.current.info, pending());
      if (latest.current.plan?.jobId !== current.jobId) throw new Error("The prepared tagged copy was canceled or replaced.");
    };
    try {
      checkSource(); await accessibilityBridge.check(current.jobId); checkSource();
      const output = await accessibilityBridge.save(current, abort.signal, checkSource);
      if (output) {
        await accessibilityBridge.cancel(current.jobId);
        updatePlan(null);
      }
      return output;
    } finally { controller.current = null; setWorking(false); }
  }

  async function cancelCopy() {
    cancelWork();
    const old = latest.current.plan;
    if (old) await accessibilityBridge.cancel(old.jobId);
    updatePlan(null);
    const draft = latest.current.draft;
    if (draft) updateDraft({ ...draft, prepared: false });
  }
  async function discard() {
    await cancelCopy(); updateDraft(null);
  }
  async function read(pending: () => boolean) {
    const current = latest.current.inspection;
    if (!current || current.status !== "tagged") throw new Error("Inspect a supported tagged PDF to read its actual logical structure.");
    const { check } = startWork();
    try {
      check(); assertSemanticCurrent(current, latest.current.info, pending());
      await accessibilityBridge.checkInspection(current.inspectionId);
      check(); assertSemanticCurrent(current, latest.current.info, pending());
      setReaderOpen(true);
    } finally { controller.current = null; setWorking(false); }
  }
  return {
    inspection, draft, plan, readerOpen, setReaderOpen, working, inspect, startDraft, begin, save,
    cancelWork, cancelCopy, discard, read,
    change: (draft: SemanticDraft) => updateDraft({ ...draft, prepared: false }),
    hasDraft: () => semanticDraftPending(latest.current.draft),
    hasPlan: () => Boolean(latest.current.plan),
    pending: semanticDraftPending(draft),
    stale: Boolean(draft && (draft.documentId !== info?.id || draft.revision !== info?.revision)),
  };
}
