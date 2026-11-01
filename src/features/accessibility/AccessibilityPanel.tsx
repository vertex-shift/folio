import { useState } from "react";
import { ArrowDown, ArrowUp, BookOpen, FileDown, ListOrdered, LoaderCircle, LocateFixed } from "lucide-react";
import { isSemanticRole, semanticProblems } from "./model";
import type { SemanticCopyPlan, SemanticDraft, SemanticDraftEntry, SemanticInspection } from "./types";

interface Props {
  inspection: SemanticInspection | null;
  draft: SemanticDraft | null;
  plan: SemanticCopyPlan | null;
  disabled: boolean;
  otherPending: boolean;
  working: boolean;
  stale: boolean;
  readerOpen: boolean;
  error: string | null;
  notice: string | null;
  onInspect(): void;
  onStart(): void;
  onChange(draft: SemanticDraft): void;
  onPrepare(): void;
  onSave(): void;
  onCancel(): void;
  onDiscard(): void;
  onRevise(): void;
  onRead(): void;
  onVisual(): void;
  onLocate(page: number): void;
}

export default function AccessibilityPanel(props: Props) {
  const [selectedId, setSelectedId] = useState("");
  const { inspection, draft, plan } = props;
  const selected = draft?.entries.find((entry) => entry.candidate.id === selectedId) ?? draft?.entries[0];
  const selectedIndex = selected && draft ? draft.entries.indexOf(selected) : -1;
  const problems = draft ? semanticProblems(draft) : [];
  const editable = !props.disabled && !props.otherPending && !props.stale && Boolean(draft && !draft.prepared);
  function changeEntry(changes: Partial<SemanticDraftEntry>) {
    if (!draft || !selected || !editable) return;
    props.onChange({ ...draft, reviewed: false, entries: draft.entries.map((entry) => entry === selected ? { ...entry, ...changes } : entry) });
  }
  function move(delta: number) {
    if (!draft || !selected || !editable) return;
    const target = selectedIndex + delta, entries = [...draft.entries];
    if (target < 0 || target >= entries.length) return;
    [entries[selectedIndex], entries[target]] = [entries[target], entries[selectedIndex]];
    props.onChange({ ...draft, entries, reviewed: false });
    setSelectedId(selected.candidate.id);
  }
  return <div className="accessibility-panel">
    <div className="semantic-intro">
      <p>Read actual PDF tags, or review every source item to make a separate tagged copy.</p>
      <p className="semantic-boundary"><strong>Structure, not certification.</strong> Human order, headings, language and meaningful alternatives remain your responsibility. No PDF/UA or WCAG claim.</p>
    </div>
    <div className="panel-scroll semantic-settings">
      {props.otherPending && <p className="semantic-warning" role="status">Apply or discard other drafts and redaction marks first. Your semantic values stay in this tab.</p>}
      {!draft && <button className="button secondary" disabled={props.disabled || props.otherPending} onClick={props.onInspect}>
        {props.working ? <LoaderCircle size={15} className="spin" /> : <ListOrdered size={15} />}Inspect structure and source
      </button>}
      {!inspection && !draft && <p className="muted semantic-empty">No inspection yet. Untagged text is not assigned roles or reading order automatically.</p>}
      {inspection && !draft && <>
        <div className="semantic-source"><strong>{inspection.name}</strong><span>Revision {inspection.revision} · {inspection.status.replaceAll("_", " ")}</span></div>
        {inspection.issues.length > 0 && <div className="semantic-warning" role="status"><strong>{inspection.status === "tagged" ? "Partial reading boundary" : "Authoring unavailable"}</strong>
          <ul>{inspection.issues.map((issue, index) => <li key={index}>{issue}</li>)}</ul></div>}
        {inspection.status === "untagged" && inspection.authoringAllowed && <>
          <p>{inspection.candidates.length} source items need explicit review. Source drawing order is not a reviewed reading order.</p>
          <button className="button primary" disabled={props.disabled || props.otherPending} onClick={props.onStart}>Review source semantics</button>
        </>}
        {inspection.status === "tagged" && <>
          <dl className="semantic-properties"><dt>Title</dt><dd>{inspection.title || "Missing"}</dd><dt>Document language</dt><dd>{inspection.language || "Missing"}</dd>
            <dt>Content associations</dt><dd>{inspection.complete ? "Supported bindings accounted for" : "Partial; see boundary above"}</dd></dl>
          <p>Existing tags are inspect-only. Malformed, custom or ambiguous trees are never replaced.</p>
          <button className="button primary" disabled={props.disabled || props.otherPending} onClick={props.readerOpen ? props.onVisual : props.onRead}><BookOpen size={15} />{props.readerOpen ? "Return to visual PDF" : "Read tagged content"}</button>
          {!props.readerOpen && <details className="semantic-scope"><summary>Inspect roles, order and alternatives</summary>
            <StructureList nodes={inspection.structure} />
          </details>}
        </>}
      </>}
      {draft && <>
        <p className="semantic-source"><strong>{draft.prepared ? plan ? "Reviewed copy prepared" : "Reviewed recipe retained" : "Reviewing source semantics"}</strong><span>Original revision {draft.revision} · source epoch {draft.sourceEpoch}</span></p>
        {props.stale && <p className="semantic-warning" role="alert">This recipe belongs to an older source. Values are retained, but cannot be exported. Discard it and inspect the current PDF.</p>}
        <label className="form-control">Tagged copy title<input value={draft.title} maxLength={4096} disabled={!editable}
          onChange={(event) => props.onChange({ ...draft, title: event.target.value, reviewed: false })} /></label>
        <label className="form-control">Document language tag<input value={draft.language} autoCapitalize="none" spellCheck={false} placeholder="For example, en-US" maxLength={63} disabled={!editable}
          onChange={(event) => props.onChange({ ...draft, language: event.target.value, reviewed: false })} /></label>
        <p className="muted">Use a language tag such as hi-IN, ar, or zh-Hant. Registry membership and language choice are not automatically verified.</p>
        <h3>Logical reading order</h3>
        <p>Choose each role. Move items into the order a person should read them. Decorative items must be explicitly marked Artifact.</p>
        <ol className="semantic-order" aria-label="Reviewed source reading order">
          {draft.entries.map((entry, index) => <li key={entry.candidate.id}>
            <button type="button" aria-pressed={selected === entry} className={selected === entry ? "selected" : ""}
              onClick={() => setSelectedId(entry.candidate.id)}>
              <span className="semantic-item-number">{index + 1}</span><span><strong>{entry.role || "Role needed"} · Page {entry.candidate.page + 1}</strong>
                <span>{entry.candidate.text || entry.candidate.description}</span></span>
            </button>
          </li>)}
        </ol>
        {selected && <section className="semantic-item-editor" aria-label={`Source item ${selectedIndex + 1} settings`}>
          <div className="semantic-item-heading"><h3>Item {selectedIndex + 1}</h3>
            <button className="text-button" disabled={props.disabled} onClick={() => props.onLocate(selected.candidate.page)}><LocateFixed size={14} />Locate page {selected.candidate.page + 1}</button></div>
          <p>{selected.candidate.description}. Source binding: {selected.candidate.id}.</p>
          {selected.candidate.text && <details className="semantic-scope"><summary>Read full source text</summary><p className="semantic-source-text">{selected.candidate.text}</p></details>}
          <div className="semantic-move-actions">
            <button className="button secondary" aria-label={`Move item ${selectedIndex + 1} earlier`} disabled={!editable || selectedIndex === 0} onClick={() => move(-1)}><ArrowUp size={14} />Earlier</button>
            <button className="button secondary" aria-label={`Move item ${selectedIndex + 1} later`} disabled={!editable || selectedIndex === draft.entries.length - 1} onClick={() => move(1)}><ArrowDown size={14} />Later</button>
          </div>
          <label className="form-control">Role for item {selectedIndex + 1}<select value={selected.role} disabled={!editable} onChange={(event) => {
            const role = event.target.value;
            if (isSemanticRole(role)) changeEntry({ role, alt: "" });
          }}>
            <option value="">Choose a role...</option>
            {selected.candidate.kind === "text" ? <>
              <option value="P">Paragraph</option><option value="H1">Heading 1</option><option value="H2">Heading 2</option><option value="H3">Heading 3</option>
              <option value="H4">Heading 4</option><option value="H5">Heading 5</option><option value="H6">Heading 6</option>
            </> : <option value="Figure">Meaningful figure</option>}
            <option value="Artifact">Artifact (decorative; not read)</option>
          </select></label>
          {selected.role === "Figure" && <label className="form-control">Human alternative for item {selectedIndex + 1}<textarea value={selected.alt} rows={4} maxLength={8192} disabled={!editable}
            placeholder="Describe the information this figure conveys." onChange={(event) => changeEntry({ alt: event.target.value })} /></label>}
          {selected.role === "Artifact" && <p className="semantic-warning">This item will be explicitly marked decorative and omitted from logical reading. Only choose this for content that conveys no needed information.</p>}
          {selected.role && selected.role !== "Artifact" && <label className="form-control">Item language (optional override)<input value={selected.language} placeholder={`Inherit ${draft.language || "document language"}`}
            maxLength={63} spellCheck={false} autoCapitalize="none" disabled={!editable} onChange={(event) => changeEntry({ language: event.target.value })} /></label>}
        </section>}
        {!draft.prepared && <label className="checkbox-setting semantic-review"><input type="checkbox" checked={draft.reviewed} disabled={!editable}
          onChange={(event) => props.onChange({ ...draft, reviewed: event.target.checked })} />I reviewed every role, heading level, logical order, meaningful alternative and language. This is not accessibility certification.</label>}
        {draft.prepared && <p className="semantic-boundary">This recipe does not edit the source. Save the separate copy, then open it to inspect and read its actual tags. Source protection is retained.</p>}
      </>}
      <details className="semantic-scope"><summary>Supported scope and privacy</summary>
        <p>Up to 100 pages and 2,000 source items. Untagged page text, images, painted graphics and validated top-level Folio reflow paragraphs. Arbitrary nested Forms, annotations/links, interactive forms, optional content, scripts, existing trees and conformance-declared sources are not rewritten.</p>
        <p>Existing supported trees are checked against actual MCIDs, page/Form ownership and reverse parent mappings. This is not full PDF/UA or WCAG validation. No automatic descriptions, AI, uploads or runtime standards downloads. Inspection text stays in memory; no unencrypted report export is provided.</p>
      </details>
    </div>
    <div className="semantic-commit">
      {props.error && <p className="semantic-error" role="alert">{props.error}</p>}
      {props.notice && <p className="semantic-notice" role="status">{props.notice}</p>}
      {draft && !draft.prepared && problems.length > 0 && !props.stale && <p className="muted" role="status">{problems[0]}</p>}
      {plan ? <button className="button primary" disabled={props.disabled || props.otherPending || props.stale} onClick={props.onSave}><FileDown size={15} />Save separate tagged copy</button>
        : draft && !draft.prepared ? <button className="button primary" disabled={!editable || problems.length > 0} onClick={props.onPrepare}><ListOrdered size={15} />Prepare reviewed tagged copy</button>
          : draft?.prepared ? <button className="button secondary" disabled={props.disabled || props.otherPending} onClick={props.onRevise}>Revise retained recipe</button> : null}
      {props.working && <button className="text-button" onClick={props.onCancel}>Cancel current operation</button>}
      {plan && !props.working && <button className="text-button" disabled={props.disabled} onClick={props.onRevise}>Cancel prepared copy and revise</button>}
      {draft && <button className="text-button" disabled={props.disabled} onClick={props.onDiscard}>Discard semantic recipe</button>}
    </div>
  </div>;
}

function StructureList({ nodes }: { nodes: SemanticInspection["structure"] }) {
  return <ol className="semantic-existing-tree">{nodes.map((node) => <li key={node.id}>
    <strong>{node.role}</strong>{node.language && <span> · {node.language}</span>}
    {node.alt && <p>Alternative: {node.alt}</p>}
    {node.text && <p>{node.text}</p>}
    {node.bindings.map((binding) => <span className="semantic-binding" key={`${binding.page}:${binding.stream}:${binding.mcid}`}>
      Page {binding.page + 1} · {binding.stream ? `Form ${binding.stream}` : "page stream"} · MCID {binding.mcid}
    </span>)}
    {node.children.length > 0 && <StructureList nodes={node.children} />}
  </li>)}</ol>;
}
