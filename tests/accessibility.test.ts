import test from "node:test";
import assert from "node:assert/strict";
import { assertSemanticCurrent, newSemanticDraft, readingLeaves, semanticDraftPending, semanticProblems, semanticRequest, validLanguage } from "../src/features/accessibility/model.ts";
import { createDraftSynchronizer, hasDocumentDrafts } from "../src/lib/draftProtection.ts";
import type { DocumentInfo } from "../src/lib/types.ts";
import type { SemanticInspection, SemanticNode } from "../src/features/accessibility/types.ts";

const info = { id: 7, revision: 3, readOnlyReason: null } as DocumentInfo;
const report: SemanticInspection = {
  documentId: 7, revision: 3, inspectionId: 21, epoch: 5, sourceEpoch: 4,
  name: "Original.pdf", title: "", language: null, status: "untagged", authoringAllowed: true, complete: false,
  candidates: [
    { id: "text-left", page: 0, kind: "text", text: "Café.\n\nAn explicit paragraph break.", description: "Reflowed paragraph" },
    { id: "text-right", page: 0, kind: "text", text: "Second paragraph", description: "Source text" },
    { id: "image", page: 0, kind: "figure", text: null, description: "Source image" },
  ], structure: [], issues: [], boundary: "No certification.",
};

function reviewed() {
  const draft = newSemanticDraft(report);
  draft.title = "Original semantic document"; draft.language = "en-US"; draft.reviewed = true;
  draft.entries[0].role = "H1"; draft.entries[0].language = "fr-FR";
  draft.entries[1].role = "P"; draft.entries[2].role = "Figure";
  draft.entries[2].alt = "Four colored squares, described by the human author.";
  return draft;
}

test("source review never infers order, roles, language or figure alternatives", () => {
  const draft = newSemanticDraft(report);
  assert.deepEqual(draft.entries.map((entry) => entry.role), ["", "", ""]);
  assert.deepEqual(draft.entries.map((entry) => entry.alt), ["", "", ""]);
  assert.deepEqual(draft.entries.map((entry) => entry.candidate.id), ["text-left", "text-right", "image"]);
  assert.equal(draft.language, ""); assert.equal(draft.reviewed, false);
  assert.equal(semanticDraftPending(draft), true, "incomplete semantics remain a protected draft");
  assert.equal(semanticDraftPending({ ...draft, prepared: true }), false, "prepared copies are retained output jobs, not source edits");
  assert.equal(draft.entries[0].candidate.text, report.candidates[0].text);
  assert.throws(() => newSemanticDraft({ ...report, status: "tagged", authoringAllowed: false }), /inspect-only/);
});

test("reviewed requests retain exact source epoch, deliberate order, languages and human alternative", () => {
  const draft = reviewed();
  [draft.entries[0], draft.entries[1]] = [draft.entries[1], draft.entries[0]];
  const request = semanticRequest(info, draft);
  assert.equal(request.sourceEpoch, 4); assert.equal(request.reviewed, true);
  assert.deepEqual(request.entries.map((entry) => entry.candidateId), ["text-right", "text-left", "image"]);
  assert.deepEqual(request.entries.map((entry) => entry.role), ["P", "H1", "Figure"]);
  assert.equal(request.entries[1].language, "fr-FR");
  assert.equal(request.entries[0].alt, null);
  assert.equal(request.entries[2].alt, draft.entries[2].alt);
  assert.equal(request.language, "en-US"); assert.equal(request.title, draft.title);
});

test("missing semantics, invalid roles and empty figure Alt are never complete recipes", () => {
  for (const variant of ["role", "alt", "language", "title", "review", "kind", "duplicate", "all-artifact"]) {
    const draft = reviewed();
    if (variant === "role") draft.entries[0].role = "";
    else if (variant === "alt") draft.entries[2].alt = " \t";
    else if (variant === "language") draft.language = "not_a_tag";
    else if (variant === "title") draft.title = "";
    else if (variant === "review") draft.reviewed = false;
    else if (variant === "kind") draft.entries[2].role = "P";
    else if (variant === "duplicate") draft.entries[1].candidate = draft.entries[0].candidate;
    else draft.entries.forEach((entry) => { entry.role = "Artifact"; entry.alt = ""; });
    assert.ok(semanticProblems(draft).length, variant);
    assert.throws(() => semanticRequest(info, draft), Error, variant);
  }
  const explicit = reviewed();
  explicit.entries[2].role = "Artifact"; explicit.entries[2].alt = "";
  assert.equal(semanticProblems(explicit).length, 0);
});

test("stale, signed and protected read-only sources do not become tagged-copy requests", () => {
  const draft = reviewed();
  assert.throws(() => semanticRequest({ ...info, revision: 4 }, draft), /older PDF revision/);
  assert.throws(() => semanticRequest({ ...info, id: 8 }, draft), /older PDF revision/);
  assert.throws(() => semanticRequest({ ...info, readOnlyReason: "Signed source is read-only." }, draft), /Signed/);
  assert.throws(() => assertSemanticCurrent(report, info, true), /pending drafts changed/);
  assert.throws(() => assertSemanticCurrent(report, null, false), /source/);
  assert.throws(() => assertSemanticCurrent(report, { ...info, revision: 4 }, false), /source/);
  assert.doesNotThrow(() => assertSemanticCurrent(report, info, false));
  for (const tag of ["en-US", "hi-IN", "zh-Hant", "ar", "fr-FR", "x-local"]) assert.equal(validLanguage(tag), true, tag);
  for (const tag of ["", "en_US", "-en", "en-", "x", "a-foo", "en--US"]) assert.equal(validLanguage(tag), false, tag);
});

test("semantic clearing and preparation preserve paragraph and every other draft authority", async () => {
  const clear = { writing: false, formFilling: false, formDesign: false, pageAuthoring: false, imageEditing: false, paragraphEditing: false, accessibility: false, redaction: false };
  const writes: boolean[] = [];
  const sync = createDraftSynchronizer(async (_id, pending) => { writes.push(pending); });
  await sync(7, hasDocumentDrafts({ ...clear, accessibility: true, paragraphEditing: true }));
  await sync(7, hasDocumentDrafts({ ...clear, accessibility: false, paragraphEditing: true }));
  await sync(7, hasDocumentDrafts(clear));
  assert.deepEqual(writes, [true, false], "clearing one feature cannot unprotect another feature's draft");
  for (const domain of Object.keys(clear)) assert.equal(hasDocumentDrafts({ ...clear, [domain]: true, accessibility: false }), domain !== "accessibility");
});

test("reading leaves preserve bound logical order rather than page drawing order", () => {
  const node = (id: string, role: string, children: SemanticNode[] = []): SemanticNode => ({ id, role, children, text: id, language: null, alt: null, bindings: [{ page: 0, mcid: Number(id), stream: null }] });
  const tree = [node("root", "Document", [node("0", "H1"), node("2", "P"), node("1", "P"), node("3", "Figure")])];
  assert.deepEqual(readingLeaves(tree).map((node) => node.id), ["0", "2", "1", "3"]);
});
