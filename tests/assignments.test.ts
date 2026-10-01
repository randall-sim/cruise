import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { NextRequest } from "next/server";
import { addCourse, readPrivate, safePath } from "../src/lib/store";
import {
  createAssignment,
  listAssignments,
  deleteAssignment,
} from "../src/lib/assignments";
import {
  createAssignmentDirectory,
  listAssignmentFiles,
  readAssignmentFile,
  writeAssignmentFile,
  editAssignmentFile,
  moveAssignmentPath,
  deleteAssignmentPath,
  reorderAssignmentFiles,
  recordAssignmentLearning,
  referenceCoursePath,
  probeAssignmentFiles,
} from "../src/lib/assignment-files";
import { allEvidence, searchMemory } from "../src/lib/memory";
import { getFileHistory } from "../src/lib/file-history";
import { saveSource } from "../src/lib/sources";
import { prepareExam, previewExamScope, queueExam } from "../src/lib/exams";
import { completeJob, createJob } from "../src/lib/jobs";
import { readCourseContext } from "../src/lib/course-context";
import { GET, POST } from "../src/app/api/assignments/route";

let root: string;
const previous = process.env.COURSE_CAPTAIN_WORKSPACE;
beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "cc-assignments-"));
  process.env.COURSE_CAPTAIN_WORKSPACE = root;
});
afterEach(async () => {
  if (previous) process.env.COURSE_CAPTAIN_WORKSPACE = previous;
  else delete process.env.COURSE_CAPTAIN_WORKSPACE;
  assert.ok(root.startsWith(path.join(os.tmpdir(), "cc-assignments-")));
  await fs.rm(root, { recursive: true, force: true });
});
async function fixture() {
  const course = await addCourse({ code: "CS", name: "Systems" });
  const assignment = await createAssignment({
    courseId: course.id,
    title: "Memory allocator",
    description: "Study an allocator with course evidence.",
  });
  return {
    course,
    assignment,
    scope: { courseId: course.id, assignmentId: assignment.id },
  };
}
const explanation =
  "Explain the allocator's free list structure and split behavior in a local draft.";

test("first-entry probe handles empty, excluded and displayed paths without full file reads", async () => {
  const { scope } = await fixture();
  assert.equal((await probeAssignmentFiles(scope)).hasEntries, false);
  await writeAssignmentFile(
    { ...scope, path: "notes.md", content: "Existing notes", explanation },
    true,
  );
  assert.deepEqual(await probeAssignmentFiles(scope), {
    hasEntries: true,
    firstPath: "notes.md",
  });
  assert.equal(
    (await probeAssignmentFiles({ courseId: scope.courseId })).hasEntries,
    true,
  );
  const other = await addCourse({ code: "EMPTY", name: "Excluded only" });
  await fs.mkdir(safePath(`courses/${other.id}/files/.git`), {
    recursive: true,
  });
  assert.equal(
    (await probeAssignmentFiles({ courseId: other.id })).hasEntries,
    false,
  );
});

