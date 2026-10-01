import { promises as fs } from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import {
  assignmentScope,
  assignmentPath,
  assignmentFileInput,
  assignmentWriteInput,
  assignmentEditInput,
  assignmentDeleteInput,
  assignmentMoveInput,
  assignmentReorderInput,
  assignmentLearningInput,
  assignmentReferenceInput,
  type AssignmentEntry,
} from "./assignment-schema";
import {
  assertNoSymlink,
  mutate,
  newId,
  readPrivate,
  readState,
  safePath,
  writePrivate,
} from "./store";
import { allEvidence } from "./memory";
import { courseAssignments, requireAssignment } from "./assignments";
import {
  assignmentStorage,
  ensureCourseFilesShared,
  showCreatedAssignmentPath,
} from "./assignment-storage";
import { requireCourse } from "./store";
import { excludedCoursePath } from "./file-policy";
import type { State } from "./schema";
import { idSchema } from "./schema";
import {
  checkpointAssignmentWorkspaces,
  saveAssignmentStep,
  validateAssignmentPart,
  readAssignmentSteps,
  reviseAssignmentTeaching,
} from "./assignment-timeline";
import {
  readReferences,
  saveReferences,
  resolveAssignmentPath,
} from "./assignment-references";
import {
  canonicalFilePath,
  recordFileChange,
  trackPathBeforeChange,
  recordPathOperation,
  type ChangeActor,
} from "./file-history";
function requireScope(state: State, courseId: string, assignmentId?: string) {
  if (assignmentId) return requireAssignment(state, courseId, assignmentId);
  return requireCourse(state, courseId);
}

const hash = (value: string | Buffer) =>
  createHash("sha256").update(value).digest("hex");
const base = (id: string, assignmentId?: string) =>
  assignmentId ? assignmentStorage(id, assignmentId) : `courses/${id}/files`;
const orderPath = (id: string, assignmentId?: string) =>
  assignmentId
    ? `courses/${id}/agent/assignments/${assignmentId}-files.json`
    : `courses/${id}/agent/course-files.json`;
const MAX_BYTES = 10_000_000;
export const assignmentMediaType = (name: string) =>
  ({
    png: "image/png",
    jpg: "image/jpeg",
    jpeg: "image/jpeg",
    webp: "image/webp",
    gif: "image/gif",
    svg: "image/svg+xml",
    html: "text/html",
    htm: "text/html",
    md: "text/markdown",
  })[name.split(".").pop()!.toLowerCase()] || "text/plain";
async function exists(target: string) {
  try {
    await fs.lstat(target);
    return true;
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code === "ENOENT") return false;
    throw e;
  }
}
async function filePath(
  courseId: string,
  assignmentId: string | undefined,
  relative: string,
) {
  return (await resolveAssignmentPath(courseId, assignmentId, relative))
    .absolutePath;
}
async function readOrder(
  courseId: string,
  assignmentId: string | undefined,
): Promise<string[]> {
  try {
    return JSON.parse(await readPrivate(orderPath(courseId, assignmentId)));
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw e;
  }
}
const fileHashes = new Map<string, { signature: string; revision: string }>();
const statSignature = (stat: Awaited<ReturnType<typeof fs.stat>>) =>
  `${stat.dev}:${stat.ino}:${stat.size}:${stat.mtimeMs}:${stat.ctimeMs}`;
const metadataRevision = (stat: Awaited<ReturnType<typeof fs.stat>>) =>
  `stat:${hash(statSignature(stat))}`;
