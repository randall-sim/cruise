import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { NextRequest } from "next/server";
import { addCourse, readPrivate, safePath } from "../src/lib/store";
import { createAssignment, deleteAssignment } from "../src/lib/assignments";
import {
  writeAssignmentFile,
  readAssignmentFile,
  recordAssignmentLearning,
  referenceCoursePath,
  moveAssignmentPath,
  deleteAssignmentPath,
  createAssignmentDirectory,
  listAssignmentFiles,
  reorderAssignmentFiles,
} from "../src/lib/assignment-files";
import {
  getAssignmentParts,
  getAssignmentPartChanges,
  getAssignmentTimeline,
  readAssignmentSteps,
  readAssignmentSnapshot,
  assignmentStepBytes,
} from "../src/lib/assignment-timeline";
import { saveSource } from "../src/lib/sources";
import { searchMemory } from "../src/lib/memory";
import { GET } from "../src/app/api/assignment-timeline/route";
import {
  startCourseCommand,
  readCourseCommand,
  stopCourseCommand,
} from "../src/lib/course-terminal";
import { setTimeout as delay } from "node:timers/promises";

let root: string;
const original = process.env.COURSE_CAPTAIN_WORKSPACE;
beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "cc-walkthrough-"));
  process.env.COURSE_CAPTAIN_WORKSPACE = root;
});
afterEach(async () => {
  if (original) process.env.COURSE_CAPTAIN_WORKSPACE = original;
  else delete process.env.COURSE_CAPTAIN_WORKSPACE;
  assert.ok(root.startsWith(path.join(os.tmpdir(), "cc-walkthrough-")));
  await fs.rm(root, { recursive: true, force: true });
});
const explanation = "Use the invariant to keep the running total correct.";
const teaching =
  "The running total starts at zero because we have not processed any values yet. Each addition replaces the total with the sum of everything seen so far. For example, processing 2 and then 3 gives 0 + 2 = 2 and 2 + 3 = 5. This also handles an empty input: without any additions, the answer remains zero.";
const gaps = ["Test fixture has no instructor evidence."];
async function fixture() {
  const course = await addCourse({ code: "WALK", name: "Walkthrough" });
  const assignment = await createAssignment({
    courseId: course.id,
    title: "Running totals",
  });
  return { courseId: course.id, assignmentId: assignment.id };
}
test("sequential parts save teaching, immutable workspace bytes, diffs and paginated summaries", async () => {
  const scope = await fixture();
  const part = { order: 1, title: "Running total" };
  const plan = await recordAssignmentLearning({
    ...scope,
    part,
    phase: "plan",
    title: "Plan the total",
    markdown: "Start at zero because no numbers have been processed yet.",
    nextAction: "Create the accumulator.",
    gaps,
  });
  assert.match(await readPrivate(plan.path), /Part: 1 — Running total/);
  const created = await writeAssignmentFile(
    { ...scope, path: "sum.py", content: "total = 0\n", explanation },
    true,
  );
  const steps = await readAssignmentSteps(scope.courseId, scope.assignmentId);
  assert.equal(steps.length, 2);
  assert.deepEqual(steps[1].part, part);
  assert.equal(steps[1].actor, "agent");
  assert.equal(steps[0].files.length, 0);
  await assert.rejects(
    recordAssignmentLearning({
      ...scope,
      part: { order: 2, title: "Loop" },
      phase: "plan",
      title: "Too soon",
      markdown: "Begin the loop before completing the total.",
      gaps,
    }),
    /Complete the current part/,
  );
  assert.equal(
    (await readAssignmentSteps(scope.courseId, scope.assignmentId)).length,
    2,
  );
  await writeAssignmentFile(
    {
      ...scope,
      path: "sum.py",
      content: "total = 1\n",
      expectedRevision: created.revision,
      explanation,
    },
    false,
    "user",
  );
  const old = await getAssignmentTimeline({
    ...scope,
    stepId: steps[1].id,
    path: "sum.py",
  });
  assert.ok("content" in old);
  assert.equal(old.content, "total = 0\n");
  assert.ok(
    old.diff.lines.some((l) => l.kind === "add" && l.text === "total = 0\n"),
  );
  const snapshot = await assignmentStepBytes({
    ...scope,
    stepId: steps[1].id,
    path: "sum.py",
  });
  assert.equal(snapshot.bytes.toString(), "total = 0\n");
  assert.equal(
    (await readAssignmentFile({ ...scope, path: "sum.py" })).content,
    "total = 1\n",
  );
  await recordAssignmentLearning({
    ...scope,
    part,
    phase: "completed",
    title: "Checked the total",
    markdown:
      "Reviewed the initial value; the demonstration check is complete.",
    teachingMarkdown: teaching,
    gaps,
  });
  await recordAssignmentLearning({
    ...scope,
    part: { order: 2, title: "Loop" },
    phase: "plan",
    title: "Plan the loop",
    markdown: "Visit each value once and add it to the current total.",
    gaps,
  });
  const page = await getAssignmentTimeline({ ...scope, limit: 2 });
  assert.ok("steps" in page);
  assert.equal(page.nextOffset, 2);
  assert.equal(page.total, 5);
  assert.ok(!("files" in page.steps[0]) && !("markdown" in page.steps[0]));
  const tail = await getAssignmentTimeline({ ...scope, offset: 4 });
  assert.ok("steps" in tail && tail.steps);
  assert.equal(tail.steps[0].part?.order, 2);
});