test("shared file references edit the original across assignments, reject stale saves and detach without deleting content", async () => {
  const { course, scope } = await fixture();
  const shared = { courseId: course.id };
  await writeAssignmentFile(
    { ...shared, path: "src/common.c", content: "int value = 1;", explanation },
    true,
  );
  const other = await createAssignment({
    ...shared,
    title: "Second assignment",
  });
  const second = { ...shared, assignmentId: other.id };
  await referenceCoursePath({
    ...scope,
    path: "common.c",
    targetPath: "src/common.c",
  });
  await referenceCoursePath({
    ...second,
    path: "reused.c",
    targetPath: "src/common.c",
  });
  const original = await readAssignmentFile({ ...scope, path: "common.c" });
  assert.equal(original.shared?.path, "src/common.c");
  assert.equal(
    original.absolutePath,
    safePath(`courses/${course.id}/files/src/common.c`),
  );
  await writeAssignmentFile({
    ...scope,
    path: "common.c",
    content: "int value = 2;",
    expectedRevision: original.revision,
    explanation,
  });
  assert.equal(
    (await readAssignmentFile({ ...shared, path: "src/common.c" })).content,
    "int value = 2;",
  );
  assert.equal(
    (await readAssignmentFile({ ...second, path: "reused.c" })).content,
    "int value = 2;",
  );
  await assert.rejects(
    writeAssignmentFile({
      ...second,
      path: "reused.c",
      content: "stale",
      expectedRevision: original.revision,
      explanation,
    }),
    /changed/,
  );
  const fresh = await readAssignmentFile({ ...scope, path: "common.c" });
  await moveAssignmentPath({
    ...scope,
    path: "common.c",
    destination: "alias.c",
    expectedRevision: fresh.revision,
    explanation,
  });
  assert.equal(
    (await readAssignmentFile({ ...scope, path: "alias.c" })).content,
    "int value = 2;",
  );
  await deleteAssignmentPath({
    ...scope,
    path: "alias.c",
    expectedRevision: fresh.revision,
    explanation,
  });
  assert.equal((await listAssignmentFiles(scope)).entries.length, 0);
  assert.equal(
    (await readAssignmentFile({ ...second, path: "reused.c" })).content,
    "int value = 2;",
  );
});

test("folder references route creates, uploads, moves, edits and deletes to shared storage", async () => {
  const { course, scope } = await fixture();
  const shared = { courseId: course.id };
  await writeAssignmentFile(
    { ...shared, path: "project/main.c", content: "start", explanation },
    true,
  );
  await referenceCoursePath({
    ...scope,
    path: "project",
    targetPath: "project",
  });
  await createAssignmentDirectory({ ...scope, path: "project/src" });
  await writeAssignmentFile(
    {
      ...scope,
      path: "project/src/new.txt",
      content: "new shared file",
      explanation,
    },
    true,
  );
  const local = await writeAssignmentFile(
    { ...scope, path: "local.txt", content: "moved to shared", explanation },
    true,
  );
  await moveAssignmentPath({
    ...scope,
    path: "local.txt",
    destination: "project/local.txt",
    expectedRevision: local.revision,
    explanation,
  });
  assert.equal(
    (await readAssignmentFile({ ...shared, path: "project/local.txt" }))
      .content,
    "moved to shared",
  );
  assert.equal(
    (await readAssignmentFile({ ...shared, path: "project/src/new.txt" }))
      .content,
    "new shared file",
  );
  const main = await readAssignmentFile({ ...scope, path: "project/main.c" });
  await editAssignmentFile({
    ...scope,
    path: main.path,
    expectedRevision: main.revision,
    edits: [{ oldText: "start", newText: "edited" }],
    explanation,
  });
  const sharedEdit = await readAssignmentFile({
    ...shared,
    path: "project/main.c",
  });
  assert.equal(sharedEdit.content, "edited");
  await referenceCoursePath({
    ...scope,
    path: "same-project",
    targetPath: "project",
  });
  const nested = (await listAssignmentFiles(scope)).entries.find(
    (e) => e.path === "project/src",
  )!;
  await assert.rejects(
    moveAssignmentPath({
      ...scope,
      path: "project/src",
      destination: "same-project/src/inside",
      expectedRevision: nested.revision,
      explanation,
    }),
    /itself/,
  );
  const deleted = await deleteAssignmentPath({
    ...scope,
    path: "project/main.c",
    expectedRevision: sharedEdit.revision,
    explanation,
  });
  assert.equal(deleted.detached, true);
  assert.equal(
    (await readAssignmentFile({ ...shared, path: "project/main.c" })).content,
    "edited",
  );
  await assert.rejects(
    readAssignmentFile({ ...scope, path: "project/main.c" }),
    /not displayed/,
  );
  await fs.writeFile(
    safePath(`courses/${course.id}/files/project/from-terminal.txt`),
    "external edit",
  );
  const entries = (await listAssignmentFiles(scope)).entries;
  assert.ok(
    entries.find((e) => e.path === "project/from-terminal.txt" && e.shared),
  );
  assert.ok(
    entries.find(
      (e) => e.path === "same-project/from-terminal.txt" && e.shared,
    ),
  );
  assert.equal(
    await fs
      .stat(
        safePath(
          `courses/${course.id}/assignments/${scope.assignmentId}/project`,
        ),
      )
      .catch(() => null),
    null,
  );
});

