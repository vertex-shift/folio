import type { DocumentInfo } from "../../lib/types.ts";
import type { SemanticCopyPlan, SemanticCopyRequest, SemanticDraft, SemanticInspection, SemanticNode, SemanticRole } from "./types.ts";

export function isSemanticRole(value: string): value is SemanticRole | "" {
  return ["", "P", "H1", "H2", "H3", "H4", "H5", "H6", "Figure", "Artifact"].includes(value);
}

export function newSemanticDraft(report: SemanticInspection): SemanticDraft {
  if (!report.authoringAllowed || report.status !== "untagged") throw new Error("This PDF is inspect-only. Existing or unsupported structure is never replaced.");
  return {
    documentId: report.documentId, revision: report.revision, sourceEpoch: report.sourceEpoch,
    title: report.title, language: report.language ?? "", reviewed: false, prepared: false,
    entries: report.candidates.map((candidate) => ({ candidate, role: "", alt: "", language: "" })),
  };
}

export const semanticDraftPending = (draft: SemanticDraft | null) => Boolean(draft && !draft.prepared);

export function validLanguage(value: string): boolean {
  const parts = value.split("-"), primary = parts[0];
  return value.length <= 63 && parts.length <= 12
    && (/^[a-z]{2,8}$/i.test(primary) || primary === "x" && parts.length > 1)
    && parts.slice(1).every((part) => /^[a-z0-9]{1,8}$/i.test(part));
}

function validText(value: string, limit: number) {
  return Boolean(value.trim()) && new TextEncoder().encode(value).length <= limit && !/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f\ufffe\uffff]/u.test(value);
}

export function semanticProblems(draft: SemanticDraft): string[] {
  const problems: string[] = [];
  if (!validText(draft.title, 4096)) problems.push("Enter a nonempty document title within 4,096 UTF-8 bytes.");
  if (!validLanguage(draft.language)) problems.push("Enter the document language tag, for example en-US or hi-IN.");
  const missing = draft.entries.filter((entry) => !entry.role).length;
  if (missing) problems.push(`Assign an explicit role or Artifact to ${missing} remaining source ${missing === 1 ? "item" : "items"}.`);
  const alternatives = draft.entries.filter((entry) => entry.role === "Figure" && !validText(entry.alt, 8192)).length;
  if (alternatives) problems.push(`Supply a meaningful human alternative for ${alternatives} ${alternatives === 1 ? "figure" : "figures"}.`);
  if (draft.entries.some((entry) => entry.candidate.kind === "text" ? entry.role === "Figure"
    : Boolean(entry.role && entry.role !== "Figure" && entry.role !== "Artifact"))) problems.push("Text needs a heading or paragraph role; non-text content needs a meaningful Figure or explicit Artifact role.");
  if (new Set(draft.entries.map((entry) => entry.candidate.id)).size !== draft.entries.length) problems.push("Each source candidate must be reviewed exactly once.");
  if (draft.entries.some((entry) => entry.language && !validLanguage(entry.language))) problems.push("Correct the per-item language tags.");
  if (draft.entries.every((entry) => entry.role === "Artifact")) problems.push("Do not hide all meaningful document content as Artifact.");
  if (!draft.reviewed) problems.push("Confirm that you reviewed reading order, heading levels, alternatives and languages.");
  return problems;
}

export function semanticRequest(info: DocumentInfo, draft: SemanticDraft | null): SemanticCopyRequest {
  if (!draft || info.id !== draft.documentId || info.revision !== draft.revision) {
    throw new Error("This semantic draft belongs to an older PDF revision. Your values are kept; discard them and inspect the current PDF.");
  }
  if (info.readOnlyReason) throw new Error(info.readOnlyReason);
  const problems = semanticProblems(draft);
  if (problems.length) throw new Error(problems.join(" "));
  return { documentId: draft.documentId, revision: draft.revision, sourceEpoch: draft.sourceEpoch,
    title: draft.title, language: draft.language, reviewed: draft.reviewed,
    entries: draft.entries.map(({ candidate, role, alt, language }) => {
      if (!role) throw new Error("Every source item needs an explicit role.");
      return { candidateId: candidate.id, role, alt: role === "Figure" ? alt : null, language: language || null };
    }) };
}

export function assertSemanticCurrent(source: Pick<SemanticCopyPlan, "documentId" | "revision">, current: DocumentInfo | null, pending: boolean) {
  if (!current || current.id !== source.documentId || current.revision !== source.revision || pending) {
    throw new Error("The semantic source or pending drafts changed. No stale reading view or tagged copy is used.");
  }
}

export function readingLeaves(nodes: SemanticNode[]): SemanticNode[] {
  return nodes.flatMap((node) => node.children.length ? readingLeaves(node.children) : [node]);
}