test("student timeline has one stop per assignment part, independent of tool and checkpoint count", async () => {
  const scope = await fixture();
  for (let order = 1; order <= 4; order++) {
    const part = { order, title: `Requirement ${order}` };
    await recordAssignmentLearning({
      ...scope,
      part,
      phase: "plan",
      title: "Operational plan",
      markdown:
        "Status in_progress. Next action: write file and run the command.",
      gaps,
    });
    await writeAssignmentFile(
      {
        ...scope,
        path: `part${order}.txt`,
        content: `Answer ${order}`,
        explanation,
      },
      true,
    );
    await recordAssignmentLearning({
      ...scope,
      part,
      phase: "completed",
      title: "Operational completion",
      markdown: "Status completed. Command run succeeded.",
      teachingMarkdown: `${teaching}\n\nPart ${order} example.`,
      gaps,
    });
  }
  // A later audit entry changes the replay state without replacing the authored lesson.
  await writeAssignmentFile(
    { ...scope, path: "check.txt", content: "Verified", explanation },
    true,
  );
  const raw = await readAssignmentSteps(scope.courseId, scope.assignmentId);
  assert.equal(raw.length, 13);
  const { parts } = await getAssignmentParts(scope);
  assert.deepEqual(
    parts.map((part) => part.order),
    [1, 2, 3, 4],
  );
  assert.equal(parts[3].stepId, raw.at(-1)!.id);
  assert.equal(parts[3].teachingMarkdown, `${teaching}\n\nPart 4 example.`);
  assert.ok(parts.every((part) => !part.teachingMarkdown.includes("Status")));
  const response = await GET(
    new NextRequest(
      `http://localhost/api/assignment-timeline?${new URLSearchParams({ ...scope, mode: "parts" })}`,
      { headers: { host: "localhost" } },
    ),
  );
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { parts });

  const single = await createAssignment({
    courseId: scope.courseId,
    title: "One short question",
  });
  const singleScope = { ...scope, assignmentId: single.id };
  await writeAssignmentFile(
    { ...singleScope, path: "answer.txt", content: "Answer", explanation },
    true,
  );
  await recordAssignmentLearning({
    ...singleScope,
    title: "Legacy checkpoint",
    markdown: "A raw status log must never become the student lesson.",
    gaps,
  });
  const fallback = await getAssignmentParts(singleScope);
  assert.equal(fallback.parts.length, 1);
  assert.equal(fallback.parts[0].teachingMarkdown, "");
  const lesson = await recordAssignmentLearning({
    ...singleScope,
    title: "Explain the solution",
    markdown: "A student lesson was authored.",
    teachingMarkdown: teaching,
    gaps,
  });
  assert.deepEqual(
    (await getAssignmentParts(singleScope)).parts.map(
      ({ stepId, teachingMarkdown }) => ({ stepId, teachingMarkdown }),
    ),
    [{ stepId: lesson.stepId, teachingMarkdown: teaching }],
  );
});