test("references reject unsafe targets and collisions and retain removable missing references", async () => {
  const { course, scope } = await fixture();
  await writeAssignmentFile(
    { courseId: course.id, path: "notes.txt", content: "shared", explanation },
    true,
  );
  await writeAssignmentFile(
    { ...scope, path: "local.txt", content: "local", explanation },
    true,
  );
  await assert.rejects(
    referenceCoursePath({
      ...scope,
      path: "local.txt",
      targetPath: "notes.txt",
    }),
    /exists/,
  );
  for (const targetPath of [
    "../notes.txt",
    ".env",
    "node_modules/secret.txt",
  ]) {
    await assert.rejects(
      referenceCoursePath({ ...scope, path: "bad", targetPath }),
    );
  }
  await fs.symlink(
    safePath(`courses/${course.id}/files/notes.txt`),
    safePath(`courses/${course.id}/files/link.txt`),
  );
  await assert.rejects(
    referenceCoursePath({ ...scope, path: "bad", targetPath: "link.txt" }),
    /[Ss]ymlink/,
  );
  const other = await addCourse({ code: "OTHER", name: "Other" });
  await assert.rejects(
    referenceCoursePath({
      ...scope,
      courseId: other.id,
      path: "bad",
      targetPath: "notes.txt",
    }),
  );
  await referenceCoursePath({
    ...scope,
    path: "notes",
    targetPath: "notes.txt",
  });
  await assert.rejects(
    writeAssignmentFile(
      { ...scope, path: "notes/child.txt", content: "bad", explanation },
      true,
    ),
    /file reference/,
  );
  await fs.unlink(safePath(`courses/${course.id}/files/notes.txt`));
  const missing = (await listAssignmentFiles(scope)).entries.find(
    (e) => e.path === "notes",
  )!;
  assert.equal(missing.shared?.missing, true);
  await assert.rejects(
    writeAssignmentFile(
      { ...scope, path: "notes", content: "do not recreate", explanation },
      true,
    ),
    /ENOENT/,
  );
  await deleteAssignmentPath({
    ...scope,
    path: "notes",
    expectedRevision: missing.revision,
    explanation,
  });
  assert.ok(
    !(await listAssignmentFiles(scope)).entries.some((e) => e.path === "notes"),
  );
  assert.ok((await listAssignmentFiles(scope)).entries.every((e) => e.shared));
});

