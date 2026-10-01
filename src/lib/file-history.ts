import { promises as fs } from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { z } from "zod";
import { idSchema } from "./schema";
import { assignmentPath } from "./assignment-schema";
import { resolveAssignmentPath } from "./assignment-references";
import { requireAssignment } from "./assignments";
import { excludedCoursePath } from "./file-policy";
import {
  assertNoSymlink,
  mutate,
  newId,
  readPrivate,
  requireCourse,
  safePath,
  writePrivate,
} from "./store";
import { fileDiff } from "./file-diff";
import { checkpointAssignmentWorkspaces } from "./assignment-timeline";

export type ChangeActor = "agent" | "user" | "terminal" | "external";
export type ChangeContext = {
  actor: ChangeActor;
  explanation: string;
  context?: string;
  assignmentId?: string;
  runId?: string;
  command?: string;
  tool?: string;
};
export type Snapshot = { hash: string; size: number; retained: boolean };
export type FileRevision = ChangeContext & {
  id: string;
  sequence: number;
  timestamp: string;
  action: "created" | "updated" | "deleted" | "moved" | "baseline" | "observed";
  path: string;
  previousPath?: string;
  before: Snapshot | null;
  after: Snapshot | null;
};
type HistoryIndex = {
  paths: Record<string, string>;
  archived: Record<string, string>;
  ids?: string[];
};
const digest = (bytes: Buffer) =>
  createHash("sha256").update(bytes).digest("hex");
const historyRoot = (courseId: string) => `courses/${courseId}/history`;
const indexPath = (courseId: string) => `${historyRoot(courseId)}/index.json`;
async function readIndex(courseId: string) {
  const index = await jsonOr<HistoryIndex>(indexPath(courseId), {
    paths: {},
    archived: {},
    ids: [],
  });
  index.ids = [
    ...new Set([
      ...(index.ids || []),
      ...Object.values(index.paths),
      ...Object.values(index.archived),
    ]),
  ];
  return index;
}
async function saveIndex(courseId: string, index: HistoryIndex) {
  index.ids = [
    ...new Set([
      ...(index.ids || []),
      ...Object.values(index.paths),
      ...Object.values(index.archived),
    ]),
  ];
  await writePrivate(indexPath(courseId), JSON.stringify(index, null, 2));
}
const timelinePath = (courseId: string, id: string) =>
  `${historyRoot(courseId)}/files/${idSchema.parse(id)}.json`;
const blobPath = (courseId: string, hash: string) =>
  `${historyRoot(courseId)}/blobs/${z
    .string()
    .regex(/^[a-f0-9]{64}$/)
    .parse(hash)}`;
async function jsonOr<T>(relative: string, fallback: T): Promise<T> {
  try {
    return JSON.parse(await readPrivate(relative)) as T;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return fallback;
    throw error;
  }
}
function validPath(relative: string) {
  const parts = relative.split("/");
  if (parts[0] === "files") assignmentPath.parse(parts.slice(1).join("/"));
  else if (parts[0] === "assignments") {
    if (parts.length === 2 && parts[1].endsWith(".md"))
      idSchema.parse(parts[1].slice(0, -3));
    else {
      idSchema.parse(parts[1]);
      assignmentPath.parse(parts.slice(2).join("/"));
    }
  } else
    throw new Error(
      "History is only available for course and assignment files",
    );
  if (excludedCoursePath(relative))
    throw new Error("Excluded path cannot be stored in file history");
  return relative;
}
export function canonicalFilePath(courseId: string, absolute: string) {
  const relative = path
    .relative(safePath(`courses/${courseId}`), absolute)
    .split(path.sep)
    .join("/");
  return validPath(relative);
}
async function snapshot(courseId: string, bytes: Buffer): Promise<Snapshot> {
  const hash = digest(bytes),
    retained = bytes.length <= 10_000_000;
  if (retained) {
    const target = safePath(blobPath(courseId, hash));
    await assertNoSymlink(target);
    try {
      await fs.access(target);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      await writePrivate(blobPath(courseId, hash), bytes);
    }
  }
  return { hash, size: bytes.length, retained };
}
export async function currentSnapshot(
  courseId: string,
  relative: string,
): Promise<Snapshot | null> {
  validPath(relative);
  const full = safePath(`courses/${courseId}/${relative}`);
  await assertNoSymlink(full);
  try {
    const stat = await fs.stat(full);
    if (!stat.isFile()) throw new Error("Select a file to view its history");
    if (stat.size > 10_000_000)
      return {
        hash: createHash("sha256")
          .update(`${stat.size}:${stat.mtimeMs}:${stat.ctimeMs}`)
          .digest("hex"),
        size: stat.size,
        retained: false,
      };
    return snapshot(courseId, await fs.readFile(full));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null;
    throw error;
  }
}
const same = (a: Snapshot | null, b: Snapshot | null) =>
  a?.hash === b?.hash && a?.retained === b?.retained;