test("completed parts need a teaching lesson; revisions preserve saved bytes, chronology and citations", async () => {
  const scope = await fixture();
  const part = { order: 1, title: "Initialize the total" };
  await recordAssignmentLearning({
    ...scope,
    part,
    phase: "plan",
    title: "Plan",
    markdown: "Operational plan is allowed here.",
    gaps,
  });
  const file = await writeAssignmentFile(
    { ...scope, path: "total.py", content: "total = 0\n", explanation },
    true,
  );
  const completion = {
    ...scope,
    part,
    phase: "completed",
    title: "Complete",
    markdown: "Operational completion.",
    gaps,
  };
  await assert.rejects(
    recordAssignmentLearning(completion),
    /teachingMarkdown/,
  );
  await assert.rejects(
    recordAssignmentLearning({ ...completion, teachingMarkdown: "Too brief." }),
    /200/,
  );
  const first = await recordAssignmentLearning({
    ...completion,
    teachingMarkdown: teaching,
  });
  await recordAssignmentLearning({
    ...scope,
    part: { order: 2, title: "Process values" },
    phase: "work",
    title: "Continue",
    markdown: "Started implementing the loop.",
    gaps,
  });
  await writeAssignmentFile({
    ...scope,
    path: "total.py",
    content: "total = sum(values)\n",
    expectedRevision: file.revision,
    explanation,
  });
  const before = await readAssignmentSteps(scope.courseId, scope.assignmentId);
  await saveSource({
    courseId: scope.courseId,
    title: "Running total notes",
    text: "An accumulator holds the sum of values processed so far, starting at zero before processing begins.",
  });
  const source = (
    await searchMemory(
      scope.courseId,
      "accumulator holds sum values processed",
      30,
    )
  ).find((item) => item.kind === "note")!;
  assert.ok(source);
  const revised = `${teaching}\n\nThe empty-input case establishes the starting point before the loop begins. [${source.id}]`;
  const revision = await recordAssignmentLearning({
    ...scope,
    part,
    teachingForStepId: first.stepId,
    title: "Clearer first-part lesson",
    markdown:
      "Teaching clarified from saved evidence; no new code work was performed.",
    teachingMarkdown: revised,
    sourceIds: [source.id],
    gaps,
  });
  const after = await readAssignmentSteps(scope.courseId, scope.assignmentId);
  assert.equal(revision.stepId, first.stepId);
  assert.equal(after.length, before.length);
  assert.deepEqual(after.at(-1), before.at(-1));
  const anchor = after.find((step) => step.id === first.stepId)!;
  const originalAnchor = before.find((step) => step.id === first.stepId)!;
  assert.equal(anchor.timestamp, originalAnchor.timestamp);
  assert.deepEqual(anchor.files, originalAnchor.files);
  assert.deepEqual(anchor.changes, originalAnchor.changes);
  assert.equal(anchor.markdown, originalAnchor.markdown);
  assert.equal(anchor.teachingMarkdown, teaching);
  assert.equal(anchor.teachingRevisions?.length, 1);
  assert.equal(
    (
      await assignmentStepBytes({
        ...scope,
        stepId: first.stepId,
        path: "total.py",
      })
    ).bytes.toString(),
    "total = 0\n",
  );
  assert.equal(
    (await readAssignmentFile({ ...scope, path: "total.py" })).content,
    "total = sum(values)\n",
  );
  const { parts } = await getAssignmentParts(scope);
  assert.equal(parts[0].teachingMarkdown, revised);
  assert.deepEqual(parts[0].sourceIds, [source.id]);
  assert.equal(parts[0].timestamp, originalAnchor.timestamp);
  assert.match(await readPrivate(revision.path), /Teaching explanation/);
  assert.ok((await readPrivate(revision.path)).includes(`[${source.id}]`));
  const indexed = await searchMemory(
    scope.courseId,
    "empty-input establishes starting point",
    30,
  );
  assert.ok(indexed.some((item) => item.path === revision.path));
  await assert.rejects(
    recordAssignmentLearning({
      ...scope,
      title: "Missing lesson",
      markdown: "Operational checkpoint only.",
      teachingForStepId: first.stepId,
      gaps,
    }),
    /requires.*teachingMarkdown/,
  );
  await assert.rejects(
    recordAssignmentLearning({
      ...scope,
      part: { order: 2, title: "Wrong part" },
      title: "Wrong target",
      markdown: "Cannot revise another part.",
      teachingMarkdown: revised,
      teachingForStepId: first.stepId,
      gaps,
    }),
    /target.*assignment and part/,
  );
});

