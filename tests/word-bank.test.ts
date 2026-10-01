import { test } from "node:test";
import assert from "node:assert/strict";
import { mergeWordBank, wordBankDefinition } from "../src/lib/word-bank";
import type { Concept, Citation } from "../src/lib/schema";

test("Word bank reuses terms and appends sourced lecture context without overwriting history", () => {
  const source: Citation = {
    id: "l2:t0",
    courseId: "course",
    lectureId: "l2",
    title: "Current lecture",
    text: "Evidence",
    path: "transcript.vtt",
    kind: "transcript",
    seconds: 0,
  };
  const old: Concept = {
    id: "original",
    courseId: "course",
    lectureId: "l1",
    term: "Hypothesis",
    definition: "A testable proposed explanation.",
    citations: [],
    mastered: true,
  };
  const other = { ...old, id: "other", courseId: "other" };
  const lecture = {
    id: "l2",
    courseId: "course",
    title: "Scientific inference",
    date: "2026-09-30",
  };
  const reused = mergeWordBank(
    [other, old],
    [
      {
        term: " hypothesis ",
        definition: "Unnecessary replacement",
        citations: [source.id],
      },
    ],
    lecture,
    [source],
    () => "new",
  );
  assert.deepEqual(reused, [other, old]);
  const entry = {
    term: "HYPOTHESIS",
    definition:
      "A historical hypothesis can be tested through predictions about surviving evidence.",
    extensionReason:
      "This lecture applies testing to events that cannot be repeated.",
    citations: [source.id],
  };
  const extended = mergeWordBank(
    reused,
    [entry],
    lecture,
    [source],
    () => "new",
  );
  assert.equal(extended.length, 2);
  assert.equal(extended[1].id, old.id);
  assert.ok(extended[1].definition.startsWith(old.definition));
  assert.match(extended[1].definition, /Scientific inference.*2026-09-30.*l2/);
  assert.ok(extended[1].definition.includes(entry.extensionReason));
  assert.ok(extended[1].definition.includes(entry.definition));
  assert.deepEqual(extended[1].citations, [source]);
  assert.equal(extended[1].mastered, false);
  assert.equal(old.definition, "A testable proposed explanation.");
  assert.deepEqual(
    mergeWordBank(extended, [entry], lecture, [source], () => "new"),
    extended,
  );
  const fresh = mergeWordBank(
    extended,
    [
      {
        term: "Prediction",
        definition: "An expected observation.",
        citations: [source.id],
      },
    ],
    lecture,
    [source],
    () => "new",
  );
  assert.equal(fresh[2].id, "new");
  assert.equal(fresh[2].lectureId, "l2");
  assert.equal(
    wordBankDefinition(
      "#word-bank-hypothesis",
      extended.filter((c) => c.courseId === "course"),
    ),
    extended[1].definition,
  );
  assert.equal(wordBankDefinition("#word-bank-%E0%A4%A", extended), undefined);
  assert.equal(wordBankDefinition("https://example.com", extended), undefined);
  assert.equal(wordBankDefinition("#word-bank-Missing", extended), undefined);
});