async function fileRevision(
  full: string,
  stat: Awaited<ReturnType<typeof fs.stat>>,
) {
  const signature = statSignature(stat);
  const cached = fileHashes.get(full);
  if (cached?.signature === signature) return cached.revision;
  const revision =
    stat.size > MAX_BYTES ? hash(signature) : hash(await fs.readFile(full));
  const after = await fs.stat(full);
  if (statSignature(after) === signature) {
    fileHashes.set(full, { signature, revision });
    if (fileHashes.size > 20000)
      fileHashes.delete(fileHashes.keys().next().value!);
  }
  return revision;
}
export async function assignmentTreeUnlocked(
  courseId: string,
  assignmentId?: string,
  options: { directory?: string; shallow?: boolean; target?: string } = {},
) {
  const entries: AssignmentEntry[] = [];
  const references = await readReferences(courseId, assignmentId);
  if (
    assignmentId &&
    (options.directory || options.target) &&
    !references.some((r) => {
      const requested = options.directory || options.target!;
      return requested === r.path || requested.startsWith(r.path + "/");
    })
  )
    throw new Error("Path is not displayed in this assignment");
  const root = safePath(base(courseId, assignmentId));
  await assertNoSymlink(root);
  async function walk(relative: string): Promise<string> {
    const current = relative
      ? await filePath(courseId, assignmentId, relative)
      : root;
    const children = await fs.readdir(current, { withFileTypes: true });
    const revisions: string[] = [];
    for (const child of children.sort((a, b) => a.name.localeCompare(b.name))) {
      if (
        !assignmentId &&
        excludedCoursePath(relative ? `${relative}/${child.name}` : child.name)
      )
        continue;
      if (!assignmentId && child.isSymbolicLink()) continue;
      if (child.isSymbolicLink())
        throw new Error("Symlinks are not allowed in assignment workspaces");
      if (!child.isDirectory() && !child.isFile())
        throw new Error("Only regular files and directories are supported");
      const name = relative ? `${relative}/${child.name}` : child.name;
      if (references.some((ref) => ref.path === name))
        throw new Error(
          `Local path conflicts with shared reference: ${name}. Move the local path using your terminal before continuing.`,
        );
      if (
        options.target &&
        relative === options.target.split("/").slice(0, -1).join("/") &&
        name !== options.target
      )
        continue;
      const full = await filePath(courseId, assignmentId, name);
      const stat = await fs.stat(full);
      if (entries.length >= (assignmentId ? 2000 : 20000))
        throw new Error(
          "File workspace is too large to display; move generated output into an excluded build directory",
        );
      const entry: AssignmentEntry = {
        path: name,
        name: child.name,
        type: child.isDirectory() ? "directory" : "file",
        size: stat.size,
        modifiedAt: stat.mtime.toISOString(),
        revision: "",
      };
      entries.push(entry);
      if (assignmentId && stat.isFile() && stat.size > MAX_BYTES)
        throw new Error(`File exceeds 10 MB: ${name}`);
      entry.revision = child.isDirectory()
        ? options.shallow
          ? ""
          : await walk(name)
        : options.shallow
          ? metadataRevision(stat)
          : await fileRevision(full, stat);
      revisions.push(`${name}:${entry.revision}`);
    }
    return hash(revisions.join("\n"));
  }
  const contentsRevision =
    !assignmentId && (await exists(root))
      ? await walk(
          options.target
            ? options.target.split("/").slice(0, -1).join("/")
            : options.directory || "",
        )
      : hash("");
  if (references.length) {
    for (const ref of references) {
      if (
        options.target &&
        options.target !== ref.path &&
        !options.target.startsWith(ref.path + "/")
      )
        continue;
      if (
        options.directory &&
        options.directory !== ref.path &&
        !options.directory.startsWith(ref.path + "/")
      )
        continue;
      if (
        options.directory &&
        ref.excludedPaths.some(
          (p) =>
            options.directory === p || options.directory!.startsWith(p + "/"),
        )
      )
        throw new Error("Path is not displayed in this assignment");
      const canonical = (p: string) =>
        ref.targetPath + p.slice(ref.path.length);
      let shared;
      try {
        shared = await tree(courseId, undefined, {
          shallow: options.shallow,
          ...(options.directory
            ? { directory: canonical(options.directory) }
            : {
                target: options.target
                  ? canonical(options.target)
                  : ref.targetPath,
              }),
        });
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
        if (
          options.directory ||
          (options.target && options.target !== ref.path)
        )
          throw error;
        shared = { entries: [] as AssignmentEntry[] };
      }
      const target = shared.entries.find(
        (item) => item.path === ref.targetPath && item.type === ref.type,
      );
      if (
        !target &&
        !options.directory &&
        (!options.target || options.target === ref.path)
      ) {
        entries.push({
          path: ref.path,
          name: ref.path,
          type: ref.type,
          size: 0,
          modifiedAt: ref.createdAt,
          revision: hash(JSON.stringify(ref) + ":missing"),
          shared: {
            path: ref.targetPath,
            referencePath: ref.path,
            root: true,
            missing: true,
          },
        });
        continue;
      }
      for (const item of shared.entries.filter(
        (item) =>
          item.path === ref.targetPath ||
          (ref.type === "directory" &&
            item.path.startsWith(ref.targetPath + "/")),
      )) {
        const alias = ref.path + item.path.slice(ref.targetPath.length);
        if (
          ref.excludedPaths.some(
            (p) => alias === p || alias.startsWith(p + "/"),
          )
        )
          continue;
        entries.push({
          ...item,
          path: alias,
          name: alias.split("/").pop()!,
          absolutePath: safePath(`${base(courseId)}/${item.path}`),
          shared: {
            path: item.path,
            referencePath: ref.path,
            root: alias === ref.path,
          },
        });
      }
    }
  }
  const order = await readOrder(courseId, assignmentId);
  const ranks = new Map(order.map((p, i) => [p, i]));
  const rank = (p: string) => ranks.get(p) ?? Number.MAX_SAFE_INTEGER;
  entries.sort(
    (a, b) => rank(a.path) - rank(b.path) || a.path.localeCompare(b.path),
  );
  return {
    courseId,
    assignmentId,
    root: base(courseId, assignmentId),
    absoluteRoot: root,
    entries,
    revision: hash(
      contentsRevision +
        JSON.stringify(order) +
        JSON.stringify(references) +
        JSON.stringify(entries.map((e) => [e.path, e.revision])),
    ),
  };
}
const tree = assignmentTreeUnlocked;
// Retain only scope IDs, never mutable State objects or lecture evidence. A
// metadata check on every read invalidates this cache for changes by any process.
const directoryScopes = new Map<
  string,
  {
    signature: string;
    courses: Map<string, Set<string>>;
  }