test("saved directory and file API use the current workspace tree with immutable nested assets", async () => {
  const scope = await fixture();
  await createAssignmentDirectory({ ...scope, path: "src" });
  const file = await writeAssignmentFile(
    { ...scope, path: "src/total.py", content: "total = 0\n", explanation },
    true,
  );
  const step = (
    await readAssignmentSteps(scope.courseId, scope.assignmentId)
  ).at(-1)!;
  await writeAssignmentFile({
    ...scope,
    path: "src/total.py",
    content: "total = 99\n",
    expectedRevision: file.revision,
    explanation,
  });
  async function request(extra: Record<string, string>) {
    return GET(
      new NextRequest(
        `http://localhost/api/assignment-timeline?${new URLSearchParams({ ...scope, stepId: step.id, ...extra })}`,
        { headers: { host: "localhost" } },
      ),
    );
  }
  const rootTree = await (await request({ mode: "directory" })).json();
  assert.deepEqual(
    rootTree.entries.map((entry: { path: string; type: string }) => [
      entry.path,
      entry.type,
    ]),
    [["src", "directory"]],
  );
  assert.equal(rootTree.courseId, scope.courseId);
  assert.equal(rootTree.assignmentId, scope.assignmentId);
  const nested = await (
    await request({ mode: "directory", directory: "src" })
  ).json();
  assert.equal(nested.entries.length, 1);
  assert.equal(nested.entries[0].name, "total.py");
  assert.equal(nested.entries[0].path, "src/total.py");
  const saved = await (
    await request({ mode: "file", path: "src/total.py" })
  ).json();
  assert.equal(saved.content, "total = 0\n");
  assert.equal(saved.revision, file.revision);
  assert.equal(saved.mediaType, "text/plain");
  assert.equal(saved.absolutePath, file.absolutePath);
  const download = await request({
    mode: "asset",
    path: "src/total.py",
    download: "1",
  });
  assert.match(download.headers.get("content-disposition")!, /attachment/);
  assert.equal(await download.text(), "total = 0\n");
  assert.equal(
    (await request({ mode: "directory", directory: "../state.json" })).status,
    400,
  );
});

