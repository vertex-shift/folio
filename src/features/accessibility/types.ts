export type SemanticRole = "P" | "H1" | "H2" | "H3" | "H4" | "H5" | "H6" | "Figure" | "Artifact";
export interface SourceRef { documentId: number; revision: number }
export interface Candidate {
  id: string;
  page: number;
  kind: "text" | "figure" | "graphic";
  text: string | null;
  description: string;
}
export interface ContentBinding { page: number; mcid: number; stream: string | null }
export interface SemanticNode {
  id: string;
  role: string;
  language: string | null;
  alt: string | null;
  text: string | null;
  bindings: ContentBinding[];
  children: SemanticNode[];
}
export interface SemanticInspection extends SourceRef {
  inspectionId: number;
  epoch: number;
  sourceEpoch: number;
  name: string;
  title: string;
  language: string | null;
  status: "untagged" | "tagged" | "unsupported" | "malformed_or_unsupported";
  authoringAllowed: boolean;
  complete: boolean;
  candidates: Candidate[];
  structure: SemanticNode[];
  issues: string[];
  boundary: string;
}
export interface SemanticEntry {
  candidateId: string;
  role: SemanticRole;
  alt: string | null;
  language: string | null;
}
export interface SemanticCopyRequest extends SourceRef {
  sourceEpoch: number;
  title: string;
  language: string;
  entries: SemanticEntry[];
  reviewed: boolean;
}
export interface SemanticCopyPlan extends SourceRef {
  jobId: number;
  epoch: number;
  sourceEpoch: number;
  suggestedName: string;
  byteLength: number;
  passwordProtected: boolean;
  boundary: string;
}
export interface SemanticDraftEntry {
  candidate: Candidate;
  role: SemanticRole | "";
  alt: string;
  language: string;
}
export interface SemanticDraft extends SourceRef {
  sourceEpoch: number;
  title: string;
  language: string;
  entries: SemanticDraftEntry[];
  reviewed: boolean;
  prepared: boolean;
}
