import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { NextRequest } from "next/server";
import { addCourse, safePath } from "../src/lib/store";
import { createAssignment } from "../src/lib/assignments";
import {
  writeAssignmentFile,
  readAssignmentFile,
  editAssignmentFile,
  moveAssignmentPath,
  deleteAssignmentPath,
  referenceCoursePath,
  listAssignmentFiles,
} from "../src/lib/assignment-files";
import {
  getFileHistory,
  downloadFileRevision,
  checkpointCourseFiles,
} from "../src/lib/file-history";
import { fileDiff } from "../src/lib/file-diff";
import { GET } from "../src/app/api/file-history/route";
import { POST } from "../src/app/api/course-files/route";
let root: string;
const previous = process.env.COURSE_CAPTAIN_WORKSPACE;
beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "cc-history-"));
  process.env.COURSE_CAPTAIN_WORKSPACE = root;
});
afterEach(async () => {
  if (previous) process.env.COURSE_CAPTAIN_WORKSPACE = previous;
  else delete process.env.COURSE_CAPTAIN_WORKSPACE;
  assert.ok(root.startsWith(path.join(os.tmpdir(), "cc-history-")));
  await fs.rm(root, { recursive: true, force: true });
});
const explanation =
  "Implement the loop invariant from the assignment requirements.";
async function fixture() {
  const course = await addCourse({ code: "HIST", name: "File history" });
  return { course, scope: { courseId: course.id, path: "main.py" } };
}
async function timeline(input: unknown) {
  const result = await getFileHistory(input);
  assert.ok("entries" in result && result.entries);
  return result;
}