test("saved parts reveal net changes and serve matching paginated file diffs, including deletions", async () => {
  const scope = await fixture();
  await createAssignmentDirectory({ ...scope, path: "empty/nested" });
  const paths = ["src/deep/edit.txt", "old/nested/gone.txt", "quiet/keep.txt"];
  for (const file of paths) {
    await writeAssignmentFile(
      { ...scope, path: file, content: "before\n", explanation },
      true,
    );
  }
  const checkpoint = async (order: number, phase: "plan" | "completed") =>
    recordAssignmentLearning({
      ...scope,
      part: { order, title: `Part ${order}` },
      phase,
      title: "Learning checkpoint",
      markdown: "Record the assignment part and its current progress.",
      ...(phase === "completed" ? { teachingMarkdown: teaching } : {}),
      gaps,
    });
  const tree = async (stepId: string, directory?: string) => {
    const result = await readAssignmentSnapshot({
      ...scope,
      stepId,
      mode: "directory",
      directory,
    });
    assert.ok("entries" in result);
    return result;
  };
  const plan = await checkpoint(1, "plan");
  assert.deepEqual((await tree(plan.stepId)).expandedDirectories, []);
  const edited = await readAssignmentFile({ ...scope, path: paths[0] });
  const intermediate = await writeAssignmentFile({
    ...scope,
    path: paths[0],
    content: "temporary\n",
    expectedRevision: edited.revision,
    explanation,
  });
  await writeAssignmentFile({
    ...scope,
    path: paths[0],
    content: "after\n",
    expectedRevision: intermediate.revision,
    explanation,
  });
  await writeAssignmentFile(
    {
      ...scope,
      path: "src/new/added.txt",
      content: "new\n".repeat(205),
      explanation,
    },
    true,
  );
  await writeAssignmentFile(
    { ...scope, path: "binary.bin", content: "a\0b", explanation },
    true,
  );
  const listing = await listAssignmentFiles(scope);
  await deleteAssignmentPath({
    ...scope,
    path: "old",
    expectedRevision: listing.entries.find((entry) => entry.path === "old")!
      .revision,
    explanation,
  });
  await deleteAssignmentPath({
    ...scope,
    path: "empty",
    expectedRevision: listing.entries.find((entry) => entry.path === "empty")!
      .revision,
    explanation,
  });
  const completed = await checkpoint(1, "completed");
  assert.deepEqual((await tree(completed.stepId)).expandedDirectories?.sort(), [
    "old",
    "old/nested",
    "src",
    "src/deep",
    "src/new",
  ]);
  const deleted = await tree(completed.stepId, "old/nested");
  assert.equal(deleted.entries[0].change, "removed");
  assert.equal(deleted.expandedDirectories, undefined);
  const file = async (path: string, diffOffset = 0) => {
    const response = await GET(
      new NextRequest(
        `http://localhost/api/assignment-timeline?${new URLSearchParams({ ...scope, stepId: completed.stepId, mode: "file", path, diffOffset: String(diffOffset) })}`,
        { headers: { host: "localhost" } },
      ),
    );
    assert.equal(response.status, 200);
    return response.json();
  };
  const changedFile = await file(paths[0]);
  assert.equal(changedFile.content, "after\n");
  assert.deepEqual(changedFile.diff.lines, [
    { kind: "remove", text: "before\n", oldLine: 1 },
    { kind: "add", text: "after\n", newLine: 1 },
  ]);
  const removedFile = await file(paths[1]);
  assert.equal(removedFile.content, "before\n");
  assert.deepEqual(removedFile.diff.lines, [
    { kind: "remove", text: "before\n", oldLine: 1 },
  ]);
  assert.equal(
    (
      await assignmentStepBytes({
        ...scope,
        stepId: completed.stepId,
        path: paths[1],
      })
    ).bytes.toString(),
    "before\n",
  );
  const addedFile = await file("src/new/added.txt");
  assert.equal(addedFile.diff.lines.length, 200);
  assert.equal(addedFile.diff.nextOffset, 200);
  assert.ok(
    addedFile.diff.lines.every((line: { kind: string }) => line.kind === "add"),
  );
  const remainder = await file("src/new/added.txt", 200);
  assert.equal(remainder.diff.lines.length, 5);
  assert.equal(remainder.diff.lines[0].newLine, 201);
  assert.equal(remainder.diff.nextOffset, null);
  assert.equal((await file(paths[2])).diff, undefined);
  const binary = await file("binary.bin");
  assert.deepEqual(binary.diff.lines, []);
  assert.match(binary.diff.message, /Binary change/);
  const next = await checkpoint(2, "plan");
  assert.deepEqual((await tree(next.stepId)).expandedDirectories, []);
});