test("named assignment workspaces support files, exact edits, folders, persistent ordering, recovery and isolated paths", async () => {
  const { scope, assignment } = await fixture();
  const file = await writeAssignmentFile(
    {
      ...scope,
      path: "src/allocator.c",
      content: "int blocks = 1;\n",
      explanation,
    },
    true,
  );
  assert.match(file.absolutePath, /assignments/);
  const edited = await editAssignmentFile({
    ...scope,
    path: file.path,
    expectedRevision: file.revision,
    edits: [{ oldText: "blocks = 1", newText: "blocks = 2" }],
    explanation,
  });
  assert.equal(edited.content, "int blocks = 2;\n");
  assert.equal(await readPrivate(edited.recoveryPath!), "int blocks = 1;\n");
  await assert.rejects(
    () =>
      editAssignmentFile({
        ...scope,
        path: file.path,
        expectedRevision: edited.revision,
        edits: [{ oldText: "missing", newText: "x" }],
        explanation,
      }),
    /exactly once/,
  );
  await assert.rejects(
    () =>
      writeAssignmentFile({
        ...scope,
        path: file.path,
        content: "lost edit",
        expectedRevision: file.revision,
        explanation,
      }),
    /changed/,
  );
  await createAssignmentDirectory({ ...scope, path: "notes" });
  await moveAssignmentPath({
    ...scope,
    path: file.path,
    destination: "notes/allocator.c",
    expectedRevision: edited.revision,
    explanation,
  });
  let tree = await listAssignmentFiles(scope);
  const paths = tree.entries.map((e) => e.path).reverse();
  await reorderAssignmentFiles({
    ...scope,
    paths,
    expectedRevision: tree.revision,
  });
  tree = await listAssignmentFiles(scope);
  assert.deepEqual(
    tree.entries.map((e) => e.path),
    paths,
  );
  const notes = tree.entries.find((e) => e.path === "notes")!;
  const deleted = await deleteAssignmentPath({
    ...scope,
    path: "notes",
    expectedRevision: notes.revision,
    explanation,
  });
  assert.equal(
    await readPrivate(
      `courses/${scope.courseId}/files/assignments/${scope.assignmentId}/notes/allocator.c`,
    ),
    "int blocks = 2;\n",
  );
  assert.ok(
    !(await listAssignmentFiles(scope)).entries.some((e) =>
      e.path.startsWith("notes"),
    ),
  );
  assert.equal((await listAssignments(scope))[0].id, assignment.id);
  await deleteAssignment(scope);
  assert.equal((await listAssignments(scope)).length, 0);
  await assert.rejects(() => listAssignmentFiles(scope), /not found/);
  assert.ok(
    (await allEvidence(scope.courseId)).some(
      (s) => s.kind === "assignment" && s.text.includes("allocator"),
    ),
  );
});

test("path escape, symlinks, binary edits, overwritten names and stale folder deletion are rejected", async () => {
  const { scope } = await fixture();
  for (const name of [
    "../escape",
    "/tmp/escape",
    "a/../../b",
    "C:/secret",
    "a\\b",
    "a//b",
    ".git/../escape",
    "CON.txt",
  ]) {
    await assert.rejects(() =>
      writeAssignmentFile(
        { ...scope, path: name, content: "bad", explanation },
        true,
      ),
    );
  }
  const file = await writeAssignmentFile(
    {
      ...scope,
      path: "folder/readme.md",
      content: "repeated repeated",
      explanation,
    },
    true,
  );
  await assert.rejects(
    () =>
      editAssignmentFile({
        ...scope,
        path: file.path,
        expectedRevision: file.revision,
        edits: [{ oldText: "repeated", newText: "x" }],
        explanation,
      }),
    /exactly once/,
  );
  await assert.rejects(
    () =>
      writeAssignmentFile(
        { ...scope, path: file.path, content: "bad", explanation },
        true,
      ),
    /already exists/,
  );
  const folder = (await listAssignmentFiles(scope)).entries.find(
    (e) => e.path === "folder",
  )!;
  await writeAssignmentFile(
    { ...scope, path: "folder/new.txt", content: "new", explanation },
    true,
  );
  await assert.rejects(
    () =>
      deleteAssignmentPath({
        ...scope,
        path: folder.path,
        expectedRevision: folder.revision,
        explanation,
      }),
    /changed/,
  );
  const binary = await writeAssignmentFile(
    {
      ...scope,
      path: "data.bin",
      content: Buffer.from([0, 255, 1]).toString("base64"),
      encoding: "base64",
      explanation,
    },
    true,
  );
  assert.equal(binary.content, undefined);
  await assert.rejects(
    () =>
      editAssignmentFile({
        ...scope,
        path: binary.path,
        expectedRevision: binary.revision,
        edits: [{ oldText: "x", newText: "y" }],
        explanation,
      }),
    /Binary/,
  );
  const outsider = await createAssignment({
    courseId: scope.courseId,
    title: "Other",
  });
  await assert.rejects(
    () =>
      readAssignmentFile({
        ...scope,
        assignmentId: outsider.id,
        path: file.path,
      }),
    /not displayed/,
  );
  await fs.symlink(
    os.tmpdir(),
    safePath(
      `courses/${scope.courseId}/files/assignments/${scope.assignmentId}/escape`,
    ),
  );
  await assert.rejects(
    () =>
      writeAssignmentFile(
        { ...scope, path: "escape/leak", content: "bad", explanation },
        true,
      ),
    /Symlinks/,
  );
  assert.ok(
    !(await listAssignmentFiles(scope)).entries.some(
      (e) => e.path === "escape",
    ),
  );
});

