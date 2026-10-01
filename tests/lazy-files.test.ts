import { test, beforeEach, afterEach, mock } from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  addCourse,
  safePath,
  readPrivate,
  writePrivate,
} from "../src/lib/store";
import { createAssignment, deleteAssignment } from "../src/lib/assignments";
import { listCourseCommands } from "../src/lib/course-terminal";
import { randomUUID } from "node:crypto";
import { saveReferences } from "../src/lib/assignment-references";
import {
  listAssignmentDirectory,
  listAssignmentFiles,
  assignmentPathRevision,
  deleteAssignmentPath,
  reorderAssignmentFiles,
  readAssignmentFile,
} from "../src/lib/assignment-files";

let root: string;
const previous = process.env.COURSE_CAPTAIN_WORKSPACE;
beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "cc-lazy-files-"));
  process.env.COURSE_CAPTAIN_WORKSPACE = root;
});
afterEach(async () => {
  mock.restoreAll();
  if (previous === undefined) delete process.env.COURSE_CAPTAIN_WORKSPACE;
  else process.env.COURSE_CAPTAIN_WORKSPACE = previous;
  await fs.rm(root, { recursive: true, force: true });
});
async function fixture() {
  const course = await addCourse({ code: "LAZY", name: "Lazy listings" });
  const courseId = course.id;
  await writePrivate(`courses/${courseId}/files/project/src/main.txt`, "first");
  await writePrivate(`courses/${courseId}/files/project/readme.txt`, "notes");
  await writePrivate(
    `courses/${courseId}/files/unrelated/deep/other.txt`,
    "other",
  );
  const assignment = await createAssignment({ courseId, title: "Scoped work" });
  const assignmentId = assignment.id;
  await saveReferences(courseId, assignmentId, [
    {
      path: "work",
      targetPath: "project",
      type: "directory",
      createdAt: new Date().toISOString(),
      excludedPaths: [],
    },
  ]);
  return { courseId, assignmentId };
}
test("top-level listings do not descend, and assignment expansion touches only the referenced directory", async () => {
  const scope = await fixture();
  const readdir = mock.method(fs, "readdir", fs.readdir);
  const before = await readPrivate("state.json");
  const top = await listAssignmentDirectory({ courseId: scope.courseId });
  assert.deepEqual(
    top.entries.map((e) => e.path),
    ["project", "unrelated"],
  );
  assert.equal(readdir.mock.callCount(), 1);
  readdir.mock.resetCalls();
  const assignment = await listAssignmentDirectory(scope);
  assert.deepEqual(
    assignment.entries.map((e) => e.path),
    ["work"],
  );
  assert.equal(assignment.entries[0].revision, "");
  assert.ok(
    readdir.mock.calls.every(
      (c) => !String(c.arguments[0]).endsWith("project"),
    ),
  );
  readdir.mock.resetCalls();
  const folder = await listAssignmentDirectory({ ...scope, directory: "work" });
  assert.deepEqual(
    folder.entries.map((e) => e.path),
    ["work/readme.txt", "work/src"],
  );
  assert.deepEqual(
    readdir.mock.calls.map((c) => c.arguments[0]),
    [safePath(`courses/${scope.courseId}/files/project`)],
  );
  assert.equal(await readPrivate("state.json"), before);
  const deep = await listAssignmentDirectory({
    ...scope,
    directory: "work/src",
  });
  assert.deepEqual(
    deep.entries.map((e) => e.path),
    ["work/src/main.txt"],
  );
  await assert.rejects(
    listAssignmentDirectory({ ...scope, directory: "unrelated" }),
    /not displayed/,
  );
  await assert.rejects(
    listAssignmentDirectory({ ...scope, directory: "../project" }),
  );
});
test("folder listings never read file contents; mutation hashes are reused and both detect external writes", async () => {
  const scope = await fixture();
  const full = safePath(`courses/${scope.courseId}/files/project/src/main.txt`);
  const read = mock.method(fs, "readFile", fs.readFile);
  const count = () =>
    read.mock.calls.filter((c) => c.arguments[0] === full).length;
  const first = await listAssignmentDirectory({
    ...scope,
    directory: "work/src",
  });
  await listAssignmentDirectory({ ...scope, directory: "work/src" });
  assert.equal(count(), 0);
  const snapshot = await assignmentPathRevision({
    ...scope,
    path: "work/src/main.txt",
  });
  await assignmentPathRevision({ ...scope, path: "work/src/main.txt" });
  assert.equal(count(), 1);
  const old = await fs.stat(full);
  await new Promise((resolve) => setTimeout(resolve, 15));
  await fs.writeFile(full, "later");
  await fs.utimes(full, old.atime, old.mtime);
  const updated = await listAssignmentDirectory({
    ...scope,
    directory: "work/src",
  });
  assert.equal(count(), 1);
  assert.notEqual(first.entries[0].revision, updated.entries[0].revision);
  const current = await assignmentPathRevision({
    ...scope,
    path: "work/src/main.txt",
  });
  assert.equal(count(), 2);
  assert.notEqual(snapshot.revision, current.revision);
  const file = await readAssignmentFile({
    ...scope,
    path: "work/src/main.txt",
  });
  assert.equal(file.metadataRevision, updated.entries[0].revision);
  await assert.rejects(
    deleteAssignmentPath({
      ...scope,
      path: "work/src/main.txt",
      expectedRevision: first.entries[0].revision,
      explanation: "Remove this test file from the view.",
    }),
    /changed since/,
  );
});