test("shared changes, aliases, removals, empty folders and display order retain earlier states", async () => {
  const scope = await fixture();
  const shared = await writeAssignmentFile(
    {
      courseId: scope.courseId,
      path: "shared.txt",
      content: "before",
      explanation,
    },
    true,
  );
  await referenceCoursePath({
    ...scope,
    path: "notes.txt",
    targetPath: "shared.txt",
  });
  const linked = (
    await readAssignmentSteps(scope.courseId, scope.assignmentId)
  ).at(-1)!;
  await writeAssignmentFile({
    courseId: scope.courseId,
    path: "shared.txt",
    expectedRevision: shared.revision,
    content: "after",
    explanation,
  });
  const updated = (
    await readAssignmentSteps(scope.courseId, scope.assignmentId)
  ).at(-1)!;
  assert.equal(updated.files[0].originalPath, "files/shared.txt");
  assert.equal(updated.changes[0].action, "changed");
  assert.equal(
    (
      await assignmentStepBytes({
        ...scope,
        stepId: linked.id,
        path: "notes.txt",
      })
    ).bytes.toString(),
    "before",
  );
  const current = await readAssignmentFile({ ...scope, path: "notes.txt" });
  await moveAssignmentPath({
    ...scope,
    path: "notes.txt",
    destination: "renamed.txt",
    expectedRevision: current.revision,
    explanation,
  });
  await createAssignmentDirectory({ ...scope, path: "empty" });
  const listing = await listAssignmentFiles(scope);
  await reorderAssignmentFiles({
    ...scope,
    paths: listing.entries.map((f) => f.path).reverse(),
    expectedRevision: listing.revision,
  });
  const reordered = (
    await readAssignmentSteps(scope.courseId, scope.assignmentId)
  ).at(-1)!;
  assert.deepEqual(
    reordered.files.map((f) => f.path),
    listing.entries.map((f) => f.path).reverse(),
  );
  assert.ok(
    reordered.files.some((f) => f.path === "empty" && f.type === "directory"),
  );
  await deleteAssignmentPath({
    ...scope,
    path: "renamed.txt",
    expectedRevision: current.revision,
    explanation,
  });
  const removed = (
    await readAssignmentSteps(scope.courseId, scope.assignmentId)
  ).at(-1)!;
  assert.ok(!removed.files.some((f) => f.path === "renamed.txt"));
  const diff = await getAssignmentTimeline({
    ...scope,
    stepId: removed.id,
    path: "renamed.txt",
  });
  assert.ok("diff" in diff && diff.diff);
  assert.ok(
    diff.diff.lines.some((l) => l.kind === "remove" && l.text === "after"),
  );
  assert.equal(
    (await readAssignmentFile({ courseId: scope.courseId, path: "shared.txt" }))
      .content,
    "after",
  );
});

test("timeline rejects foreign identities and paths; downloads remain inert and exact", async () => {
  const scope = await fixture();
  const html = "<script>alert('never execute')</script>";
  await writeAssignmentFile(
    { ...scope, path: "demo.html", content: html, explanation },
    true,
  );
  const step = (
    await readAssignmentSteps(scope.courseId, scope.assignmentId)
  ).at(-1)!;
  const other = await createAssignment({
    courseId: scope.courseId,
    title: "Other work",
  });
  await assert.rejects(
    getAssignmentTimeline({
      ...scope,
      assignmentId: other.id,
      stepId: step.id,
    }),
    /Step not found/,
  );
  await assert.rejects(
    getAssignmentTimeline({ ...scope, stepId: step.id, path: "../state.json" }),
    /relative assignment path/,
  );
  await assert.rejects(
    assignmentStepBytes({ ...scope, stepId: step.id, path: "missing.txt" }),
    /File not found/,
  );
  const query = new URLSearchParams({
    ...scope,
    stepId: step.id,
    path: "demo.html",
    asset: "1",
  });
  const response = await GET(
    new NextRequest(`http://localhost/api/assignment-timeline?${query}`, {
      headers: { host: "localhost" },
    }),
  );
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type")!, /text\/plain/);
  assert.match(response.headers.get("content-security-policy")!, /sandbox/);
  assert.equal(await response.text(), html);
  const denied = await GET(
    new NextRequest(`http://localhost/api/assignment-timeline?${query}`, {
      headers: { host: "localhost", "sec-fetch-site": "cross-site" },
    }),
  );
  assert.equal(denied.status, 400);
  await deleteAssignment(scope);
  await assert.rejects(
    assignmentStepBytes({ ...scope, stepId: step.id, path: "demo.html" }),
    /Assignment not found/,
  );
});