async function append(
  courseId: string,
  id: string,
  entries: FileRevision[],
  relative: string,
  before: Snapshot | null,
  after: Snapshot | null,
  context: ChangeContext,
  action: FileRevision["action"],
  previousPath?: string,
) {
  const entry: FileRevision = {
    ...context,
    id: newId(),
    sequence: entries.length + 1,
    timestamp: new Date().toISOString(),
    action,
    path: relative,
    previousPath,
    before,
    after,
  };
  entries.push(entry);
  await writePrivate(
    timelinePath(courseId, id),
    JSON.stringify(entries, null, 2),
  );
  return entry;
}
// All mutating functions below are called with the workspace lock held.
async function observe(
  courseId: string,
  relative: string,
  value: Snapshot | null,
  context: ChangeContext,
  sharedIndex?: HistoryIndex,
) {
  const index = sharedIndex || (await readIndex(courseId));
  let id = index.paths[relative] || index.archived[relative];
  if (!id && !value) return null;
  if (!id) id = newId();
  let entries = await jsonOr<FileRevision[]>(timelinePath(courseId, id), []);
  if (
    !index.paths[relative] &&
    entries.length &&
    entries.at(-1)!.path !== relative
  ) {
    if (!value) return { id, entries, index };
    id = newId();
    entries = [];
  }
  if (!entries.length || !same(entries.at(-1)!.after, value)) {
    await append(
      courseId,
      id,
      entries,
      relative,
      entries.at(-1)?.after || null,
      value,
      context,
      !entries.length ? "baseline" : value ? "observed" : "deleted",
    );
  } else if (
    value ? index.paths[relative] === id : index.archived[relative] === id
  )
    return { id, entries, index };
  if (value) {
    index.paths[relative] = id;
    delete index.archived[relative];
  } else {
    delete index.paths[relative];
    index.archived[relative] = id;
  }
  await saveIndex(courseId, index);
  return { id, entries, index };
}
export async function recordFileChange(
  courseId: string,
  relative: string,
  beforeBytes: Buffer | null,
  afterBytes: Buffer | null,
  context: ChangeContext,
) {
  validPath(relative);
  const before =
    beforeBytes === null ? null : await snapshot(courseId, beforeBytes);
  const after =
    afterBytes === null ? null : await snapshot(courseId, afterBytes);
  const prior = await observe(courseId, relative, before, {
    actor: "external",
    explanation:
      "State observed before a tracked save. Earlier changes and their context are unknown.",
  });
  const index = prior?.index || (await readIndex(courseId));
  const reuse = prior?.entries.at(-1)?.path === relative ? prior : null;
  const id = reuse?.id || newId(),
    entries = reuse?.entries || [];
  await append(
    courseId,
    id,
    entries,
    relative,
    before,
    after,
    context,
    before === null ? "created" : after === null ? "deleted" : "updated",
  );
  if (after) {
    index.paths[relative] = id;
    delete index.archived[relative];
  } else {
    delete index.paths[relative];
    index.archived[relative] = id;
  }
  await saveIndex(courseId, index);
}
export async function trackPathBeforeChange(
  courseId: string,
  relative: string,
) {
  const paths = await scanPaths(courseId, [relative]);
  for (const file of paths.files)
    await observe(courseId, file, await currentSnapshot(courseId, file), {
      actor: "external",
      explanation:
        "Existing file observed before a tracked operation; earlier history is unavailable.",
    });
  return paths.files;
}
export async function recordPathOperation(
  courseId: string,
  paths: string[],
  context: ChangeContext,
  from: string,
  to?: string,
) {
  const index = await readIndex(courseId);
  for (const oldPath of paths) {
    const id = index.paths[oldPath];
    if (!id) continue;
    const entries = await jsonOr<FileRevision[]>(
        timelinePath(courseId, id),
        [],
      ),
      before = entries.at(-1)?.after || null;
    const newPath = to ? to + oldPath.slice(from.length) : oldPath;
    await append(
      courseId,
      id,
      entries,
      newPath,
      before,
      to ? before : null,
      context,
      to ? "moved" : "deleted",
      to ? oldPath : undefined,
    );
    delete index.paths[oldPath];
    index.archived[oldPath] = id;
    if (to) {
      index.paths[newPath] = id;
      delete index.archived[newPath];
    }
  }
  await saveIndex(courseId, index);
}
async function scanPaths(courseId: string, roots = ["files", "assignments"]) {
  const files: string[] = [],
    skipped: string[] = [];
  let visited = 0;
  async function walk(relative: string, depth: number) {
    if (excludedCoursePath(relative)) return;
    if (++visited > 20000 || depth > 24) {
      skipped.push(relative);
      return;
    }
    const full = safePath(`courses/${courseId}/${relative}`);
    await assertNoSymlink(path.dirname(full));
    let stat;
    try {
      stat = await fs.lstat(full);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
      throw error;
    }
    if (stat.isSymbolicLink()) {
      skipped.push(relative);
      return;
    }
    if (stat.isDirectory()) {
      for (const name of (await fs.readdir(full)).sort())
        await walk(`${relative}/${name}`, depth + 1);
    } else if (stat.isFile()) {
      try {
        validPath(relative);
        files.push(relative);
      } catch {
        skipped.push(relative);
      }
    }
  }
  for (const root of roots) await walk(root, 0);
  return { files, skipped };
}
export async function checkpointCourseFiles(
  courseId: string,
  context: ChangeContext,
) {
  return mutate(async (state) => {
    requireCourse(state, courseId);
    const scan = await scanPaths(courseId);
    const index = await readIndex(courseId);
    const paths = new Set([
      ...scan.files,
      ...Object.keys(index.paths).filter(
        (p) =>
          !scan.skipped.some((skip) => p === skip || p.startsWith(skip + "/")),
      ),
    ]);
    let changed = 0;
    for (const relative of paths) {
      const entries = index.paths[relative]
        ? await jsonOr<FileRevision[]>(
            timelinePath(courseId, index.paths[relative]),
            [],
          )
        : [];
      const value = await currentSnapshot(courseId, relative);
      if (!same(entries.at(-1)?.after || null, value)) changed++;
      await observe(courseId, relative, value, context, index);
    }
    await checkpointAssignmentWorkspaces(state, courseId, context);
    return {
      changed,
      skipped: scan.skipped,
      coverage:
        "Observed snapshots only. Intermediate writes between checkpoints are not captured. Excluded dependencies, credentials and symlinks are omitted; files over 10 MB retain metadata only.",
    };
  });
}
export const fileHistoryInput = z.object({
  courseId: idSchema,
  assignmentId: idSchema.optional(),
  path: assignmentPath.optional(),
  historyId: idSchema.optional(),
  revisionId: idSchema.optional(),
  offset: z.number().int().min(0).default(0),
  limit: z.number().int().min(1).max(200).default(50),
  diffOffset: z.number().int().min(0).default(0),
});
export async function getFileHistory(input: unknown) {
  const data = fileHistoryInput.parse(input);
  return mutate(async (state) => {
    requireCourse(state, data.courseId);
    if (data.assignmentId)
      requireAssignment(state, data.courseId, data.assignmentId);
    let relative: string | undefined;
    if (data.path) {
      const resolved = await resolveAssignmentPath(
        data.courseId,
        data.assignmentId,
        data.path,
        true,
      );
      relative = canonicalFilePath(data.courseId, resolved.absolutePath);
      await observe(
        data.courseId,
        relative,
        await currentSnapshot(data.courseId, relative),
        {
          actor: "external",
          explanation:
            "Current file observed when history was opened. Any changes outside tracked tools have unknown author and context.",
        },
      );
    }
    const index = await readIndex(data.courseId);
    const known = new Set([
      ...(index.ids || []),
      ...Object.values(index.paths),
      ...Object.values(index.archived),
    ]);
    const id =
      data.historyId ||
      (relative
        ? index.paths[relative] || index.archived[relative]
        : undefined);
    if (id && !known.has(id))
      throw new Error("History is outside this course or does not exist");
    if (!id) {
      if (relative)
        return {
          path: relative,
          historyId: null,
          entries: [],
          total: 0,
          nextOffset: null,
        };
      const files = await Promise.all(
        [...known].map(async (historyId) => {
          const entries = await jsonOr<FileRevision[]>(
            timelinePath(data.courseId, historyId),
            [],
          );
          const latest = entries.at(-1)!;
          return {
            historyId,
            path: latest.path,
            timestamp: latest.timestamp,
            revisions: entries.length,
            deleted: !latest.after,
          };
        }),
      );
      files.sort((a, b) => b.timestamp.localeCompare(a.timestamp));
      return {
        files: files.slice(data.offset, data.offset + data.limit),
        total: files.length,
        nextOffset:
          data.offset + data.limit < files.length
            ? data.offset + data.limit
            : null,
      };
    }
    let entries = await jsonOr<FileRevision[]>(
      timelinePath(data.courseId, id),
      [],
    );
    const currentPath = entries.at(-1)?.path;
    if (!data.revisionId && currentPath && index.paths[currentPath] === id) {
      await observe(
        data.courseId,
        currentPath,
        await currentSnapshot(data.courseId, currentPath),
        {
          actor: "external",
          explanation:
            "Current file observed when history was opened. Changes outside tracked tools have unknown author and context.",
        },
      );
      entries = await jsonOr<FileRevision[]>(
        timelinePath(data.courseId, id),
        [],
      );
    }
    if (data.revisionId) {
      const revision = entries.find((e) => e.id === data.revisionId);
      if (!revision) throw new Error("Revision does not belong to this file");
      const before = await loadSnapshot(data.courseId, revision.before),
        after = await loadSnapshot(data.courseId, revision.after);
      const diff =
        (revision.before && !revision.before.retained) ||
        (revision.after && !revision.after.retained)
          ? {
              lines: [],
              message:
                "File exceeds 10 MB. Only metadata was retained for this version.",
              coarse: true,
            }
          : fileDiff(before, after);
      return {
        historyId: id,
        revision,
        diff: {
          ...diff,
          lines: diff.lines.slice(data.diffOffset, data.diffOffset + 200),
          total: diff.lines.length,
          offset: data.diffOffset,
          nextOffset:
            data.diffOffset + 200 < diff.lines.length
              ? data.diffOffset + 200
              : null,
        },
      };
    }
    return {
      historyId: id,
      path: entries.at(-1)?.path,
      entries: [...entries]
        .reverse()
        .slice(data.offset, data.offset + data.limit),
      total: entries.length,
      nextOffset:
        data.offset + data.limit < entries.length
          ? data.offset + data.limit
          : null,
    };
  });
}
export async function loadSnapshot(courseId: string, value: Snapshot | null) {
  if (!value || !value.retained) return null;
  const target = safePath(blobPath(courseId, value.hash));
  await assertNoSymlink(target);
  const bytes = await fs.readFile(target);
  if (digest(bytes) !== value.hash)
    throw new Error("History snapshot failed its integrity check");
  return bytes;
}
export async function downloadFileRevision(
  input: unknown,
  side: "before" | "after",
) {
  const data = fileHistoryInput.parse(input);
  if (!data.historyId || !data.revisionId)
    throw new Error("History and revision IDs are required");
  const result = await getFileHistory(data);
  if (!("revision" in result) || !result.revision)
    throw new Error("Revision not found");
  const bytes = await loadSnapshot(data.courseId, result.revision[side]);
  if (!bytes) throw new Error("No retained snapshot for this side");
  return { bytes, name: result.revision.path.split("/").pop()! };
}
