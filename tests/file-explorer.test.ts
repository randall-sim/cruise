import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import childProcess from "node:child_process";
import { EventEmitter } from "node:events";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { addCourse } from "../src/lib/store";
import { createAssignment } from "../src/lib/assignments";
import {
  writeAssignmentFile,
  referenceCoursePath,
} from "../src/lib/assignment-files";
import { readAssignmentSteps } from "../src/lib/assignment-timeline";
import {
  fileExplorerCommand,
  openFileExplorer,
  resolveExplorerTarget,
} from "../src/lib/file-explorer";
import { POST } from "../src/app/api/file-explorer/route";
import { NextRequest } from "../src/lib/web-request";

let root: string;
const original = process.env.COURSE_CAPTAIN_WORKSPACE;
beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "cc-explorer-"));
  process.env.COURSE_CAPTAIN_WORKSPACE = root;
});
afterEach(async () => {
  if (original) process.env.COURSE_CAPTAIN_WORKSPACE = original;
  else delete process.env.COURSE_CAPTAIN_WORKSPACE;
  assert.ok(root.startsWith(path.join(os.tmpdir(), "cc-explorer-")));
  await fs.rm(root, { recursive: true, force: true });
});
async function fixture() {
  const course = await addCourse({
    code: "EXPLORER",
    name: "Explorer fixture",
  });
  const assignment = await createAssignment({
    courseId: course.id,
    title: "File reveal",
  });
  const file = await writeAssignmentFile(
    {
      courseId: course.id,
      path: "project name/a & $'file.txt",
      content: "fixture",
      explanation: "Create an isolated file explorer fixture.",
    },
    true,
  );
  await referenceCoursePath({
    courseId: course.id,
    assignmentId: assignment.id,
    path: "alias.txt",
    targetPath: "project name/a & $'file.txt",
  });
  const step = (await readAssignmentSteps(course.id, assignment.id)).at(-1)!;
  return {
    courseId: course.id,
    assignmentId: assignment.id,
    path: "alias.txt",
    stepId: step.id,
    file,
  };
}

test("uses native file managers with literal arguments on Windows, macOS, Linux and WSL", async (t) => {
  const windows = "C:\\Course files\\a & $'file.txt";
  const posix = "/home/student/Course files/a & $'file.txt";
  assert.deepEqual(
    (
      await fileExplorerCommand(
        { path: windows, directory: false },
        "win32",
        false,
      )
    ).args,
    ["/select,", windows],
  );
  assert.deepEqual(
    await fileExplorerCommand(
      { path: posix, directory: false },
      "darwin",
      false,
    ),
    { command: "/usr/bin/open", args: ["-R", posix], wait: true },
  );
  assert.deepEqual(
    await fileExplorerCommand(
      { path: posix, directory: false },
      "linux",
      false,
    ),
    { command: "xdg-open", args: [path.dirname(posix)], wait: true },
  );
  assert.deepEqual(
    (
      await fileExplorerCommand(
        { path: "/tmp/folder", directory: true },
        "darwin",
        false,
      )
    ).args,
    ["/tmp/folder"],
  );
  t.mock.method(childProcess, "execFile", ((
    command: string,
    args: string[],
    options: unknown,
    callback: (error: null, stdout: string) => void,
  ) => {
    assert.equal(command, "wslpath");
    assert.deepEqual(args, ["-w", posix]);
    callback(
      null,
      "\\\\wsl.localhost\\Ubuntu\\home\\student\\Course files\\a & $'file.txt\n",
    );
  }) as typeof childProcess.execFile);
  const wsl = await fileExplorerCommand(
    { path: posix, directory: false },
    "linux",
    true,
  );
  assert.equal(wsl.command, "explorer.exe");
  assert.deepEqual(wsl.args, [
    "/select,",
    "\\\\wsl.localhost\\Ubuntu\\home\\student\\Course files\\a & $'file.txt",
  ]);
  await assert.rejects(
    fileExplorerCommand({ path: posix, directory: false }, "freebsd", false),
    /supported/,
  );
});

