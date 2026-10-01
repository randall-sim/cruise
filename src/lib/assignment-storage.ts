import { promises as fs } from "node:fs";
import path from "node:path";
import type { State } from "./schema";
import { assertNoSymlink, newId, readPrivate, safePath } from "./store";
import { readReferences, saveReferences } from "./assignment-references";
import { trackPathBeforeChange, recordPathOperation } from "./file-history";

export const assignmentStorage = (courseId: string, assignmentId: string) =>
  `courses/${courseId}/files/assignments/${assignmentId}`;

async function checkRunningCommands(courseId: string, oldRoot: string) {
  const runs = `courses/${courseId}/.runtime/commands`;
  const directory = safePath(runs);
  await assertNoSymlink(directory);
  let ids: string[];
  try {
    ids = await fs.readdir(directory);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
    throw error;
  }
  for (const id of ids) {
    const run = JSON.parse(await readPrivate(`${runs}/${id}/run.json`));
    if (
      (run.status === "running" || run.status === "queued") &&
      (run.cwd === oldRoot || run.cwd?.startsWith(oldRoot + path.sep))
    )
      throw new Error(
        `Wait for command ${id} to finish before migrating its assignment files`,
      );
  }
}

// Caller holds the workspace lock. Old roots are moved only after a recovery
// copy exists; refs/history are prepared first so retries can finish a move.
export async function ensureCourseFilesShared(state: State, courseId: string) {
  const ids = new Set([
    ...(state.assignments || [])
      .filter((a) => a.courseId === courseId)
      .map((a) => a.id),
    ...state.jobs
      .filter((j) => j.courseId === courseId && j.kind === "assignment")
      .map((j) => j.id),
  ]);
  for (const id of ids) {
    const oldRelative = `assignments/${id}`;
    const oldRoot = safePath(`courses/${courseId}/${oldRelative}`);
    await assertNoSymlink(oldRoot);
    let children;
    try {
      children = await fs.readdir(oldRoot, { withFileTypes: true });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") continue;
      throw error;
    }
    if (!children.length) continue;
    await checkRunningCommands(courseId, oldRoot);
    const destination = safePath(assignmentStorage(courseId, id));
    await assertNoSymlink(destination);
    // Do not merge unrelated bytes or overwrite an existing canonical folder.
    try {
      await fs.stat(destination);
      throw new Error(
        `Assignment migration needs conflict resolution: ${destination} already exists. Original files were preserved.`,
      );
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    const refs = await readReferences(courseId, id);
    for (const child of children) {
      if (child.isSymbolicLink() || (!child.isFile() && !child.isDirectory()))
        throw new Error(
          "Assignment migration requires regular files and folders",
        );
      const existing = refs.find(
        (r) => r.path.toLowerCase() === child.name.toLowerCase(),
      );
      const targetPath = `assignments/${id}/${child.name}`;
      if (existing && existing.targetPath !== targetPath)
        throw new Error(
          `Assignment migration reference conflict: ${child.name}`,
        );
      if (!existing)
        refs.push({
          path: child.name,
          targetPath,
          type: child.isDirectory() ? "directory" : "file",
          createdAt: new Date().toISOString(),
          excludedPaths: [],
        });
    }
    const backup = safePath(
      `.trash/assignment-storage/${newId()}/${courseId}/${id}`,
    );
    await assertNoSymlink(backup);
    await fs.mkdir(path.dirname(backup), { recursive: true });
    await fs.cp(oldRoot, backup, {
      recursive: true,
      force: false,
      errorOnExist: true,
      preserveTimestamps: true,
    });
    const tracked = await trackPathBeforeChange(courseId, oldRelative);
    await saveReferences(courseId, id, refs);
    await recordPathOperation(
      courseId,
      tracked,
      {
        actor: "agent",
        assignmentId: id,
        tool: "migrate_assignment_storage",
        explanation:
          "Preserved assignment files in course Files; assignment now selects shared originals.",
        context: `Recovery copy: ${backup}`,
      },
      oldRelative,
      `files/assignments/${id}`,
    );
    await fs.mkdir(path.dirname(destination), { recursive: true });
    await fs.rename(oldRoot, destination);
  }
  for (const id of ids) {
    const from = `assignments/${id}.md`;
    const original = safePath(`courses/${courseId}/${from}`);
    await assertNoSymlink(original);
    try {
      await fs.stat(original);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") continue;
      throw error;
    }
    const to = `files/assignments/${id}/legacy-draft.md`;
    const refs = await readReferences(courseId, id);
    const existing = refs.find(
      (ref) => ref.path.toLowerCase() === "legacy-draft.md",
    );
    if (existing && existing.targetPath !== `assignments/${id}/legacy-draft.md`)
      throw new Error(
        "Legacy draft display name is already in use; original preserved",
      );
    const destination = safePath(`courses/${courseId}/${to}`);
    await assertNoSymlink(destination);
    try {
      await fs.stat(destination);
      throw new Error(
        "Legacy draft migration destination already exists; original preserved",
      );
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    const backup = safePath(
      `.trash/assignment-storage/${newId()}/${courseId}/${id}.md`,
    );
    await assertNoSymlink(backup);
    await fs.mkdir(path.dirname(backup), { recursive: true });
    await fs.copyFile(original, backup);
    const tracked = await trackPathBeforeChange(courseId, from);
    await fs.mkdir(path.dirname(destination), { recursive: true });
    if (!existing) {
      refs.push({
        path: "legacy-draft.md",
        targetPath: `assignments/${id}/legacy-draft.md`,
        type: "file",
        createdAt: new Date().toISOString(),
        excludedPaths: [],
      });
      await saveReferences(courseId, id, refs);
    }
    await recordPathOperation(
      courseId,
      tracked,
      {
        actor: "agent",
        assignmentId: id,
        tool: "migrate_assignment_storage",
        explanation: "Preserved legacy assignment draft in course Files.",
        context: `Recovery copy: ${backup}`,
      },
      from,
      to,
    );
    await fs.rename(original, destination);
  }
}

// Register newly created roots in the assignment's display manifest. The bytes
// already live in course Files, including files generated by local commands.
export async function showCreatedAssignmentPath(
  courseId: string,
  assignmentId: string | undefined,
  relative: string,
) {
  if (!assignmentId) return;
  const refs = await readReferences(courseId, assignmentId);
  const root = relative.split("/")[0];
  if (refs.some((ref) => ref.path === root)) return;
  const targetPath = `assignments/${assignmentId}/${root}`;
  const stat = await fs.stat(
    safePath(`courses/${courseId}/files/${targetPath}`),
  );
  refs.push({
    path: root,
    targetPath,
    type: stat.isDirectory() ? "directory" : "file",
    createdAt: new Date().toISOString(),
    excludedPaths: [],
  });
  await saveReferences(courseId, assignmentId, refs);
}