test("external changes are observed honestly, with bounded retention and read-only replay", async () => {
  const scope = await fixture();
  await writeAssignmentFile(
    { ...scope, path: "notes.txt", content: "original", explanation },
    true,
  );
  const originalStep = (
    await readAssignmentSteps(scope.courseId, scope.assignmentId)
  ).at(-1)!;
  const current = await readAssignmentFile({ ...scope, path: "notes.txt" });
  await fs.writeFile(current.absolutePath, "outside edit");
  await writeAssignmentFile(
    { ...scope, path: "check.txt", content: "checkpoint", explanation },
    true,
  );
  const steps = await readAssignmentSteps(scope.courseId, scope.assignmentId);
  assert.equal(steps.at(-2)?.actor, "external");
  assert.match(steps.at(-2)!.markdown, /unknown/);
  assert.equal(
    (
      await assignmentStepBytes({
        ...scope,
        stepId: originalStep.id,
        path: "notes.txt",
      })
    ).bytes.toString(),
    "original",
  );
  await fs.writeFile(current.absolutePath, Buffer.alloc(10_000_001, 65));
  const learning = await recordAssignmentLearning({
    ...scope,
    title: "Observe large output",
    markdown:
      "The generated output is too large for retained content; only metadata can be inspected here.",
    gaps,
  });
  const detail = await getAssignmentTimeline({
    ...scope,
    stepId: learning.stepId,
    path: "notes.txt",
  });
  assert.ok("file" in detail && detail.file);
  assert.equal(detail.file.snapshot?.retained, false);
  assert.match(detail.diff.message, /metadata/);
  const savedFile = await readAssignmentSnapshot({
    ...scope,
    stepId: learning.stepId,
    path: "notes.txt",
    mode: "file",
  });
  assert.ok("diff" in savedFile);
  assert.match(savedFile.diff!.message, /metadata/);
  assert.deepEqual(savedFile.diff!.lines, []);
  await assert.rejects(
    assignmentStepBytes({
      ...scope,
      stepId: learning.stepId,
      path: "notes.txt",
    }),
    /not retained/,
  );
  const manifest = safePath(
    `courses/${scope.courseId}/agent/assignments/${scope.assignmentId}-timeline.json`,
  );
  const before = await fs.readFile(manifest);
  await getAssignmentTimeline({
    ...scope,
    stepId: originalStep.id,
    path: "notes.txt",
  });
  assert.deepEqual(await fs.readFile(manifest), before);
});

test("failed command captures newly generated assignment files and observed result", async () => {
  const scope = await fixture();
  const run = await startCourseCommand({
    ...scope,
    location: "assignment",
    purpose: "Generate a local fixture then deliberately fail.",
    command: "printf 'generated' > generated.txt; exit 7",
  });
  const identity = { courseId: scope.courseId, runId: run.id };
  try {
    let result = await readCourseCommand(identity);
    for (
      let i = 0;
      i < 150 && ["queued", "running"].includes(result.status);
      i++
    ) {
      await delay(100);
      result = await readCourseCommand(identity);
    }
    assert.equal(result.status, "failed");
    assert.equal(result.exitCode, 7);
    const step = (
      await readAssignmentSteps(scope.courseId, scope.assignmentId)
    ).at(-1)!;
    assert.equal(step.runId, run.id);
    assert.match(step.markdown, /exit 7/);
    assert.equal(
      (
        await assignmentStepBytes({
          ...scope,
          stepId: step.id,
          path: "generated.txt",
        })
      ).bytes.toString(),
      "generated",
    );
  } finally {
    await stopCourseCommand(identity);
  }
});

