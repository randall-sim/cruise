import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import path from "node:path";
import os from "node:os";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { setTimeout as delay } from "node:timers/promises";
import { NextRequest } from "next/server";
import {
  addCourse,
  readPrivate,
  safePath,
  writePrivate,
} from "../src/lib/store";
import { createAssignment } from "../src/lib/assignments";
import {
  listAssignmentFiles,
  readAssignmentFile,
  writeAssignmentFile,
} from "../src/lib/assignment-files";
import { indexCourseFiles } from "../src/lib/course-file-index";
import { getFileHistory } from "../src/lib/file-history";
import {
  cloneCourseRepository,
  startCourseCommand,
  readCourseCommand,
  stopCourseCommand,
  listCourseCommands,
} from "../src/lib/course-terminal";
import { searchMemory } from "../src/lib/memory";
import { readCourseContext } from "../src/lib/course-context";
import { previewExamScope, queueExam } from "../src/lib/exams";
import { POST as commandPost } from "../src/app/api/course-commands/route";
import { GET as filesGet } from "../src/app/api/course-files/route";
const exec = promisify(execFile);
let root: string;
let runs: { courseId: string; runId: string }[] = [];
const original = process.env.COURSE_CAPTAIN_WORKSPACE;
beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "cc-course-files-"));
  process.env.COURSE_CAPTAIN_WORKSPACE = root;
  runs = [];
});
afterEach(async () => {
  for (const run of runs) {
    await stopCourseCommand(run).catch(() => {});
    await finished(run).catch(() => {});
  }
  if (original) process.env.COURSE_CAPTAIN_WORKSPACE = original;
  else delete process.env.COURSE_CAPTAIN_WORKSPACE;
  assert.ok(root.startsWith(path.join(os.tmpdir(), "cc-course-files-")));
  await fs.rm(root, { recursive: true, force: true });
});
async function finished(run: {
  courseId: string;
  id?: string;
  runId?: string;
}) {
  const identity = { courseId: run.courseId, runId: run.runId || run.id! };
  if (!runs.some((r) => r.runId === identity.runId)) runs.push(identity);
  for (let i = 0; i < 150; i++) {
    const result = await readCourseCommand(identity);
    if (!["queued", "running"].includes(result.status)) return result;
    await delay(100);
  }
  throw Error("Command failed to finish during test");
}
const explanation =
  "Set up shared course code that can be reused and tested across assignments.";