test("migration preserves both sides of a storage collision without overwriting", async () => {
  const { scope } = await fixture();
  const original = safePath(
    `courses/${scope.courseId}/assignments/${scope.assignmentId}`,
  );
  const canonical = safePath(
    `courses/${scope.courseId}/files/assignments/${scope.assignmentId}`,
  );
  await fs.mkdir(original, { recursive: true });
  await fs.mkdir(canonical, { recursive: true });
  await fs.writeFile(path.join(original, "answer.txt"), "old work");
  await fs.writeFile(path.join(canonical, "answer.txt"), "new work");
  await assert.rejects(listAssignmentFiles(scope), /conflict resolution/);
  assert.equal(
    await fs.readFile(path.join(original, "answer.txt"), "utf8"),
    "old work",
  );
  assert.equal(
    await fs.readFile(path.join(canonical, "answer.txt"), "utf8"),
    "new work",
  );
});

test("concurrent writers cannot silently overwrite one another", async () => {
  const { scope } = await fixture();
  const file = await writeAssignmentFile(
    { ...scope, path: "answer.py", content: "x=1", explanation },
    true,
  );
  const results = await Promise.allSettled(
    [2, 3].map((x) =>
      writeAssignmentFile({
        ...scope,
        path: file.path,
        content: `x=${x}`,
        expectedRevision: file.revision,
        explanation,
      }),
    ),
  );
  assert.equal(results.filter((r) => r.status === "fulfilled").length, 1);
  assert.equal(results.filter((r) => r.status === "rejected").length, 1);
});

test("assignment-created files belong to course Files, can be reused, and survive removal from any view", async () => {
  const { scope } = await fixture();
  const first = await writeAssignmentFile(
    {
      ...scope,
      path: "code/main.py",
      content: "print('reusable')",
      explanation,
    },
    true,
  );
  const targetPath = `assignments/${scope.assignmentId}/code/main.py`;
  assert.equal(
    first.workspacePath,
    `courses/${scope.courseId}/files/${targetPath}`,
  );
  assert.equal(first.shared?.path, targetPath);
  assert.ok(
    (await listAssignmentFiles({ courseId: scope.courseId })).entries.some(
      (e) => e.path === targetPath,
    ),
  );
  assert.ok(
    (await allEvidence(scope.courseId)).some(
      (s) => s.kind === "file" && s.text.includes("reusable"),
    ),
  );
  const other = await createAssignment({
    courseId: scope.courseId,
    title: "Reuse",
  });
  const second = { courseId: scope.courseId, assignmentId: other.id };
  await referenceCoursePath({ ...second, path: "main.py", targetPath });
  await deleteAssignmentPath({
    ...scope,
    path: "code/main.py",
    expectedRevision: first.revision,
    explanation,
  });
  assert.equal(
    (await readAssignmentFile({ ...second, path: "main.py" })).content,
    first.content,
  );
  await writeAssignmentFile({
    ...second,
    path: "main.py",
    content: "print('updated')",
    expectedRevision: first.revision,
    explanation,
  });
  assert.equal(
    (await readAssignmentFile({ courseId: scope.courseId, path: targetPath }))
      .content,
    "print('updated')",
  );
  await deleteAssignment(second);
  assert.equal(
    (await readAssignmentFile({ courseId: scope.courseId, path: targetPath }))
      .content,
    "print('updated')",
  );
});