test("every tracked save records creation, context, actor, sequence and exact diff", async () => {
  const { scope } = await fixture();
  const created = await writeAssignmentFile(
    {
      ...scope,
      content: "x = 1\nprint(x)\n",
      explanation,
      context: "Assignment 2: loop invariant.",
    },
    true,
  );
  const updated = await editAssignmentFile({
    ...scope,
    expectedRevision: created.revision,
    edits: [{ oldText: "x = 1", newText: "x = 2" }],
    explanation: "Fix off-by-one after checking the rubric.",
    context: "Rubric section 3 and lecture 4.",
  });
  await writeAssignmentFile(
    {
      ...scope,
      content: updated.content!,
      expectedRevision: updated.revision,
      explanation: "Save the same contents after reviewing the result.",
    },
    false,
    "user",
  );
  const history = await timeline(scope);
  assert.equal(history.total, 3);
  assert.deepEqual(
    history.entries!.map((e) => e.sequence),
    [3, 2, 1],
  );
  assert.equal(history.entries![2].action, "created");
  assert.equal(history.entries![2].before, null);
  assert.equal(history.entries![0].actor, "user");
  assert.equal(history.entries![1].context, "Rubric section 3 and lecture 4.");
  assert.ok(
    history.entries!.every((e) => Number.isFinite(Date.parse(e.timestamp))),
  );
  const detail = await getFileHistory({
    courseId: scope.courseId,
    historyId: history.historyId,
    revisionId: history.entries![1].id,
  });
  assert.ok("diff" in detail && detail.diff);
  assert.ok(
    detail.diff.lines.some(
      (line) => line.kind === "remove" && line.text === "x = 1\n",
    ),
  );
  assert.ok(
    detail.diff.lines.some(
      (line) => line.kind === "add" && line.text === "x = 2\n",
    ),
  );
  const before = await downloadFileRevision(
    {
      courseId: scope.courseId,
      historyId: history.historyId,
      revisionId: history.entries![1].id,
    },
    "before",
  );
  assert.equal(before.bytes.toString(), "x = 1\nprint(x)\n");
  await assert.rejects(
    writeAssignmentFile({
      ...scope,
      content: "stale",
      expectedRevision: created.revision,
      explanation,
    }),
    /changed/,
  );
  assert.equal((await timeline(scope)).total, 3);
});
test("references share history, moves preserve lineage, and recreated old paths get separate identities", async () => {
  const { course, scope } = await fixture();
  const assignment = await createAssignment({
    courseId: course.id,
    title: "Shared implementation",
  });
  await writeAssignmentFile(
    { ...scope, content: "original", explanation },
    true,
  );
  await referenceCoursePath({
    courseId: course.id,
    assignmentId: assignment.id,
    path: "alias.py",
    targetPath: "main.py",
  });
  const alias = {
    courseId: course.id,
    assignmentId: assignment.id,
    path: "alias.py",
  };
  const file = await readAssignmentFile(alias);
  await writeAssignmentFile({
    ...alias,
    content: "shared edit",
    expectedRevision: file.revision,
    explanation,
  });
  const history = await timeline(scope);
  assert.equal((await timeline(alias)).historyId, history.historyId);
  assert.equal(history.entries![0].assignmentId, assignment.id);
  const shared = await readAssignmentFile(scope);
  await moveAssignmentPath({
    ...scope,
    destination: "renamed.py",
    expectedRevision: shared.revision,
    explanation,
  });
  assert.equal(
    (await timeline({ ...scope, path: "renamed.py" })).historyId,
    history.historyId,
  );
  assert.equal((await timeline(scope)).entries![0].action, "moved");
  await writeAssignmentFile(
    { ...scope, content: "different file", explanation },
    true,
  );
  assert.notEqual((await timeline(scope)).historyId, history.historyId);
  const renamed = { ...scope, path: "renamed.py" };
  await deleteAssignmentPath({
    ...renamed,
    expectedRevision: shared.revision,
    explanation,
  });
  const deleted = await timeline(renamed);
  assert.equal(deleted.entries![0].action, "deleted");
  assert.equal(deleted.entries![0].after, null);
  assert.equal(deleted.total, 4);
});
test("legacy files and outside changes are explicit baselines/observations", async () => {
  const { scope, course } = await fixture();
  await fs.mkdir(safePath(`courses/${course.id}/files`), { recursive: true });
  const target = safePath(`courses/${course.id}/files/main.py`);
  await fs.writeFile(target, "legacy version");
  const baseline = await timeline(scope);
  assert.equal(baseline.entries![0].action, "baseline");
  assert.equal(baseline.entries![0].actor, "external");
  await fs.writeFile(target, "outside edit");
  const changed = await timeline(scope);
  assert.equal(changed.total, 2);
  assert.equal(changed.entries![0].action, "observed");
  assert.match(changed.entries![0].explanation, /unknown/);
  await checkpointCourseFiles(course.id, {
    actor: "terminal",
    explanation: "Run a generator",
    command: "make generate",
  });
  assert.equal((await timeline(scope)).total, 2);
});
test("history is course-scoped, paginated, binary-safe and integrity-checked", async () => {
  const { scope, course } = await fixture();
  const binary = Buffer.from([0, 255, 17]);
  const file = await writeAssignmentFile(
    {
      ...scope,
      path: "image.bin",
      content: binary.toString("base64"),
      encoding: "base64",
      explanation,
    },
    true,
  );
  const history = await timeline({ ...scope, path: "image.bin", limit: 1 });
  const detailInput = {
    courseId: course.id,
    historyId: history.historyId,
    revisionId: history.entries![0].id,
  };
  const detail = await getFileHistory(detailInput);
  assert.ok("diff" in detail && detail.diff);
  assert.match(detail.diff.message, /Binary/);
  assert.deepEqual(
    (await downloadFileRevision(detailInput, "after")).bytes,
    binary,
  );
  const other = await addCourse({ code: "OTHER", name: "Other" });
  await assert.rejects(
    getFileHistory({ ...detailInput, courseId: other.id }),
    /outside/,
  );
  await assert.rejects(getFileHistory({ ...scope, path: "../state.json" }));
  const response = await GET(
    new NextRequest(
      `http://127.0.0.1:3000/api/file-history?courseId=${course.id}&historyId=${history.historyId}&revisionId=${history.entries![0].id}&snapshot=after`,
      { headers: { host: "127.0.0.1:3000" } },
    ),
  );
  assert.equal(response.status, 200);
  assert.match(response.headers.get("Content-Disposition")!, /attachment/);
  assert.equal(
    (
      await GET(
        new NextRequest(
          `http://127.0.0.1:3000/api/file-history?courseId=${course.id}`,
          {
            headers: {
              host: "127.0.0.1:3000",
              origin: "https://evil.example",
              "sec-fetch-site": "cross-site",
            },
          },
        ),
      )
    ).status,
    400,
  );
  await writeAssignmentFile({
    ...scope,
    path: "image.bin",
    content: "new",
    expectedRevision: file.revision,
    explanation,
  });
  const page = await timeline({ ...scope, path: "image.bin", limit: 1 });
  assert.equal(page.nextOffset, 1);
  assert.equal(
    (await timeline({ ...scope, path: "image.bin", limit: 1, offset: 1 }))
      .entries![0].action,
    "created",
  );
  await fs.writeFile(
    safePath(
      `courses/${course.id}/history/blobs/${history.entries![0].after!.hash}`,
    ),
    "tampered",
  );
  await assert.rejects(downloadFileRevision(detailInput, "after"), /integrity/);
});
test("UI saves are labeled user changes and directory moves/deletes retain child histories", async () => {
  const { course } = await fixture();
  const scope = { courseId: course.id };
  const response = await POST(
    new NextRequest("http://127.0.0.1:3000/api/course-files", {
      method: "POST",
      headers: {
        host: "127.0.0.1:3000",
        origin: "http://127.0.0.1:3000",
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        action: "create",
        data: { ...scope, path: "src/a.txt", content: "A", explanation },
      }),
    }),
  );
  assert.equal(response.status, 200);
  assert.equal(
    (await timeline({ ...scope, path: "src/a.txt" })).entries![0].actor,
    "user",
  );
  await writeAssignmentFile(
    { ...scope, path: "src/b.txt", content: "B", explanation },
    true,
  );
  let folder = (await listAssignmentFiles(scope)).entries.find(
    (e) => e.path === "src",
  )!;
  await moveAssignmentPath({
    ...scope,
    path: "src",
    destination: "lib",
    expectedRevision: folder.revision,
    explanation,
  });
  folder = (await listAssignmentFiles(scope)).entries.find(
    (e) => e.path === "lib",
  )!;
  await deleteAssignmentPath({
    ...scope,
    path: "lib",
    expectedRevision: folder.revision,
    explanation,
  });
  for (const name of ["a.txt", "b.txt"])
    assert.deepEqual(
      (await timeline({ ...scope, path: `lib/${name}` })).entries!.map(
        (e) => e.action,
      ),
      ["deleted", "moved", "created"],
    );
});
test("diff handles CRLF, missing final newlines and bounded large replacements", () => {
  const diff = fileDiff(
    Buffer.from("first\r\nlast"),
    Buffer.from("first\r\nnew\nlast\n"),
  );
  assert.ok(
    diff.lines.some((e) => e.kind === "context" && e.text === "first\r\n"),
  );
  assert.ok(diff.lines.some((e) => e.kind === "remove" && e.text === "last"));
  assert.ok(diff.lines.some((e) => e.kind === "add" && e.text === "new\n"));
  const coarse = fileDiff(
    Buffer.from("a\n".repeat(1500)),
    Buffer.from("b\n".repeat(1500)),
  );
  assert.equal(coarse.coarse, true);
  assert.equal(coarse.lines.length, 3000);
  assert.match(fileDiff(Buffer.alloc(1_100_000, "a"), null).message, /Large/);
});

test("moving onto a formerly deleted path keeps both file histories browsable", async () => {
  const { scope } = await fixture();
  const a = await writeAssignmentFile(
    { ...scope, content: "old destination", explanation },
    true,
  );
  const oldHistory = await timeline(scope);
  await deleteAssignmentPath({
    ...scope,
    expectedRevision: a.revision,
    explanation,
  });
  const b = await writeAssignmentFile(
    { ...scope, path: "other.py", content: "new destination", explanation },
    true,
  );
  await moveAssignmentPath({
    ...scope,
    path: "other.py",
    destination: "main.py",
    expectedRevision: b.revision,
    explanation,
  });
  const list = await getFileHistory({ courseId: scope.courseId });
  assert.ok("files" in list && list.files);
  assert.equal(list.files.length, 2);
  assert.ok(
    list.files.some(
      (file) => file.historyId === oldHistory.historyId && file.deleted,
    ),
  );
  const preserved = await getFileHistory({
    courseId: scope.courseId,
    historyId: oldHistory.historyId,
  });
  assert.ok(
    "entries" in preserved && preserved.entries?.[0].action === "deleted",
  );
});
