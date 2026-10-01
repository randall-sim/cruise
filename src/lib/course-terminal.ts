import { promises as fs } from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { z } from "zod";
import { assignmentPath } from "./assignment-schema";
import { idSchema } from "./schema";
import {
  assertNoSymlink,
  newId,
  readPrivate,
  readState,
  requireCourse,
  safePath,
  writePrivate,
  workspaceRoot,
} from "./store";
import { requireAssignment } from "./assignments";
import { mutate } from "./store";
import {
  assignmentStorage,
  ensureCourseFilesShared,
} from "./assignment-storage";
import { resolveAssignmentPath } from "./assignment-references";

export const commandInput = z.object({
  courseId: idSchema,
  assignmentId: idSchema.optional(),
  location: z.enum(["files", "assignment"]).default("files"),
  cwd: z.union([z.literal("."), assignmentPath]).default("."),
  command: z.string().trim().min(1).max(12000),
  purpose: z.string().trim().min(5).max(2000),
  timeoutSeconds: z.number().int().min(1).max(3600).default(300),
});
export const commandReadInput = z.object({
  courseId: idSchema,
  runId: idSchema,
  offset: z.number().int().min(0).default(0),
  limit: z.number().int().min(1).max(50000).default(12000),
});
export const commandIdentity = commandReadInput.pick({
  courseId: true,
  runId: true,
});
export const cloneInput = z.object({
  courseId: idSchema,
  repository: z
    .string()
    .min(1)
    .max(2000)
    .refine(
      (value) =>
        !/[\r\n\0]/.test(value) &&
        (/^https:\/\/[^\s/@]+\//.test(value) ||
          /^ssh:\/\/(?:git@)?[^\s/]+\//.test(value) ||
          /^git@[^\s:]+:.+/.test(value) ||
          path.isAbsolute(value)),
      "Use an HTTPS/SSH repository URL without embedded credentials, or a local repository path",
    ),
  destination: assignmentPath,
  branch: z
    .string()
    .min(1)
    .max(200)
    .regex(/^[A-Za-z0-9][A-Za-z0-9._/-]*$/)
    .optional(),
  timeoutSeconds: z.number().int().min(1).max(3600).default(600),
});
export type CommandRun = {
  id: string;
  courseId: string;
  assignmentId?: string;
  command: string;
  purpose: string;
  cwd: string;
  status:
    | "queued"
    | "running"
    | "completed"
    | "failed"
    | "cancelled"
    | "timed_out"
    | "interrupted";
  createdAt: string;
  startedAt?: string;
  finishedAt?: string;
  exitCode?: number | null;
  signal?: string | null;
  pid?: number;
  heartbeat?: string;
  truncated?: boolean;
  error?: string;
  summaryPath: string;
};
const relativeRun = (courseId: string, id: string) =>
  `courses/${courseId}/.runtime/commands/${id}`;