test("legacy assignment folders migrate with a backup, stable history and idempotent course listing", async () => {
  const { scope } = await fixture();
  const oldRoot = safePath(
    `courses/${scope.courseId}/assignments/${scope.assignmentId}`,
  );
  await fs.mkdir(path.join(oldRoot, "src"), { recursive: true });
  await fs.writeFile(path.join(oldRoot, "src/main.py"), "old source");
  const { recordFileChange } = await import("../src/lib/file-history");
  const { mutate } = await import("../src/lib/store");
  await mutate(() =>
    recordFileChange(
      scope.courseId,
      `assignments/${scope.assignmentId}/src/main.py`,
      null,
      Buffer.from("old source"),
      { actor: "agent", explanation, assignmentId: scope.assignmentId },
    ),
  );
  const files = await listAssignmentFiles({ courseId: scope.courseId });
  assert.ok(
    files.entries.some(
      (e) => e.path === `assignments/${scope.assignmentId}/src/main.py`,
    ),
  );
  assert.equal(
    (await readAssignmentFile({ ...scope, path: "src/main.py" })).content,
    "old source",
  );
  const history = await getFileHistory({ ...scope, path: "src/main.py" });
  assert.ok("entries" in history && history.entries);
  assert.deepEqual(
    history.entries.map((e) => e.action),
    ["moved", "created"],
  );
  const backups = await fs.readdir(safePath(".trash/assignment-storage"));
  assert.equal(
    await readPrivate(
      `.trash/assignment-storage/${backups[0]}/${scope.courseId}/${scope.assignmentId}/src/main.py`,
    ),
    "old source",
  );
  assert.equal(await fs.stat(oldRoot).catch(() => null), null);
  await listAssignmentFiles(scope);
  assert.equal(
    (await fs.readdir(safePath(".trash/assignment-storage"))).length,
    1,
  );
});

test("assignment learning is readable RAG and selectable exam evidence with course isolation and citations", async () => {
  const { scope } = await fixture();
  await saveSource({
    courseId: scope.courseId,
    title: "Allocator lecture notes",
    text: "An allocator uses a free list to track unused memory blocks and splits large blocks to satisfy smaller requests.",
  });
  const source = (
    await searchMemory(scope.courseId, "allocator free list")
  ).find((s) => s.kind === "note")!;
  await writeAssignmentFile(
    {
      ...scope,
      path: "allocator.md",
      content: "# Allocator\n\nKeep a free list.",
      explanation,
    },
    true,
  );
  const record = await recordAssignmentLearning({
    ...scope,
    title: "Allocator reasoning",
    markdown:
      "A free list tracks unused blocks. I separated block splitting from coalescing so the allocator is easier to inspect. No execution was performed.",
    paths: ["allocator.md"],
    sourceIds: [source.id],
    gaps: ["Generated draft: code has not been executed."],
  });
  assert.match(await readPrivate(record.path), /Course evidence/);
  const found = (
    await searchMemory(scope.courseId, "allocator splitting coalescing", 30)
  ).find((s) => s.path === record.path && s.text.includes("coalescing"))!;
  assert.equal(found.kind, "assignment");
  assert.match(found.text, /not primary/);
  const context = await readCourseContext({
    courseId: scope.courseId,
    sourceId: found.id,
  });
  assert.ok(JSON.stringify(context).includes("No execution was performed"));
  const prepared = await prepareExam(scope.courseId, "allocator free list");
  assert.ok(prepared.assignmentEvidence.some((s) => s.path === record.path));
  const input = {
    courseId: scope.courseId,
    prompt: "Practice allocator reasoning",
    scope: { mode: "dates", from: "2026-09-01", through: "2026-09-30" },
    questionCount: 1,
    questionTypes: ["short_answer"],
    contentSourceIds: [found.id],
  };
  assert.equal(
    (await previewExamScope(input)).assignmentEvidence[0].id,
    found.id,
  );
  const exam = await queueExam(input);
  await completeJob(exam.id, {
    title: "Allocator practice",
    instructions: "Explain your reasoning.",
    format: { rationale: "Practice mix", inferred: true, citations: [] },
    questions: [
      {
        type: "short_answer",
        topic: "Allocator",
        prompt: "Why separate splitting from coalescing?",
        points: 2,
        answer: "To inspect distinct operations.",
        explanation:
          "The assignment record separates these operations to make reasoning easier.",
        rubric: [{ criterion: "Explains distinct operations", points: 2 }],
        citations: [found.id],
      },
    ],
    gaps: ["Generated assignment evidence was used."],
  });
  const other = await addCourse({ code: "OTHER", name: "Other" });
  await assert.rejects(
    () => queueExam({ ...input, courseId: other.id }),
    /other-course/,
  );
});