test("command history checkpoints capture net file changes with run context even after a failed command", async () => {
  const course = await addCourse({ code: "CMDHIST", name: "Command history" });
  await writeAssignmentFile(
    {
      courseId: course.id,
      path: "tracked.txt",
      content: "before",
      explanation,
    },
    true,
  );
  const run = await startCourseCommand({
    courseId: course.id,
    command:
      "node -e \"require('fs').writeFileSync('tracked.txt','after');require('fs').writeFileSync('generated.txt','generated');process.exit(7)\"",
    purpose: "Generate fixtures for assignment 3",
  });
  const result = await finished(run);
  assert.equal(result.exitCode, 7);
  const history = await getFileHistory({
    courseId: course.id,
    path: "tracked.txt",
  });
  assert.ok("entries" in history && history.entries);
  assert.equal(history.total, 2);
  assert.equal(history.entries[0].actor, "terminal");
  assert.equal(history.entries[0].runId, run.id);
  assert.equal(history.entries[0].command, run.command);
  assert.match(history.entries[0].context!, /exit 7/);
  const detail = await getFileHistory({
    courseId: course.id,
    historyId: history.historyId,
    revisionId: history.entries[0].id,
  });
  assert.ok(
    "diff" in detail &&
      detail.diff?.lines.some((e) => e.kind === "add" && e.text === "after"),
  );
  const generated = await getFileHistory({
    courseId: course.id,
    path: "generated.txt",
  });
  assert.ok("entries" in generated && generated.entries?.[0].runId === run.id);
});
test("clone once, reuse a real Git repository across assignments, run tests and retrieve sources/results for exams", async () => {
  const course = await addCourse({ code: "CS", name: "Shared workspace" });
  const remote = path.join(root, "fixture-origin");
  await fs.mkdir(remote);
  await fs.writeFile(
    path.join(remote, "add.cjs"),
    "module.exports = (a,b) => a+b;\n",
  );
  await fs.writeFile(
    path.join(remote, "test.cjs"),
    "const assert=require('node:assert/strict');assert.equal(require('./add.cjs')(19,23),42);console.log('addition check passed');\n",
  );
  await exec("git", ["init", remote]);
  await exec("git", ["-C", remote, "add", "add.cjs", "test.cjs"]);
  await exec("git", [
    "-C",
    remote,
    "-c",
    "user.name=Test",
    "-c",
    "user.email=test@example.test",
    "commit",
    "-m",
    "fixture",
  ]);
  const cloned = await cloneCourseRepository({
    courseId: course.id,
    repository: remote,
    destination: "starter",
  });
  assert.equal((await finished(cloned)).status, "completed");
  assert.ok(
    (await listAssignmentFiles({ courseId: course.id })).entries.some(
      (e) => e.path === "starter/add.cjs",
    ),
  );
  assert.ok(
    !(await listAssignmentFiles({ courseId: course.id })).entries.some((e) =>
      e.path.includes(".git"),
    ),
  );
  await assert.rejects(
    () =>
      cloneCourseRepository({
        courseId: course.id,
        repository: remote,
        destination: "starter",
      }),
    /already exists/,
  );
  for (const title of ["Assignment one", "Assignment two"]) {
    const assignment = await createAssignment({ courseId: course.id, title });
    const run = await startCourseCommand({
      courseId: course.id,
      assignmentId: assignment.id,
      cwd: "starter",
      command: "node test.cjs",
      purpose: `Verify reusable addition for ${title}`,
    });
    const result = await finished(run);
    assert.equal(result.exitCode, 0);
    assert.match(result.output, /addition check passed/);
    assert.match(await readPrivate(result.summaryPath), /Status: completed/);
  }
  const source = (
    await searchMemory(course.id, "add.cjs module exports", 30)
  ).find((s) => s.kind === "file" && s.path.endsWith("add.cjs"))!;
  assert.ok(source);
  assert.match(
    JSON.stringify(
      await readCourseContext({ courseId: course.id, sourceId: source.id }),
    ),
    /module.exports/,
  );
  const input = {
    courseId: course.id,
    prompt: "Practice addition",
    scope: { mode: "dates", from: "2026-09-01", through: "2026-09-30" },
    contentSourceIds: [source.id],
  };
  assert.ok(
    (await previewExamScope(input)).additionalContent.some(
      (s) => s.id === source.id,
    ),
  );
  assert.ok((await queueExam(input)).context.some((s) => s.kind === "file"));
  const other = await addCourse({ code: "OTHER", name: "Other" });
  assert.ok(
    (await searchMemory(other.id, "add.cjs")).every(
      (source) => source.courseId === other.id && source.kind !== "file",
    ),
  );
  assert.equal((await listCourseCommands(course.id)).length, 3);
});
test("shared files index current edits and report omissions without indexing dependencies, symlinks or credentials", async () => {
  const course = await addCourse({ code: "CS", name: "Files" });
  const scope = { courseId: course.id };
  const first = await writeAssignmentFile(
    {
      ...scope,
      path: "src/vector.py",
      content: "def vector_norm():\n    return 'magnitude'\n",
      explanation,
    },
    true,
  );
  const evidence = (await indexCourseFiles(course.id)).evidence;
  assert.equal(evidence.length, 1);
  await writeAssignmentFile({
    ...scope,
    path: first.path,
    content: "def vector_norm():\n    return 'squared magnitude'\n",
    expectedRevision: first.revision,
    explanation,
  });
  const revised = (await indexCourseFiles(course.id)).evidence;
  assert.notEqual(revised[0].id, evidence[0].id);
  await writePrivate(
    `courses/${course.id}/files/node_modules/pkg/secret.txt`,
    "Excluded dependency keyword",
  );
  await writePrivate(`courses/${course.id}/files/.env`, "TOKEN=must-not-index");
  await writePrivate(
    `courses/${course.id}/files/image.bin`,
    Buffer.from([0, 255]),
  );
  await fs.symlink(os.tmpdir(), safePath(`courses/${course.id}/files/link`));
  const indexed = await indexCourseFiles(course.id);
  assert.equal(indexed.files, 1);
  assert.equal(indexed.skipped.length, 2);
  assert.ok(!JSON.stringify(indexed.evidence).includes("must-not-index"));
  assert.ok(
    !(await listAssignmentFiles(scope)).entries.some((e) => e.path === "link"),
  );
  await assert.rejects(
    () => readAssignmentFile({ ...scope, path: ".env" }),
    /excluded/,
  );
  await assert.rejects(() =>
    writeAssignmentFile(
      { ...scope, path: "../elsewhere", content: "x", explanation },
      true,
    ),
  );
});
test("command lifecycle preserves failures, supports assignment cwd, cancellation, timeout and paginated output", async () => {
  const course = await addCourse({ code: "CS", name: "Commands" });
  const scope = { courseId: course.id };
  const assignment = await createAssignment({ ...scope, title: "Run locally" });
  const run = await startCourseCommand({
    ...scope,
    assignmentId: assignment.id,
    location: "assignment",
    command:
      "node -e \"require('fs').writeFileSync('generated.txt','Reusable command output');console.log(process.env.COURSE_FILES_ROOT);console.log(process.env.ASSIGNMENT_ROOT);process.exit(7)\"",
    purpose: "Inspect shared roots and deliberate failure",
  });
  const failure = await finished(run);
  assert.equal(failure.status, "failed");
  assert.equal(failure.exitCode, 7);
  assert.match(failure.output, /\/files/);
  assert.match(failure.output, new RegExp(assignment.id));
  assert.equal(
    (
      await readAssignmentFile({
        ...scope,
        assignmentId: assignment.id,
        path: "generated.txt",
      })
    ).content,
    "Reusable command output",
  );
  assert.equal(
    (
      await readAssignmentFile({
        ...scope,
        path: `assignments/${assignment.id}/generated.txt`,
      })
    ).content,
    "Reusable command output",
  );
  const partial = await readCourseCommand({
    ...scope,
    runId: run.id,
    limit: 5,
  });
  assert.equal(partial.output.length, 5);
  assert.equal(partial.nextOffset, 5);
  const timeout = await startCourseCommand({
    ...scope,
    command: 'node -e "setInterval(()=>{},1000)"',
    purpose: "Timeout a long-running command",
    timeoutSeconds: 1,
  });
  assert.equal((await finished(timeout)).status, "timed_out");
  const cancel = await startCourseCommand({
    ...scope,
    command: 'node -e "setInterval(()=>{},1000)"',
    purpose: "Cancel a long-running command",
  });
  await stopCourseCommand({ ...scope, runId: cancel.id });
  assert.equal((await finished(cancel)).status, "cancelled");
  await assert.rejects(() =>
    startCourseCommand({
      ...scope,
      cwd: "../../..",
      command: "pwd",
      purpose: "Reject an invalid initial working directory",
    }),
  );
  const response = await commandPost(
    new NextRequest("http://localhost:3000/api/course-commands", {
      method: "POST",
      headers: {
        host: "localhost:3000",
        origin: "https://foreign.test",
        "content-type": "application/json",
      },
      body: JSON.stringify({
        action: "start",
        data: {
          ...scope,
          command: "pwd",
          purpose: "No CSRF command execution",
        },
      }),
    }),
  );
  assert.equal(response.status, 400);
  const list = await filesGet(
    new NextRequest(
      `http://localhost:3000/api/course-files?courseId=${course.id}`,
      { headers: { host: "localhost:3000" } },
    ),
  );
  assert.equal(list.status, 200);
});
