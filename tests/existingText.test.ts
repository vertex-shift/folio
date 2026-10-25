import test from "node:test";
import assert from "node:assert/strict";
import { matchExistingText, textRevisionKey } from "../src/lib/existingText.ts";
import type { ExistingTextRun } from "../src/lib/types.ts";

function run(text: string, path: number[], origin: [number, number] | null): ExistingTextRun {
  return { path, text, origin, font: "Helvetica", fontSize: 12, editable: true, reason: null };
}

test("matches a rendered text run without changing its source identity", () => {
  const source = run("A passage of text", [8], [40, 700]);
  assert.equal(matchExistingText([source], "A passage of text", [40, 700]), source);
  assert.equal(matchExistingText([source], "passage", [50, 700]), source);
});

test("resolves repeated text only when source positions distinguish the occurrence", () => {
  const first = run("Repeated", [8], [40, 700]);
  const second = run("Repeated", [14], [40, 500]);
  assert.equal(matchExistingText([first, second], "Repeated", [40, 500]), second);
  assert.equal(matchExistingText([first, second], "Repeated", [40, 600]), null);
  assert.equal(matchExistingText([first, { ...second, origin: null }], "Repeated", [40, 700]), null);
  assert.equal(matchExistingText([first, { ...second, origin: [40, 700] }], "Repeated", [40, 700]), null);
});

test("handles rendered whitespace and ligature normalization without accepting empty hits", () => {
  const source = run("An ofﬁce line", [7], [10, 20]);
  assert.equal(matchExistingText([source], "An office line", [10, 20]), source);
  assert.equal(matchExistingText([run("OneTwo", [8], [10, 20])], "One Two", [10, 20])?.path[0], 8);
  assert.equal(matchExistingText([source], "  ", [10, 20]), null);
});

test("never substitutes a same-spelled run at another position or an overlapping drawing", () => {
  const source = run("Semantic text", [8], [20, 500]);
  assert.equal(matchExistingText([source], "Semantic text", [20, 700]), null);
  assert.equal(matchExistingText([{ ...source, origin: null }], "Semantic text", [20, 500]), null);
  assert.equal(matchExistingText([source, run("Other drawing", [9], [20, 500])], "Semantic text", [20, 500]), null);
});
test("does not guess when PDF.js combines multiple drawing operations into one text item", () => {
  assert.equal(matchExistingText([run("First", [8], [20, 30]), run("Second", [9], null)],
    "First Second", [20, 30]), null);
});

test("search and source-text caches change identity on edits, undo, and document switches", () => {
  assert.notEqual(textRevisionKey({ id: 1, revision: 0 }), textRevisionKey({ id: 1, revision: 1 }));
  assert.notEqual(textRevisionKey({ id: 1, revision: 1 }), textRevisionKey({ id: 2, revision: 1 }));
  assert.equal(textRevisionKey({ id: 1, revision: 0 }), "1:0");
});