test("legacy study drafts remain listed and completion contributes to assignment memory", async () => {
  const { scope } = await fixture();
  const job = await createJob({
    courseId: scope.courseId,
    kind: "assignment",
    prompt: "Explain allocator metadata",
  });
  await completeJob(job.id, {
    markdown:
      "Allocator metadata records block sizes and allocation state for inspection.",
    citations: [],
    gaps: ["No instructor source imported yet."],
  });
  assert.ok(
    (await listAssignments(scope)).some((a) => a.legacyJobId === job.id),
  );
  const histories = await getFileHistory({ courseId: scope.courseId });
  assert.ok("files" in histories && histories.files);
  const legacy = histories.files.find(
    (file) => file.path === `files/assignments/${job.id}/draft.md`,
  )!;
  assert.ok(legacy);
  const timeline = await getFileHistory({
    courseId: scope.courseId,
    historyId: legacy.historyId,
  });
  assert.ok("entries" in timeline && timeline.entries);
  assert.equal(timeline.entries[0].action, "created");
  assert.equal(timeline.entries[0].context, job.prompt);
  assert.ok(
    (await searchMemory(scope.courseId, "allocator metadata")).some(
      (s) => s.kind === "assignment",
    ),
  );
});

test("HTTP assets allow separate downloads but render HTML inert and reject cross-origin writes and UI assignment creation", async () => {
  const { scope } = await fixture();
  await writeAssignmentFile(
    {
      ...scope,
      path: "demo.html",
      content: "<h1>Hello</h1><script>alert(1)</script>",
      explanation,
    },
    true,
  );
  const query = new URLSearchParams({
    ...scope,
    path: "demo.html",
    mode: "asset",
    download: "1",
  });
  const response = await GET(
    new NextRequest(`http://localhost:3000/api/assignments?${query}`, {
      headers: { host: "localhost:3000" },
    }),
  );
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type")!, /text\/plain/);
  assert.match(response.headers.get("content-disposition")!, /attachment/);
  assert.match(response.headers.get("content-security-policy")!, /sandbox/);
  const cross = await POST(
    new NextRequest("http://localhost:3000/api/assignments", {
      method: "POST",
      headers: {
        host: "localhost:3000",
        origin: "https://evil.test",
        "content-type": "application/json",
      },
      body: JSON.stringify({
        action: "create",
        data: { ...scope, path: "x", content: "x", explanation },
      }),
    }),
  );
  assert.equal(cross.status, 400);
  const create = await POST(
    new NextRequest("http://localhost:3000/api/assignments", {
      method: "POST",
      headers: {
        host: "localhost:3000",
        origin: "http://localhost:3000",
        "content-type": "application/json",
      },
      body: JSON.stringify({
        action: "create-assignment",
        data: { courseId: scope.courseId, title: "No UI create" },
      }),
    }),
  );
  assert.equal(create.status, 400);
});