>();
async function requireDirectoryScope(courseId: string, assignmentId?: string) {
  const statePath = safePath("state.json");
  await assertNoSymlink(statePath);
  let signature: string;
  try {
    signature = statSignature(await fs.stat(statePath));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    requireScope(await readState(), courseId, assignmentId);
    return;
  }
  let cached = directoryScopes.get(statePath);
  if (cached?.signature !== signature) {
    const state = await readState();
    cached = {
      signature,
      courses: new Map(
        state.courses.map((course) => [
          course.id,
          new Set(courseAssignments(state, course.id).map((a) => a.id)),
        ]),
      ),
    };
    if (statSignature(await fs.stat(statePath)) === signature) {
      directoryScopes.set(statePath, cached);
      if (directoryScopes.size > 4)
        directoryScopes.delete(directoryScopes.keys().next().value!);
    }
  }
  const assignments = cached.courses.get(courseId);
  if (!assignments) throw new Error("Course not found");
  if (assignmentId && !assignments.has(assignmentId))
    throw new Error("Assignment not found in this course");
}
// Dashboard folder reads avoid content reads, recursive scans and the write lock.
export async function listAssignmentDirectory(
  input: unknown,
  timings?: Record<string, number>,
) {
  const started = performance.now();
  const data = assignmentScope
    .extend({
      assignmentId: idSchema.optional(),
      directory: assignmentPath.optional(),
    })
    .parse(input);
  await requireDirectoryScope(data.courseId, data.assignmentId);
  const authorized = performance.now();
  const result = await tree(data.courseId, data.assignmentId, {
    directory: data.directory,
    shallow: true,
  });
  if (timings) {
    timings.scope = authorized - started;
    timings.listing = performance.now() - authorized;
  }
  return result;
}
export async function assignmentPathRevision(input: unknown) {
  const data = assignmentFileInput
    .extend({ assignmentId: idSchema.optional() })
    .parse(input);
  requireScope(await readState(), data.courseId, data.assignmentId);
  const result = await tree(data.courseId, data.assignmentId, {
    target: data.path,
  });
  const entry = result.entries.find((e) => e.path === data.path);
  if (!entry) throw new Error("Assignment path not found");
  return { revision: entry.revision };
}
export async function listAssignmentFiles(input: unknown) {
  const { courseId, assignmentId } = assignmentScope
    .extend({ assignmentId: idSchema.optional() })
    .parse(input);
  return mutate(async (state) => {
    requireScope(state, courseId, assignmentId);
    await ensureCourseFilesShared(state, courseId);
    return tree(courseId, assignmentId);
  });
}

