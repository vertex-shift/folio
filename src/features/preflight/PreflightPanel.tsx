import { useEffect, useRef, useState } from "react";
import { ClipboardCheck, FileDown, LoaderCircle } from "lucide-react";
import { errorMessage } from "../../lib/geometry";
import type { DocumentInfo } from "../../lib/types";
import { assertTechnicalCurrent } from "./model";
import type { ObservationState, TechnicalReport } from "./model";
import { preflightBridge } from "./bridge";

interface Props {
  info: DocumentInfo;
  pending: boolean;
  disabled: boolean;
  current(): DocumentInfo | null;
  hasPending(): boolean;
  prepare(): Promise<DocumentInfo>;
  run(label: string, task: () => Promise<void>): Promise<void>;
  onError(message: string): void;
  onNotice(message: string): void;
  onArchive(): void;
}

export default function PreflightPanel(props: Props) {
  const [report, setReport] = useState<TechnicalReport | null>(null);
  const [filter, setFilter] = useState<"all" | ObservationState>("all");
  const [acknowledged, setAcknowledged] = useState(false);
  const [inspecting, setInspecting] = useState(false);
  const latest = useRef(props); latest.current = props;
  const retained = useRef<TechnicalReport | null>(null);
  const generation = useRef(0);
  const controller = useRef<AbortController | null>(null);

  function release() {
    generation.current += 1;
    controller.current?.abort();
    const old = retained.current;
    retained.current = null;
    if (old) void preflightBridge.cancel(old.inspectionId).catch((error: unknown) => {
      latest.current.onError(`Technical report cleanup failed: ${errorMessage(error)}`);
    });
  }

  useEffect(() => {
    release(); setReport(null); setAcknowledged(false);
    return release;
  }, [props.info.id, props.info.revision, props.pending]);

  function inspect() {
    void props.run("Inspecting technical PDF properties...", async () => {
      const info = await props.prepare();
      release(); setReport(null); setInspecting(true);
      const token = generation.current;
      let next: TechnicalReport | undefined;
      try {
        next = await preflightBridge.inspect({ documentId: info.id, revision: info.revision });
        if (token !== generation.current) throw new Error("The inspection was canceled or its source changed. Inspect the current PDF again.");
        assertTechnicalCurrent(next, latest.current.current(), latest.current.hasPending());
        retained.current = next;
        setReport(next);
      } catch (error) {
        if (next) await preflightBridge.cancel(next.inspectionId);
        throw error;
      } finally { setInspecting(false); }
    });
  }

  function save() {
    void props.run("Exporting technical report...", async () => {
      const current = retained.current;
      if (!current) throw new Error("Inspect the current PDF before exporting a report.");
      const abort = new AbortController(); controller.current = abort;
      const check = () => assertTechnicalCurrent(current, latest.current.current(), latest.current.hasPending());
      try {
        check();
        await preflightBridge.check(current.inspectionId);
        const output = await preflightBridge.save(current, acknowledged, abort.signal, check);
        props.onNotice(output ? "Technical report exported separately. Input conformance is not validated; the source PDF is unchanged." : "Report export canceled. The source PDF is unchanged.");
      } finally { controller.current = null; }
    });
  }

  const shown = report?.findings.filter((finding) => filter === "all" || finding.state === filter);
  return <div className="preflight-panel panel-scroll">
    <p>Inspect concrete properties of this working PDF, including applied unsaved edits.</p>
    <p className="preflight-boundary"><strong>Technical observations, not certification.</strong> No full accessibility, PDF/UA or arbitrary-input PDF/A validation.</p>
    <div className="preflight-actions">
      <button className="button primary" disabled={props.disabled || props.pending || !props.info.copyAllowed} onClick={inspect}>
        {inspecting ? <LoaderCircle size={15} className="spin" /> : <ClipboardCheck size={15} />}Inspect current PDF
      </button>
      <button className="button secondary" disabled={props.disabled || props.pending} onClick={props.onArchive}>Create archival copy...</button>
    </div>
    {props.pending && <p className="inline-error" role="status">Apply or discard pending drafts and redaction marks before inspecting.</p>}
    {!props.info.copyAllowed && <p className="inline-error">This PDF restricts copying. Reopen with its owner password.</p>}
    {!report && <p className="muted">Fonts and embedding-stream presence, page geometry, image declarations, forms/active keys, language/tags and existing conformance declarations. The original is never rewritten.</p>}
    {report && <>
      <div className="preflight-report-source"><strong>{report.name}</strong><span>Revision {report.revision} · {report.findings.length} observations</span></div>
      <p className="preflight-boundary">{report.inputConformance}</p>
      <label className="preflight-filter">Show observations<select value={filter} disabled={props.disabled}
        onChange={(event) => {
          const value = event.target.value;
          if (value === "all" || value === "fact" || value === "absent" || value === "unsupported" || value === "unchecked") setFilter(value);
        }}>
        <option value="all">All observations</option><option value="fact">Observed facts</option><option value="absent">Absence in scope</option>
        <option value="unsupported">Unsupported inspection</option><option value="unchecked">Not checked</option>
      </select></label>
      <p className="muted">Fact = observed entry, not a pass. Absent = not found in the stated scope. Unsupported = could not inspect reliably. Unchecked = outside this method.</p>
      <ol className="preflight-findings" aria-label="Technical observations">
        {shown?.map((finding, index) => <li key={`${finding.property}-${index}`}>
          <div><strong>{finding.property}</strong><span className={`observation-state state-${finding.state}`}>{finding.state}</span></div>
          <p>{finding.detail}</p>
        </li>)}
      </ol>
      {!shown?.length && <p className="muted">No observations in this category.</p>}
      <details className="preflight-scope"><summary>Exact inspection boundary</summary><p>{report.scope}</p></details>
      {report.passwordProtected && <div className="output-disclosure">
        <p>This JSON report discloses unencrypted document properties. PDF passwords, permissions and signatures do not carry over.</p>
        <label className="checkbox-setting"><input type="checkbox" checked={acknowledged} disabled={props.disabled}
          onChange={(event) => setAcknowledged(event.target.checked)} />Export these properties unencrypted.</label>
      </div>}
      <button className="button secondary" disabled={props.disabled || props.pending || report.passwordProtected && !acknowledged} onClick={save}>
        <FileDown size={15} />Export JSON report
      </button>
    </>}
  </div>;
}
