import { createElement, useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowUp, Eye } from "lucide-react";
import { readingLeaves } from "./model";
import type { SemanticInspection, SemanticNode } from "./types";

interface Props { inspection: SemanticInspection; onClose(): void; onVisual(page: number): void }

export default function StructuredReader({ inspection, onClose, onVisual }: Props) {
  const host = useRef<HTMLDivElement>(null), title = useRef<HTMLHeadingElement>(null);
  const leaves = readingLeaves(inspection.structure);
  const [index, setIndex] = useState(-1);
  const headings = leaves.filter((node) => /^H[1-6]?$/.test(node.role));
  const prefix = `semantic-${inspection.documentId}-`;
  useEffect(() => { title.current?.focus(); }, [inspection.inspectionId]);
  function focus(node: SemanticNode) {
    const target = host.current?.querySelector<HTMLElement>(`[id="${prefix}${node.id}"]`);
    target?.focus(); target?.scrollIntoView({ block: "nearest", behavior: "auto" });
    setIndex(leaves.indexOf(node));
  }
  return <div className="structured-reader" ref={host}>
    <header className="structured-reader-header">
      <div><h2 ref={title} tabIndex={-1}>Structured reading</h2><p>Actual PDF logical order. Visual page text is not duplicated in this view.</p></div>
      <button className="button secondary" onClick={onClose}><Eye size={15} />Visual PDF</button>
    </header>
    <nav className="semantic-reader-navigation" aria-label="Tagged content navigation">
      <button className="button secondary" disabled={index <= 0} onClick={() => focus(leaves[index - 1])}><ArrowUp size={14} />Previous item</button>
      <button className="button secondary" disabled={index >= leaves.length - 1} onClick={() => focus(leaves[index + 1])}><ArrowDown size={14} />{index < 0 ? "Start reading" : "Next item"}</button>
      <label>Go to tagged heading<select aria-label="Go to tagged heading" value="" onChange={(event) => { const node = headings.find((node) => node.id === event.target.value); if (node) focus(node); }}>
        <option value="">Choose a heading...</option>{headings.map((node) => <option key={node.id} value={node.id}>{node.role}: {node.text}</option>)}
      </select></label>
    </nav>
    <div className="structured-reader-body">
      <p className="semantic-boundary">{inspection.complete ? "Supported content bindings accounted for; human semantics are not certified." : "Partial structured view. Unbound or unsupported content is not represented; review the stated boundary."}</p>
      {inspection.issues.map((issue, index) => <p className="semantic-warning" key={index}>{issue}</p>)}
      <article className="semantic-document" lang={inspection.language ?? undefined} aria-label={inspection.title || "Tagged PDF content"}>
        {inspection.structure.map((node) => <ReadingNode key={node.id} node={node} prefix={prefix} />)}
      </article>
      {index >= 0 && leaves[index]?.bindings[0] && <button className="text-button semantic-locate-reading" onClick={() => onVisual(leaves[index].bindings[0].page)}>Locate current item on visual page {leaves[index].bindings[0].page + 1}</button>}
    </div>
  </div>;
}

function ReadingNode({ node, prefix }: { node: SemanticNode; prefix: string }) {
  const properties = { id: `${prefix}${node.id}`, lang: node.language ?? undefined, tabIndex: node.children.length ? undefined : -1 };
  if (node.children.length) {
    const tag = node.role === "L" ? "ul" : node.role === "LI" ? "li" : node.role === "Art" ? "article" : "div";
    return createElement(tag, properties, node.children.map((child) => <ReadingNode key={child.id} node={child} prefix={prefix} />));
  }
  if (node.role === "Figure") return <figure {...properties} role="img" aria-label={node.alt ?? undefined}><figcaption aria-hidden="true">{node.alt}</figcaption></figure>;
  const tag = /^H[1-6]$/.test(node.role) ? node.role.toLowerCase() : node.role === "H" ? "h2" : node.role === "Code" ? "pre" : node.role === "BlockQuote" ? "blockquote" : "p";
  return createElement(tag, properties, node.text);
}