// Cheap first-entry probe: no recursive hashing, file contents or migration.
export async function probeAssignmentFiles(input: unknown) {
  const { courseId, assignmentId } = assignmentScope
    .extend({ assignmentId: idSchema.optional() })
    .parse(input);
  requireScope(await readState(), courseId, assignmentId);
  const refs = await readReferences(courseId, assignmentId);
  if (refs.length) return { hasEntries: true, firstPath: refs[0].path };
  const roots = [base(courseId, assignmentId)];
  if (assignmentId)
    roots.push(`courses/${courseId}/assignments/${assignmentId}`);
  for (const relative of roots) {
    const root = safePath(relative);
    await assertNoSymlink(root);
    try {
      const directory = await fs.opendir(root);
      for await (const entry of directory) {
        if (entry.isSymbolicLink() || excludedCoursePath(entry.name)) continue;
        if (entry.isFile() || entry.isDirectory())
          return { hasEntries: true, firstPath: entry.name };
      }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
  }
  return { hasEntries: false };
}
export async function referenceCoursePath(
  input: unknown,
  actor: ChangeActor = "agent",
) {
  const data = assignmentReferenceInput.parse(input);
  return mutate(async (state) => {
    requireAssignment(state, data.courseId, data.assignmentId);
    await ensureCourseFilesShared(state, data.courseId);
    const references = await readReferences(data.courseId, data.assignmentId);
    if (references.length >= 2000)
      throw new Error("An assignment supports up to 2000 displayed roots");
    if (
      references.some(
        (ref) => ref.path.toLowerCase() === data.path.toLowerCase(),
      ) ||
      (await exists(
        safePath(`${base(data.courseId, data.assignmentId)}/${data.path}`),
      ))
    )
      throw new Error("Reference name already exists");
    const target = await filePath(data.courseId, undefined, data.targetPath);
    const stat = await fs.stat(target);
    if (!stat.isFile() && !stat.isDirectory())
      throw new Error("Only files and folders can be referenced");
    references.push({
      path: data.path,
      targetPath: data.targetPath,
      type: stat.isDirectory() ? "directory" : "file",
      createdAt: new Date().toISOString(),
      excludedPaths: [],
    });
    await checkpointAssignmentWorkspaces(state, data.courseId);
    await saveReferences(data.courseId, data.assignmentId, references);
    await checkpointAssignmentWorkspaces(state, data.courseId, {
      actor,
      assignmentId: data.assignmentId,
      tool: "reference_path",
      explanation: `Display files/${data.targetPath} as ${data.path}; reuse the shared original.`,
    });
    await appendFileMemory(
      data.courseId,
      "Referenced shared path",
      `Assignment ${data.assignmentId} references files/${data.targetPath} as ${data.path}. Edits affect the shared original.`,
      [`files/${data.targetPath}`],
    );
    return tree(data.courseId, data.assignmentId);
  });
}
export async function assignmentBytes(input: unknown) {
  const data = assignmentFileInput
    .extend({ assignmentId: idSchema.optional() })
    .parse(input);
  return mutate(async (state) => {
    requireScope(state, data.courseId, data.assignmentId);
    await ensureCourseFilesShared(state, data.courseId);
    return assignmentBytesUnlocked(data);
  });
}
async function assignmentBytesUnlocked(input: unknown) {
  const data = assignmentFileInput
    .extend({ assignmentId: idSchema.optional() })
    .parse(input);
  requireScope(await readState(), data.courseId, data.assignmentId);
  if (
    data.assignmentId &&
    !(await readReferences(data.courseId, data.assignmentId)).some(
      (ref) => data.path === ref.path || data.path.startsWith(ref.path + "/"),
    )
  )
    throw new Error("Path is not displayed in this assignment");
  const file = await filePath(data.courseId, data.assignmentId, data.path);
  const stat = await fs.stat(file);
  if (!stat.isFile() || stat.size > MAX_BYTES)
    throw new Error("Expected a regular file under 10 MB");
  return {
    ...data,
    metadataRevision: metadataRevision(stat),
    bytes: await fs.readFile(file),
    ...(await resolveAssignmentPath(
      data.courseId,
      data.assignmentId,
      data.path,
    )),
  };
}
export async function readAssignmentFile(input: unknown) {
  return describeAssignmentFile(await assignmentBytes(input));
}
async function describeAssignmentFile(
  result: Awaited<ReturnType<typeof assignmentBytesUnlocked>>,
) {
  const {
    courseId,
    assignmentId,
    path: relative,
    bytes,
    absolutePath,
    workspacePath,
    shared,
  } = result;
  let content: string | undefined;
  try {
    content = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    if (content.includes("\0")) content = undefined;
  } catch {
    /* Binary file: preview/download without decoding. */
  }
  return {
    path: relative,
    workspacePath,
    shared,
    absolutePath,
    revision: hash(bytes),
    metadataRevision: result.metadataRevision,
    size: bytes.length,
    mediaType: assignmentMediaType(relative),
    content,
  };
}
// Called under the existing workspace lock; never recursively acquire mutate().
async function appendFileMemory(
  courseId: string,
  action: string,
  explanation: string,
  paths: string[],
) {
  const file = paths.some((p) => p.startsWith("files/"))
    ? `courses/${courseId}/memory/files/activity.md`
    : `courses/${courseId}/memory/assignments/activity.md`;
  let previous =
    "# File workspace activity\n\nGenerated work history, not instructor evidence.\n";
  try {
    previous = await readPrivate(file);
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code !== "ENOENT") throw e;
  }
  await writePrivate(
    file,
    `${previous}\n## ${new Date().toISOString()} — ${action}\n\n${explanation}\n\nFiles: ${paths.map((p) => `courses/${courseId}/${p}`).join(", ")}\n`,
  );
}
export async function appendAssignmentMemory(
  courseId: string,
  action: string,
  explanation: string,
  paths: string[],
) {
  const file = `courses/${courseId}/memory/assignments/activity.md`;
  let previous =
    "# Assignment workspace activity\n\nGenerated work history, not instructor evidence. File changes do not establish correctness or submission.\n";
  try {
    previous = await readPrivate(file);
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code !== "ENOENT") throw e;
  }
  await writePrivate(
    file,
    `${previous}\n## ${new Date().toISOString()} — ${action}\n\n${explanation}\n\nFiles: ${paths.map((p) => `\`courses/${courseId}/${p}\``).join(", ") || "Workspace order"}\n`,
  );
}
async function checkRevision(
  courseId: string,
  assignmentId: string | undefined,
  relative: string,
  revision: string,
) {
  const entry = (
    await tree(courseId, assignmentId, {
      target: relative,
      shallow: revision.startsWith("stat:"),
    })
  ).entries.find((e) => e.path === relative);
  if (!entry) throw new Error("Assignment path not found");
  if (entry.revision !== revision)
    throw new Error(
      "File changed since it was read. Reload before applying this change.",
    );
  return entry;
}
async function backup(
  courseId: string,
  assignmentId: string | undefined,
  relative: string,
) {
  const source = await filePath(courseId, assignmentId, relative);
  const destination = `.trash/assignments/${newId()}/${courseId}/${relative}`;
  const target = safePath(destination);
  await assertNoSymlink(target);
  await fs.mkdir(path.dirname(target), { recursive: true });
  await fs.cp(source, target, {
    recursive: true,
    errorOnExist: true,
    force: false,
  });
  return destination;
}
export async function writeAssignmentFile(
  input: unknown,
  create = false,
  actor: ChangeActor = "agent",
) {
  const data = assignmentWriteInput
    .extend({ assignmentId: idSchema.optional() })
    .parse(input);
  const bytes =
    data.encoding === "utf8"
      ? Buffer.from(data.content)
      : Buffer.from(data.content, "base64");
  if (data.encoding === "base64" && bytes.toString("base64") !== data.content)
    throw new Error("Invalid base64 content");
  if (bytes.length > MAX_BYTES) throw new Error("Files must be under 10 MB");
  return mutate(async (state) => {
    requireScope(state, data.courseId, data.assignmentId);
    await ensureCourseFilesShared(state, data.courseId);
    const full = await filePath(data.courseId, data.assignmentId, data.path);
    const historyPath = canonicalFilePath(data.courseId, full);
    const present = await exists(full);
    if (create && present) throw new Error("Path already exists");
    if (!create && !present)
      throw new Error("Use create_assignment_file for a new file");
    let recoveryPath: string | undefined;
    if (present) {
      if (!data.expectedRevision)
        throw new Error(
          "expectedRevision from read_assignment_file is required",
        );
      const entry = await checkRevision(
        data.courseId,
        data.assignmentId,
        data.path,
        data.expectedRevision,
      );
      if (entry.type !== "file") throw new Error("Cannot write a directory");
      recoveryPath = await backup(data.courseId, data.assignmentId, data.path);
    } else if (
      (await tree(data.courseId, data.assignmentId)).entries.length >=
      (data.assignmentId ? 1980 : 19980)
    )
      throw new Error("Assignment workspace is full");
    const resolved = await resolveAssignmentPath(
      data.courseId,
      data.assignmentId,
      data.path,
    );
    const before = present ? await fs.readFile(full) : null;
    await checkpointAssignmentWorkspaces(state, data.courseId);
    await writePrivate(resolved.workspacePath, bytes);
    await recordFileChange(data.courseId, historyPath, before, bytes, {
      actor,
      explanation: data.explanation,
      context: data.context,
      assignmentId: data.assignmentId,
      tool: create ? "create_file" : "write_file",
    });
    await appendFileMemory(
      data.courseId,
      create ? "Created file" : "Updated file",
      data.explanation,
      [resolved.workspacePath.slice(`courses/${data.courseId}/`.length)],
    );
    await showCreatedAssignmentPath(
      data.courseId,
      data.assignmentId,
      data.path,
    );
    await checkpointAssignmentWorkspaces(state, data.courseId, {
      actor,
      assignmentId: data.assignmentId,
      tool: create ? "create_file" : "write_file",
      explanation: data.explanation,
      context: data.context,
    });
    return {
      ...(await describeAssignmentFile(await assignmentBytesUnlocked(data))),
      recoveryPath,
    };
  });
}
export async function editAssignmentFile(
  input: unknown,
  actor: ChangeActor = "agent",
) {
  const data = assignmentEditInput
    .extend({ assignmentId: idSchema.optional() })
    .parse(input);
  // The final write rechecks the revision under the lock.
  const file = await readAssignmentFile(data);
  if (file.content === undefined)
    throw new Error("Binary files cannot be text edited");
  let content = file.content;
  for (const edit of data.edits) {
    const index = content.indexOf(edit.oldText);
    if (index < 0 || content.indexOf(edit.oldText, index + 1) >= 0)
      throw new Error(
        "Each oldText must match exactly once; include more surrounding text",
      );
    content =
      content.slice(0, index) +
      edit.newText +
      content.slice(index + edit.oldText.length);
  }
  return writeAssignmentFile({ ...data, content }, false, actor);
}
export async function createAssignmentDirectory(
  input: unknown,
  actor: ChangeActor = "agent",
) {
  const data = assignmentFileInput
    .extend({ assignmentId: idSchema.optional() })
    .parse(input);
  return mutate(async (state) => {
    requireScope(state, data.courseId, data.assignmentId);
    await ensureCourseFilesShared(state, data.courseId);
    const full = await filePath(data.courseId, data.assignmentId, data.path);
    if (await exists(full)) throw new Error("Path already exists");
    if (
      (await tree(data.courseId, data.assignmentId)).entries.length >=
      (data.assignmentId ? 1980 : 19980)
    )
      throw new Error("Assignment workspace is full");
    await checkpointAssignmentWorkspaces(state, data.courseId);
    await fs.mkdir(full, { recursive: true });
    await showCreatedAssignmentPath(
      data.courseId,
      data.assignmentId,
      data.path,
    );
    await appendFileMemory(
      data.courseId,
      "Created folder",
      "Organized assignment files into a new folder.",
      [
        `${data.assignmentId ? `assignments/${data.assignmentId}` : "files"}/${data.path}`,
      ],
    );
    await checkpointAssignmentWorkspaces(state, data.courseId, {
      actor,
      assignmentId: data.assignmentId,
      tool: "create_folder",
      explanation: `Create folder ${data.path} to organize the workspace.`,
    });
    return tree(data.courseId, data.assignmentId);
  });
}
export async function deleteAssignmentPath(
  input: unknown,
  actor: ChangeActor = "agent",
) {
  const data = assignmentDeleteInput
    .extend({ assignmentId: idSchema.optional() })
    .parse(input);
  return mutate(async (state) => {
    requireScope(state, data.courseId, data.assignmentId);
    await ensureCourseFilesShared(state, data.courseId);
    await checkRevision(
      data.courseId,
      data.assignmentId,
      data.path,
      data.expectedRevision,
    );
    const references = await readReferences(data.courseId, data.assignmentId);
    const displayed = references.find(
      (ref) => data.path === ref.path || data.path.startsWith(ref.path + "/"),
    );
    if (data.assignmentId && displayed) {
      await checkpointAssignmentWorkspaces(state, data.courseId);
      if (displayed.path !== data.path) displayed.excludedPaths.push(data.path);
      await saveReferences(
        data.courseId,
        data.assignmentId,
        references.filter((ref) => ref.path !== data.path),
      );
      await appendFileMemory(
        data.courseId,
        "Removed shared reference",
        "Detached the assignment reference; shared files are preserved.",
        [`assignments/${data.assignmentId}/${data.path}`],
      );
      await checkpointAssignmentWorkspaces(state, data.courseId, {
        actor,
        assignmentId: data.assignmentId,
        tool: "remove_reference",
        explanation: data.explanation,
        context: data.context,
      });
      return { path: data.path, detached: true };
    }
    const historyPath = canonicalFilePath(
      data.courseId,
      await filePath(data.courseId, data.assignmentId, data.path),
    );
    const tracked = await trackPathBeforeChange(data.courseId, historyPath);
    const recoveryPath = await backup(
      data.courseId,
      data.assignmentId,
      data.path,
    );
    await checkpointAssignmentWorkspaces(state, data.courseId);
    await fs.rm(await filePath(data.courseId, data.assignmentId, data.path), {
      recursive: true,
    });
    await recordPathOperation(
      data.courseId,
      tracked,
      {
        actor,
        explanation: data.explanation,
        context: data.context,
        assignmentId: data.assignmentId,
        tool: "delete_path",
      },
      historyPath,
    );
    await appendFileMemory(
      data.courseId,
      "Deleted path (recovery copy saved)",
      data.explanation,
      [
        `${data.assignmentId ? `assignments/${data.assignmentId}` : "files"}/${data.path}`,
      ],
    );
    await checkpointAssignmentWorkspaces(state, data.courseId, {
      actor,
      assignmentId: data.assignmentId,
      tool: "delete_path",
      explanation: data.explanation,
      context: data.context,
    });
    return { path: data.path, recoveryPath };
  });
}
export async function moveAssignmentPath(
  input: unknown,
  actor: ChangeActor = "agent",
) {
  const data = assignmentMoveInput
    .extend({ assignmentId: idSchema.optional() })
    .parse(input);
  return mutate(async (state) => {
    requireScope(state, data.courseId, data.assignmentId);
    await ensureCourseFilesShared(state, data.courseId);
    await checkRevision(
      data.courseId,
      data.assignmentId,
      data.path,
      data.expectedRevision,
    );
    if (
      data.destination === data.path ||
      data.destination.startsWith(data.path + "/")
    )
      throw new Error("Cannot move a path into itself");
    const references = await readReferences(data.courseId, data.assignmentId);
    const reference = references.find((ref) => ref.path === data.path);
    if (reference && data.assignmentId && !data.destination.includes("/")) {
      if (
        references.some(
          (ref) => ref.path.toLowerCase() === data.destination.toLowerCase(),
        ) ||
        (await exists(
          safePath(
            `${base(data.courseId, data.assignmentId)}/${data.destination}`,
          ),
        ))
      )
        throw new Error("Destination already exists");
      await checkpointAssignmentWorkspaces(state, data.courseId);
      reference.path = data.destination;
      reference.excludedPaths = reference.excludedPaths.map(
        (p) => data.destination + p.slice(data.path.length),
      );
      await saveReferences(data.courseId, data.assignmentId, references);
      await checkpointAssignmentWorkspaces(state, data.courseId, {
        actor,
        assignmentId: data.assignmentId,
        tool: "rename_reference",
        explanation: data.explanation,
        context: data.context,
      });
      return tree(data.courseId, data.assignmentId);
    }
    const destination = await filePath(
      data.courseId,
      data.assignmentId,
      data.destination,
    );
    if (await exists(destination))
      throw new Error("Destination already exists");
    const source = await filePath(data.courseId, data.assignmentId, data.path);
    const historySource = canonicalFilePath(data.courseId, source),
      historyDestination = canonicalFilePath(data.courseId, destination);
    const containment = path.relative(source, destination);
    if (
      !containment ||
      (!containment.startsWith(".." + path.sep) &&
        containment !== ".." &&
        !path.isAbsolute(containment))
    )
      throw new Error(
        "Cannot move a shared path into itself through another reference",
      );
    const tracked = await trackPathBeforeChange(data.courseId, historySource);
    await checkpointAssignmentWorkspaces(state, data.courseId);
    await fs.mkdir(path.dirname(destination), { recursive: true });
    await fs.rename(
      await filePath(data.courseId, data.assignmentId, data.path),
      destination,
    );
    await recordPathOperation(
      data.courseId,
      tracked,
      {
        actor,
        explanation: data.explanation,
        context: data.context,
        assignmentId: data.assignmentId,
        tool: "move_path",
      },
      historySource,
      historyDestination,
    );
    if (reference && data.assignmentId) {
      await saveReferences(
        data.courseId,
        data.assignmentId,
        references.filter((ref) => ref !== reference),
      );
    }
    await showCreatedAssignmentPath(
      data.courseId,
      data.assignmentId,
      data.destination,
    );
    const order = await readOrder(data.courseId, data.assignmentId);
    await writePrivate(
      orderPath(data.courseId, data.assignmentId),
      JSON.stringify(
        order.map((p) =>
          p === data.path || p.startsWith(data.path + "/")
            ? data.destination + p.slice(data.path.length)
            : p,
        ),
      ),
    );
    await appendFileMemory(
      data.courseId,
      "Moved or renamed path",
      data.explanation,
      [
        `${data.assignmentId ? `assignments/${data.assignmentId}` : "files"}/${data.path}`,
        `${data.assignmentId ? `assignments/${data.assignmentId}` : "files"}/${data.destination}`,
      ],
    );
    await checkpointAssignmentWorkspaces(state, data.courseId, {
      actor,
      assignmentId: data.assignmentId,
      tool: "move_path",
      explanation: data.explanation,
      context: data.context,
    });
    return tree(data.courseId, data.assignmentId);
  });
}
export async function reorderAssignmentFiles(
  input: unknown,
  actor: ChangeActor = "agent",
) {
  const data = assignmentReorderInput
    .extend({ assignmentId: idSchema.optional() })
    .parse(input);
  return mutate(async (state) => {
    requireScope(state, data.courseId, data.assignmentId);
    await ensureCourseFilesShared(state, data.courseId);
    const current = await tree(
      data.courseId,
      data.assignmentId,
      data.directory !== undefined
        ? { directory: data.directory || undefined, shallow: true }
        : {},
    );
    if (current.revision !== data.expectedRevision)
      throw new Error("Workspace changed. Refresh before reordering.");
    if (
      new Set(data.paths).size !== data.paths.length ||
      data.paths.length !== current.entries.length ||
      data.paths.some((p) => !current.entries.some((e) => e.path === p))
    )
      throw new Error("Include every current path exactly once");
    await checkpointAssignmentWorkspaces(state, data.courseId);
    await writePrivate(
      orderPath(data.courseId, data.assignmentId),
      JSON.stringify([
        ...data.paths,
        ...(await readOrder(data.courseId, data.assignmentId)).filter(
          (p) => !data.paths.includes(p),
        ),
      ]),
    );
    await checkpointAssignmentWorkspaces(state, data.courseId, {
      actor,
      assignmentId: data.assignmentId,
      tool: "reorder_files",
      explanation: "Update the display order of workspace files.",
    });
    return tree(
      data.courseId,
      data.assignmentId,
      data.directory !== undefined
        ? { directory: data.directory || undefined, shallow: true }
        : {},
    );
  });
}
export async function recordAssignmentLearning(input: unknown) {
  const data = assignmentLearningInput.parse(input);
  if (data.part && data.phase === "completed" && !data.teachingMarkdown)
    throw new Error(
      "Complete this part with teachingMarkdown: teach the concepts, worked solution and why it works. Operational checkpoint markdown is not a student lesson.",
    );
  if (data.teachingForStepId && !data.teachingMarkdown)
    throw new Error(
      "teachingForStepId requires a student-facing teachingMarkdown explanation.",
    );
  return mutate(async (state) => {
    const assignment = requireAssignment(
      state,
      data.courseId,
      data.assignmentId,
    );
    await ensureCourseFilesShared(state, data.courseId);
    const evidence = await allEvidence(data.courseId, state);
    const sources = data.sourceIds.map((id) =>
      evidence.find((s) => s.id === id),
    );
    if (sources.some((s) => !s))
      throw new Error("Source is outside this course or no longer exists");
    if (!sources.length && !data.gaps.length)
      throw new Error("Provide course sources or an explicit evidence gap");
    for (const name of data.paths)
      if (
        !(await exists(await filePath(data.courseId, data.assignmentId, name)))
      )
        throw new Error(`Assignment path not found: ${name}`);
    const relatedPaths = await Promise.all(
      data.paths.map(
        async (p) =>
          (await resolveAssignmentPath(data.courseId, data.assignmentId, p))
            .workspacePath,
      ),
    );
    const file = `courses/${data.courseId}/memory/assignments/${newId()}.md`;
    if (data.teachingForStepId) {
      const anchor = (
        await readAssignmentSteps(data.courseId, data.assignmentId)
      ).find((s) => s.id === data.teachingForStepId);
      if (!anchor || (data.part && anchor.part?.order !== data.part.order))
        throw new Error(
          "Teaching target must belong to this assignment and part.",
        );
    } else
      await validateAssignmentPart(data.courseId, data.assignmentId, data.part);
    await writePrivate(
      file,
      `# ${assignment.title} — ${data.title}\n\nAssignment ID: ${data.assignmentId}\nPart: ${data.part ? `${data.part.order} — ${data.part.title}` : "Unspecified"}\nPhase: ${data.phase}\nNext action: ${data.nextAction || "See checkpoint"}\n\n` +
        `Generated assignment learning record — an explanation of work, not primary instructor evidence.\nRecorded: ${new Date().toISOString()}\n\n${data.markdown}\n\n${data.teachingMarkdown ? `## Teaching explanation\n\n${data.teachingMarkdown}\n\n` : ""}## Related files\n\n${relatedPaths.map((p) => `- ${p}`).join("\n")}\n\n## Course evidence\n\n${sources.map((s) => `- [${s!.id}] ${s!.title}${s!.url ? ` — ${s!.url}` : ""}\n  ${s!.text}`).join("\n")}\n\n## Uncertainty and unverified checks\n\n${data.gaps.map((g) => `- ${g}`).join("\n")}\n`,
    );
    if (data.teachingForStepId) {
      await reviseAssignmentTeaching(
        data.courseId,
        data.assignmentId,
        data.teachingForStepId,
        {
          markdown: data.teachingMarkdown!,
          timestamp: new Date().toISOString(),
          memoryPath: file,
          sourceIds: data.sourceIds,
        },
      );
      return { path: file, indexed: true, stepId: data.teachingForStepId };
    }
    const step = await saveAssignmentStep(data.courseId, data.assignmentId, {
      title: data.title,
      markdown: data.markdown,
      teachingMarkdown: data.teachingMarkdown,
      actor: "agent",
      part: data.part,
      phase: data.phase,
      nextAction: data.nextAction,
      memoryPath: file,
      sourceIds: data.sourceIds,
      gaps: data.gaps,
    });
    return { path: file, indexed: true, stepId: step.id };
  });
}