test("recursive assignment tools stay scoped and folder mutations reject stale descendant revisions", async () => {
  const scope = await fixture();
  const readdir = mock.method(fs, "readdir", fs.readdir);
  const full = await listAssignmentFiles(scope);
  assert.ok(full.entries.some((e) => e.path === "work/src/main.txt"));
  assert.ok(
    readdir.mock.calls.every(
      (c) => !String(c.arguments[0]).includes("unrelated"),
    ),
  );
  const snapshot = await assignmentPathRevision({
    courseId: scope.courseId,
    path: "project",
  });
  await writePrivate(
    `courses/${scope.courseId}/files/project/src/main.txt`,
    "changed",
  );
  await assert.rejects(
    deleteAssignmentPath({
      courseId: scope.courseId,
      path: "project",
      expectedRevision: snapshot.revision,
      explanation: "Remove the test project directory.",
    }),
    /changed since/,
  );
});
test("shallow reorder preserves unloaded paths and hidden/missing references stay safe", async () => {
  const scope = await fixture();
  const initial = await listAssignmentDirectory({
    ...scope,
    directory: "work",
  });
  await reorderAssignmentFiles({
    ...scope,
    directory: "work",
    paths: ["work/src", "work/readme.txt"],
    expectedRevision: initial.revision,
  });
  assert.deepEqual(
    (
      await listAssignmentDirectory({ ...scope, directory: "work" })
    ).entries.map((e) => e.path),
    ["work/src", "work/readme.txt"],
  );
  await saveReferences(scope.courseId, scope.assignmentId, [
    {
      path: "work",
      targetPath: "project",
      type: "directory",
      createdAt: new Date().toISOString(),
      excludedPaths: ["work/src"],
    },
    {
      path: "missing",
      targetPath: "gone",
      type: "directory",
      createdAt: new Date().toISOString(),
      excludedPaths: [],
    },
  ]);
  const top = await listAssignmentDirectory(scope);
  assert.equal(
    top.entries.find((e) => e.path === "missing")?.shared?.missing,
    true,
  );
  assert.deepEqual(
    (
      await listAssignmentDirectory({ ...scope, directory: "work" })
    ).entries.map((e) => e.path),
    ["work/readme.txt"],
  );
  await assert.rejects(
    listAssignmentDirectory({ ...scope, directory: "work/src" }),
    /not displayed/,
  );
  await assert.rejects(
    reorderAssignmentFiles({
      ...scope,
      directory: "work",
      paths: ["work/readme.txt"],
      expectedRevision: initial.revision,
    }),
    /changed/,
  );
});

test("scope cache avoids parsing unchanged evidence and observes assignment removal", async () => {
  const scope = await fixture();
  const read = mock.method(fs, "readFile", fs.readFile);
  const stateReads = () =>
    read.mock.calls.filter((c) => String(c.arguments[0]).endsWith("state.json"))
      .length;
  await listAssignmentDirectory(scope);
  await listAssignmentDirectory({ ...scope, directory: "work" });
  await listAssignmentDirectory({ courseId: scope.courseId });
  assert.equal(stateReads(), 1);
  await deleteAssignment(scope);
  await assert.rejects(listAssignmentDirectory(scope), /Assignment not found/);
});
test("command history reads workspace state once and no output logs regardless of history size", async () => {
  const { courseId } = await fixture();
  const newest = [];
  for (let i = 0; i < 35; i++) {
    const id = randomUUID();
    newest.unshift(id);
    await writePrivate(
      `courses/${courseId}/.runtime/commands/${id}/run.json`,
      JSON.stringify({
        id,
        courseId,
        status: "completed",
        createdAt: new Date(1700000000000 + i * 1000).toISOString(),
      }),
    );
  }
  const read = mock.method(fs, "readFile", fs.readFile);
  const runs = await listCourseCommands(courseId);
  assert.deepEqual(
    runs.map((r) => r.id),
    newest.slice(0, 30),
  );
  assert.equal(
    read.mock.calls.filter((c) => String(c.arguments[0]).endsWith("state.json"))
      .length,
    1,
  );
  assert.equal(
    read.mock.calls.filter((c) => String(c.arguments[0]).endsWith("output.log"))
      .length,
    0,
  );
  assert.ok(runs.every((r) => !("output" in r)));
});