test("part change counts compare net text changes with saved baselines, not tool writes or live bytes", async () => {
  const scope = await fixture();
  const original = await writeAssignmentFile(
    { ...scope, path: "answer.txt", content: "keep\nold\n", explanation },
    true,
  );
  const checkpoint = async (order: number, phase: "plan" | "completed") =>
    recordAssignmentLearning({
      ...scope,
      part: { order, title: `Part ${order}` },
      phase,
      title: "Learning checkpoint",
      markdown: "Record the assignment part and its current progress.",
      ...(phase === "completed" ? { teachingMarkdown: teaching } : {}),
      gaps,
    });
  const plan = await checkpoint(1, "plan");
  const baseline = await readAssignmentSnapshot({
    ...scope,
    stepId: plan.stepId,
    mode: "directory",
  });
  assert.ok("entries" in baseline);
  assert.equal(baseline.entries[0].change, undefined);
  const intermediate = await writeAssignmentFile({
    ...scope,
    path: "answer.txt",
    content: "temporary\n",
    expectedRevision: original.revision,
    explanation,
  });
  const final = await writeAssignmentFile({
    ...scope,
    path: "answer.txt",
    content: "keep\nnew\nextra\n",
    expectedRevision: intermediate.revision,
    explanation,
  });
  const first = await checkpoint(1, "completed");
  const changed = await readAssignmentSnapshot({
    ...scope,
    stepId: first.stepId,
    mode: "directory",
  });
  assert.ok("entries" in changed);
  assert.equal(
    changed.entries.find((f) => f.path === "answer.txt")?.change,
    "changed",
  );
  assert.deepEqual(
    await getAssignmentPartChanges({ ...scope, stepId: first.stepId }),
    {
      filesChanged: 1,
      linesAdded: 2,
      linesDeleted: 1,
      omittedFiles: 0,
      approximate: false,
    },
  );
  await checkpoint(2, "plan");
  await createAssignmentDirectory({ ...scope, path: "empty" });
  await deleteAssignmentPath({
    ...scope,
    path: "answer.txt",
    expectedRevision: final.revision,
    explanation,
  });
  await writeAssignmentFile(
    {
      ...scope,
      path: "replacement.txt",
      content: "replacement\n",
      explanation,
    },
    true,
  );
  const second = await checkpoint(2, "completed");
  const sidebar = await readAssignmentSnapshot({
    ...scope,
    stepId: second.stepId,
    mode: "directory",
  });
  assert.ok("entries" in sidebar);
  assert.equal(
    sidebar.entries.find((f) => f.path === "answer.txt")?.change,
    "removed",
  );
  assert.equal(
    sidebar.entries.find((f) => f.path === "replacement.txt")?.change,
    "added",
  );
  assert.equal(
    sidebar.entries.find((f) => f.path === "empty")?.change,
    undefined,
  );
  const expected = {
    filesChanged: 2,
    linesAdded: 1,
    linesDeleted: 3,
    omittedFiles: 0,
    approximate: false,
  };
  assert.deepEqual(
    await getAssignmentPartChanges({ ...scope, stepId: second.stepId }),
    expected,
  );
  const response = await GET(
    new NextRequest(
      `http://127.0.0.1:3000/api/assignment-timeline?${new URLSearchParams({ ...scope, stepId: second.stepId, mode: "changes" })}`,
      { headers: { host: "127.0.0.1:3000" } },
    ),
  );
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), expected);
  const file = await readAssignmentFile({ ...scope, path: "replacement.txt" });
  await writeAssignmentFile({
    ...scope,
    path: "replacement.txt",
    content: "changed later\nextra later\n",
    expectedRevision: file.revision,
    explanation,
  });
  assert.deepEqual(
    await getAssignmentPartChanges({ ...scope, stepId: second.stepId }),
    expected,
  );
  const other = await fixture();
  await assert.rejects(
    getAssignmentPartChanges({ ...other, stepId: second.stepId }),
    /Step not found/,
  );
});