test("resolves live aliases and saved paths; missing files use the nearest existing course folder", async () => {
  const { file, ...input } = await fixture();
  const { stepId, ...live } = input;
  assert.equal((await resolveExplorerTarget(live)).path, file.absolutePath);
  assert.equal((await resolveExplorerTarget(input)).path, file.absolutePath);
  assert.equal(
    (
      await resolveExplorerTarget({
        courseId: input.courseId,
        path: "project name/a & $'file.txt",
      })
    ).path,
    file.absolutePath,
  );
  await fs.unlink(file.absolutePath);
  const missing = await resolveExplorerTarget(input);
  assert.deepEqual(missing, {
    path: path.dirname(file.absolutePath),
    directory: true,
    missing: true,
  });
  await fs.rmdir(path.dirname(file.absolutePath));
  assert.equal(
    (await resolveExplorerTarget(input)).path,
    path.dirname(path.dirname(file.absolutePath)),
  );
});

test("rejects traversal, arbitrary paths, foreign scopes and symlinks before launching", async () => {
  const { file, ...input } = await fixture();
  await assert.rejects(
    resolveExplorerTarget({ ...input, path: "../state.json" }),
  );
  await assert.rejects(
    resolveExplorerTarget({ ...input, absolutePath: "/etc/passwd" }),
  );
  await assert.rejects(
    resolveExplorerTarget({ ...input, stepId: undefined, path: ".env" }),
  );
  const other = await addCourse({ code: "OTHER", name: "Other course" });
  await assert.rejects(
    resolveExplorerTarget({ ...input, courseId: other.id }),
    /Assignment not found/,
  );
  await fs.unlink(file.absolutePath);
  await fs.symlink(path.join(root, "state.json"), file.absolutePath);
  await assert.rejects(resolveExplorerTarget(input), /Symlinks/);
  const denied = await POST(
    new NextRequest("http://localhost/api/file-explorer", {
      method: "POST",
      headers: {
        host: "localhost",
        origin: "https://example.com",
        "content-type": "application/json",
      },
      body: JSON.stringify(input),
    }),
  );
  assert.equal(denied.status, 400);
});

test("launches without a shell and surfaces launcher failures", async (t) => {
  const { file, ...input } = await fixture();
  const commands: string[] = [];
  let fail = false;
  t.mock.method(childProcess, "execFile", ((
    command: string,
    args: string[],
    options: { windowsHide: boolean; timeout: number; shell?: unknown },
    callback: (error: Error | null, stdout: string) => void,
  ) => {
    assert.equal(options.shell, undefined);
    assert.equal(options.windowsHide, true);
    assert.equal(options.timeout, 8000);
    commands.push(command);
    callback(
      fail ? new Error("No desktop available") : null,
      command === "wslpath" ? "C:\\Course files\\fixture.txt\n" : "",
    );
  }) as typeof childProcess.execFile);
  t.mock.method(childProcess, "spawn", ((
    command: string,
    args: string[],
    options: { shell?: unknown; windowsHide: boolean },
  ) => {
    assert.equal(options.shell, undefined);
    assert.equal(options.windowsHide, true);
    commands.push(command);
    const child = Object.assign(new EventEmitter(), { unref() {} });
    queueMicrotask(() =>
      child.emit(fail ? "error" : "spawn", new Error("Launch failed")),
    );
    return child;
  }) as typeof childProcess.spawn);
  assert.deepEqual(await openFileExplorer(input), { missing: false });
  const response = await POST(
    new NextRequest("http://localhost/api/file-explorer", {
      method: "POST",
      headers: {
        host: "localhost",
        origin: "http://localhost",
        "content-type": "application/json",
      },
      body: JSON.stringify(input),
    }),
  );
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { missing: false });
  assert.ok(commands.length > 0);
  fail = true;
  await assert.rejects(
    openFileExplorer(input),
    /Could not open the file explorer/,
  );
});