export async function startCourseCommand(
  input: unknown,
  invocation?: { executable: string; args: string[]; repository?: string },
) {
  const data = commandInput.parse(input);
  const state = await readState();
  requireCourse(state, data.courseId);
  if (data.assignmentId)
    requireAssignment(state, data.courseId, data.assignmentId);
  if (data.location === "assignment" && !data.assignmentId)
    throw new Error("An assignment ID is required for this working directory");
  await mutate((current) => ensureCourseFilesShared(current, data.courseId));
  const filesRoot = safePath(`courses/${data.courseId}/files`);
  const assignmentRoot = data.assignmentId
    ? safePath(assignmentStorage(data.courseId, data.assignmentId))
    : undefined;
  const root = data.location === "assignment" ? assignmentRoot! : filesRoot;
  await assertNoSymlink(root);
  await fs.mkdir(root, { recursive: true });
  const cwd =
    data.cwd === "."
      ? root
      : data.location === "assignment"
        ? (
            await resolveAssignmentPath(
              data.courseId,
              data.assignmentId,
              data.cwd,
            )
          ).absolutePath
        : path.join(root, data.cwd);
  await assertNoSymlink(cwd);
  if (!(await fs.stat(cwd)).isDirectory())
    throw new Error("Working directory is not a directory");
  const id = newId(),
    base = relativeRun(data.courseId, id);
  const summaryPath = `courses/${data.courseId}/memory/files/runs/${id}.md`;
  const run: CommandRun = {
    id,
    courseId: data.courseId,
    assignmentId: data.assignmentId,
    command: data.command,
    purpose: data.purpose,
    cwd,
    status: "queued",
    createdAt: new Date().toISOString(),
    summaryPath,
  };
  await writePrivate(`${base}/run.json`, JSON.stringify(run));
  // Only a small host environment is inherited; never copy the app's .env secrets.
  const env: NodeJS.ProcessEnv = { NODE_ENV: "development" };
  for (const key of [
    "PATH",
    "HOME",
    "USERPROFILE",
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "TMP",
    "TEMP",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "SSH_AUTH_SOCK",
  ]) {
    if (process.env[key]) env[key] = process.env[key];
  }
  env.COURSE_FILES_ROOT = filesRoot;
  env.COURSE_ROOT = safePath(`courses/${data.courseId}`);
  env.COURSE_CAPTAIN_WORKSPACE = workspaceRoot();
  if (assignmentRoot) env.ASSIGNMENT_ROOT = assignmentRoot;
  const request = {
    ...run,
    timeoutSeconds: data.timeoutSeconds,
    env,
    invocation,
    summaryFile: safePath(summaryPath),
    assignmentRoot,
    assignmentRootsBefore: assignmentRoot
      ? await fs.readdir(assignmentRoot).catch((error) => {
          if (error.code === "ENOENT") return [] as string[];
          throw error;
        })
      : [],
  };
  await writePrivate(`${base}/request.json`, JSON.stringify(request));
  const script = path.resolve(
    /*turbopackIgnore: true*/ "scripts/course-command-worker.mjs",
  );
  await assertNoSymlink(safePath(summaryPath));
  const child = spawn(
    process.execPath,
    ["--import", "tsx", script, safePath(`${base}/request.json`)],
    { detached: true, stdio: "ignore", windowsHide: true, env },
  );
  await new Promise<void>((resolve, reject) => {
    child.once("spawn", resolve);
    child.once("error", reject);
  }).catch(async (error) => {
    await writePrivate(
      `${base}/run.json`,
      JSON.stringify({ ...run, status: "failed", error: String(error) }),
    );
    throw error;
  });
  await writePrivate(`${base}/owner.json`, JSON.stringify({ pid: child.pid }));
  child.unref();
  return {
    ...run,
    instructions:
      "Poll read_course_command until terminal; inspect exit code and output. Use stop_course_command to cancel. Commands run on this host, not in a security sandbox. Never submit coursework.",
  };
}
export async function readCourseCommand(input: unknown) {
  const data = commandReadInput.parse(input);
  requireCourse(await readState(), data.courseId);
  const run = await readCommandMetadata(data.courseId, data.runId);
  const base = relativeRun(data.courseId, data.runId);
  let output = "";
  try {
    output = await readPrivate(`${base}/output.log`);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  return {
    ...run,
    output: output.slice(data.offset, data.offset + data.limit),
    offset: data.offset,
    nextOffset: Math.min(output.length, data.offset + data.limit),
    totalCharacters: output.length,
  };
}
// Caller validates the course once. History listings need no output bodies or
// repeated reads of the entire workspace state for each historical command.
async function readCommandMetadata(courseId: string, runId: string) {
  const base = relativeRun(courseId, runId);
  const run = JSON.parse(await readPrivate(`${base}/run.json`)) as CommandRun;
  if (run.courseId !== courseId || run.id !== runId)
    throw new Error("Command does not belong to this course");
  if (run.status === "queued" || run.status === "running") {
    try {
      const owner = JSON.parse(await readPrivate(`${base}/owner.json`));
      process.kill(owner.pid, 0);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ESRCH") {
        run.status = "interrupted";
        run.error =
          "The command worker is no longer running. Inspect outputs before retrying.";
      }
    }
  }
  return run;
}
export async function stopCourseCommand(input: unknown) {
  const data = commandIdentity.parse(input);
  const run = await readCourseCommand(data);
  if (run.status !== "queued" && run.status !== "running") return run;
  await writePrivate(
    `${relativeRun(data.courseId, data.runId)}/cancel`,
    "cancel requested",
  );
  return { id: run.id, status: run.status, cancellationRequested: true };
}
export async function listCourseCommands(courseId: string) {
  requireCourse(await readState(), courseId);
  const directory = safePath(`courses/${courseId}/.runtime/commands`);
  await assertNoSymlink(directory);
  let entries: string[];
  try {
    entries = await fs.readdir(directory);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }
  const runs = await Promise.all(
    entries
      .filter((id) => idSchema.safeParse(id).success)
      .map((runId) => readCommandMetadata(courseId, runId)),
  );
  return runs
    .sort((a, b) => b.createdAt.localeCompare(a.createdAt))
    .slice(0, 30);
}
export async function cloneCourseRepository(input: unknown) {
  const data = cloneInput.parse(input);
  requireCourse(await readState(), data.courseId);
  const target = safePath(`courses/${data.courseId}/files/${data.destination}`);
  await assertNoSymlink(target);
  try {
    await fs.lstat(target);
    throw new Error(
      "Destination already exists. Reuse the existing repository through course commands; cloning never overwrites files.",
    );
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  await fs.mkdir(path.dirname(target), { recursive: true });
  const args = [
    "clone",
    "--progress",
    ...(data.branch ? ["--branch", data.branch] : []),
    "--",
    data.repository,
    target,
  ];
  return startCourseCommand(
    {
      courseId: data.courseId,
      command: `git clone ${JSON.stringify(data.repository)} ${JSON.stringify(data.destination)}`,
      purpose: `Clone reusable course repository ${data.repository} into ${data.destination}`,
      timeoutSeconds: data.timeoutSeconds,
    },
    { executable: "git", args, repository: data.repository },
  );
}
