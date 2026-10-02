import { z } from "zod";
import {
  assignmentScope,
  assignmentPath,
  assignmentLearningInput,
  type AssignmentTree,
  type AssignmentFile,
} from "./assignment-schema";
import { idSchema, type State } from "./schema";
import { courseAssignments, requireAssignment } from "./assignments";
import { readPrivate, readState, writePrivate, newId, safePath } from "./store";
import {
  assignmentTreeUnlocked,
  assignmentMediaType,
} from "./assignment-files";
import { readReferences } from "./assignment-references";
import {
  currentSnapshot,
  loadSnapshot,
  type Snapshot,
  type ChangeContext,
} from "./file-history";
import { fileDiff } from "./file-diff";

export type TimelineFile = {
  path: string;
  type: "file" | "directory";
  originalPath: string;
  snapshot: Snapshot | null;
  missing: boolean;
};
export type AssignmentStep = {
  id: string;
  timestamp: string;
  title: string;
  markdown: string;
  teachingMarkdown?: string;
  teachingRevisions?: {
    markdown: string;
    timestamp: string;
    memoryPath: string;
    sourceIds: string[];
  }[];
  actor: ChangeContext["actor"];
  part?: { order: number; title: string };
  phase?: z.infer<typeof assignmentLearningInput>["phase"];
  nextAction?: string;
  memoryPath?: string;
  sourceIds?: string[];
  gaps?: string[];
  runId?: string;
  files: TimelineFile[];
  changes: { path: string; action: "added" | "changed" | "removed" }[];
};
type StepNote = Omit<AssignmentStep, "id" | "timestamp" | "files" | "changes">;
export type AssignmentPart = {
  order: number;
  title: string;
  stepId: string;
  teachingMarkdown: string;
  sourceIds: string[];
  timestamp: string;
};

// Tool activity stays in the audit history; the student reader has one stop per real part.
export function assignmentParts(steps: AssignmentStep[]): AssignmentPart[] {
  const numbered = steps.some((step) => step.part);
  const groups = new Map<number, AssignmentStep[]>();
  for (const step of steps) {
    if (numbered && !step.part) continue;
    const order = step.part?.order || 1;
    const group = groups.get(order) || [];
    group.push(step);
    groups.set(order, group);
  }
  return [...groups]
    .sort(([a], [b]) => a - b)
    .map(([order, group]) => {
      const last = group.at(-1)!;
      const teaching = group
        .flatMap((step) => [
          ...(step.teachingMarkdown
            ? [
                {
                  markdown: step.teachingMarkdown,
                  timestamp: step.timestamp,
                  sourceIds: step.sourceIds || [],
                },
              ]
            : []),
          ...(step.teachingRevisions || []),
        ])
        .sort((a, b) => a.timestamp.localeCompare(b.timestamp))
        .at(-1);
      return {
        order,
        title: last.part?.title || "Assignment solution",
        stepId: last.id,
        teachingMarkdown: teaching?.markdown || "",
        sourceIds: teaching?.sourceIds || [],
        timestamp: last.timestamp,
      };
    });
}

export async function getAssignmentParts(input: unknown) {
  const data = assignmentScope.parse(input);
  requireAssignment(await readState(), data.courseId, data.assignmentId);
  return {
    parts: assignmentParts(
      await readAssignmentSteps(data.courseId, data.assignmentId),
    ),
  };
}

// Called under the workspace lock. Keep original bytes, dates and progress unchanged.
export async function reviseAssignmentTeaching(
  courseId: string,
  assignmentId: string,
  stepId: string,
  revision: NonNullable<AssignmentStep["teachingRevisions"]>[number],
) {
  const steps = await readAssignmentSteps(courseId, assignmentId);
  const step = steps.find((s) => s.id === stepId);
  if (!step) throw new Error("Step not found in this assignment");
  (step.teachingRevisions ||= []).push(revision);
  await writePrivate(
    timelinePath(courseId, assignmentId),
    JSON.stringify(steps, null, 2),
  );
}
const timelinePath = (courseId: string, assignmentId: string) =>
  `courses/${courseId}/agent/assignments/${assignmentId}-timeline.json`;
export async function readAssignmentSteps(
  courseId: string,
  assignmentId: string,
): Promise<AssignmentStep[]> {
  try {
    return JSON.parse(await readPrivate(timelinePath(courseId, assignmentId)));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }
}
const signature = (file: TimelineFile) => JSON.stringify(file);

// Caller holds the workspace lock. Manifests reuse the existing deduplicated history blobs.
export async function saveAssignmentStep(
  courseId: string,
  assignmentId: string,
  note: StepNote,
  onlyIfChanged = false,
) {
  const steps = await readAssignmentSteps(courseId, assignmentId);
  const previous = steps.at(-1);
  const tree = await assignmentTreeUnlocked(courseId, assignmentId);
  const files: TimelineFile[] = [];
  for (const entry of tree.entries) {
    const originalPath = `files/${entry.shared!.path}`;
    files.push({
      path: entry.path,
      type: entry.type,
      originalPath,
      missing: !!entry.shared?.missing,
      snapshot:
        entry.type === "file" && !entry.shared?.missing
          ? await currentSnapshot(courseId, originalPath)
          : null,
    });
  }
  if (
    onlyIfChanged &&
    previous &&
    JSON.stringify(previous.files) === JSON.stringify(files)
  )
    return previous;
  const before = new Map(previous?.files.map((f) => [f.path, f]) || []);
  const after = new Map(files.map((f) => [f.path, f]));
  const changes: AssignmentStep["changes"] = [];
  for (const file of files) {
    const old = before.get(file.path);
    if (!old || signature(old) !== signature(file))
      changes.push({ path: file.path, action: old ? "changed" : "added" });
  }
  for (const file of previous?.files || [])
    if (!after.has(file.path))
      changes.push({ path: file.path, action: "removed" });
  const step: AssignmentStep = {
    ...note,
    part: note.part || previous?.part,
    id: newId(),
    timestamp: new Date().toISOString(),
    files,
    changes,
  };
  steps.push(step);
  await writePrivate(
    timelinePath(courseId, assignmentId),
    JSON.stringify(steps, null, 2),
  );
  return step;
}

// Capture shared edits for every referencing assignment, including view-only changes.
export async function checkpointAssignmentWorkspaces(
  state: State,
  courseId: string,
  context?: ChangeContext,
) {
  for (const assignment of courseAssignments(state, courseId)) {
    const previous = await readAssignmentSteps(courseId, assignment.id);
    const references = await readReferences(courseId, assignment.id);
    if (
      !references.length &&
      !previous.length &&
      context?.assignmentId !== assignment.id
    )
      continue;
    await saveAssignmentStep(
      courseId,
      assignment.id,
      {
        title: context
          ? (context.tool || "Command checkpoint").replaceAll("_", " ")
          : "Observed workspace",
        markdown: context
          ? context.explanation +
            (context.context ? `\n\n${context.context}` : "") +
            (context.command ? `\n\nCommand: ${context.command}` : "")
          : "Workspace observed before a tracked change. Earlier decisions and intermediate outside edits are unknown.",
        actor: context?.actor || "external",
        runId: context?.runId,
      },
      !context || context.assignmentId !== assignment.id,
    );
  }
}

export async function validateAssignmentPart(
  courseId: string,
  assignmentId: string,
  part?: AssignmentStep["part"],
) {
  if (!part) return;
  const latest = (await readAssignmentSteps(courseId, assignmentId)).findLast(
    (s) => s.phase && s.part,
  );
  if (
    latest?.part &&
    part.order > latest.part.order &&
    latest.phase !== "completed"
  )
    throw new Error(
      "Complete the current part with a learning checkpoint before starting the next part.",
    );
}
export const assignmentTimelineInput = assignmentScope.extend({
  stepId: idSchema.optional(),
  path: assignmentPath.optional(),
  offset: z.number().int().min(0).default(0),
  limit: z.number().int().min(1).max(200).default(100),
  diffOffset: z.number().int().min(0).default(0),
});
export type AssignmentPartChanges = {
  filesChanged: number;
  linesAdded: number;
  linesDeleted: number;
  omittedFiles: number;
  approximate: boolean;
};

function partBaseline(steps: AssignmentStep[], index: number) {
  const step = steps[index];
  const parts = assignmentParts(steps.slice(0, index + 1));
  const partIndex = parts.findIndex(
    (part) => part.order === (step.part?.order || 1),
  );
  return (
    steps.find((s) => s.id === parts[partIndex - 1]?.stepId) ||
    (step.part && steps.find((s) => s.part?.order === step.part!.order)) ||
    steps[0]
  );
}

export async function getAssignmentPartChanges(
  input: unknown,
): Promise<AssignmentPartChanges> {
  const data = assignmentTimelineInput
    .extend({ stepId: idSchema })
    .parse(input);
  requireAssignment(await readState(), data.courseId, data.assignmentId);
  const steps = await readAssignmentSteps(data.courseId, data.assignmentId);
  const index = steps.findIndex((step) => step.id === data.stepId);
  if (index < 0) throw new Error("Step not found in this assignment");
  const step = steps[index];
  const previous = partBaseline(steps, index);
  // Compare saved part endpoints, not individual tool calls. Existing files in
  // the first observed state are a baseline, never fabricated additions.
  const files = (entries: TimelineFile[]) =>
    new Map(
      entries
        .filter((file) => file.type === "file")
        .map((file) => [file.originalPath, file]),
    );
  const before = files(previous.files),
    after = files(step.files);
  const result: AssignmentPartChanges = {
    filesChanged: 0,
    linesAdded: 0,
    linesDeleted: 0,
    omittedFiles: 0,
    approximate: false,
  };
  for (const name of new Set([...before.keys(), ...after.keys()])) {
    const old = before.get(name),
      next = after.get(name);
    if (
      old &&
      next &&
      old.snapshot?.hash === next.snapshot?.hash &&
      old.missing === next.missing
    )
      continue;
    result.filesChanged++;
    if (
      [old, next].some(
        (file) => file && (file.missing || !file.snapshot?.retained),
      )
    ) {
      result.omittedFiles++;
      continue;
    }
    const [oldBytes, newBytes] = await Promise.all([
      loadSnapshot(data.courseId, old?.snapshot || null),
      loadSnapshot(data.courseId, next?.snapshot || null),
    ]);
    const diff = fileDiff(oldBytes, newBytes);
    result.linesAdded += diff.lines.filter(
      (line) => line.kind === "add",
    ).length;
    result.linesDeleted += diff.lines.filter(
      (line) => line.kind === "remove",
    ).length;
    result.approximate ||= diff.coarse;
    if (diff.coarse && !diff.lines.length) result.omittedFiles++;
  }
  return result;
}

export async function getAssignmentTimeline(input: unknown) {
  const data = assignmentTimelineInput.parse(input);
  requireAssignment(await readState(), data.courseId, data.assignmentId);
  const steps = await readAssignmentSteps(data.courseId, data.assignmentId);
  if (!data.stepId)
    return {
      steps: steps
        .slice(data.offset, data.offset + data.limit)
        .map(({ files, markdown, ...step }) => ({
          ...step,
          fileCount: files.length,
        })),
      total: steps.length,
      nextOffset:
        data.offset + data.limit < steps.length
          ? data.offset + data.limit
          : null,
    };
  const index = steps.findIndex((s) => s.id === data.stepId);
  if (index < 0) throw new Error("Step not found in this assignment");
  const step = steps[index];
  if (!data.path) return { step };
  const file = step.files.find((f) => f.path === data.path);
  const before = steps[index - 1]?.files.find((f) => f.path === data.path);
  if (!file && !before)
    throw new Error("File not found in this workspace state");
  const bytes = await loadSnapshot(data.courseId, file?.snapshot || null);
  const old = await loadSnapshot(data.courseId, before?.snapshot || null);
  let content: string | undefined;
  let truncated = false;
  if (bytes) {
    try {
      const decoded = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      if (!decoded.includes("\0")) {
        content = decoded.slice(0, 200000);
        truncated = decoded.length > 200000;
      }
    } catch {
      /* Binary versions use the snapshot download. */
    }
  }
  const diff = [file, before].some((f) => f?.snapshot && !f.snapshot.retained)
    ? {
        lines: [],
        message: "Only metadata was retained for files over 10 MB.",
        coarse: true,
      }
    : fileDiff(old, bytes);
  return {
    file: file || null,
    content,
    truncated,
    diff: {
      ...diff,
      lines: diff.lines.slice(data.diffOffset, data.diffOffset + 200),
      nextOffset:
        data.diffOffset + 200 < diff.lines.length
          ? data.diffOffset + 200
          : null,
    },
  };
}
export async function assignmentStepBytes(input: unknown) {
  const data = assignmentTimelineInput.parse(input);
  requireAssignment(await readState(), data.courseId, data.assignmentId);
  const steps = await readAssignmentSteps(data.courseId, data.assignmentId);
  const index = steps.findIndex((s) => s.id === data.stepId);
  if (index < 0) throw new Error("Step not found in this assignment");
  const file =
    steps[index].files.find((f) => f.path === data.path) ||
    partBaseline(steps, index).files.find((f) => f.path === data.path);
  if (!file?.snapshot)
    throw new Error("File not found in this workspace state");
  const bytes = await loadSnapshot(data.courseId, file.snapshot);
  if (!bytes) throw new Error("Snapshot bytes were not retained");
  return { bytes, path: file.path };
}

export async function readAssignmentSnapshot(input: unknown) {
  const data = assignmentTimelineInput
    .extend({
      stepId: idSchema,
      mode: z.enum(["directory", "file"]),
      directory: assignmentPath.optional(),
    })
    .parse(input);
  requireAssignment(await readState(), data.courseId, data.assignmentId);
  const steps = await readAssignmentSteps(data.courseId, data.assignmentId);
  const index = steps.findIndex((s) => s.id === data.stepId);
  if (index < 0) throw new Error("Step not found in this assignment");
  const step = steps[index];
  const before = new Map(
    partBaseline(steps, index).files.map((f) => [f.path, f]),
  );
  const after = new Map(step.files.map((f) => [f.path, f]));
  function change(
    file: TimelineFile,
  ): "added" | "changed" | "removed" | undefined {
    const old = before.get(file.path);
    if (!after.has(file.path) || (file.missing && old && !old.missing))
      return "removed";
    if (file.type !== "file" || file.missing) return undefined;
    if (!old || old.missing) return "added";
    if (
      old.snapshot?.hash !== file.snapshot?.hash ||
      old.originalPath !== file.originalPath
    )
      return "changed";
  }
  const root = `courses/${data.courseId}/files`;
  function original(file: TimelineFile) {
    if (!file.originalPath.startsWith("files/"))
      throw new Error("Invalid saved file path");
    assignmentPath.parse(file.originalPath.slice(6));
    return `courses/${data.courseId}/${file.originalPath}`;
  }
  function shared(file: TimelineFile) {
    return {
      path: file.originalPath.slice(6),
      referencePath: file.path.split("/")[0],
      root: !file.path.includes("/"),
      missing: file.missing,
    };
  }
  if (data.mode === "directory") {
    // Keep removed folders navigable so their deleted children remain visible.
    const visible = [
      ...step.files,
      ...[...before.values()].filter((f) => !after.has(f.path)),
    ];
    if (
      data.directory &&
      !visible.some((f) => f.path === data.directory && f.type === "directory")
    )
      throw new Error("Folder not found in this workspace state");
    const result: AssignmentTree = {
      expandedDirectories: data.directory
        ? undefined
        : [
            ...new Set(
              visible
                .filter((file) => file.type === "file" && change(file))
                .flatMap((file) => {
                  const parents = file.path.split("/").slice(0, -1);
                  return parents.map((_, i) =>
                    parents.slice(0, i + 1).join("/"),
                  );
                }),
            ),
          ],
      courseId: data.courseId,
      assignmentId: data.assignmentId,
      root,
      absoluteRoot: safePath(root),
      revision: step.id,
      entries: visible
        .filter(
          (f) =>
            f.path.split("/").slice(0, -1).join("/") === (data.directory || ""),
        )
        .map((file) => ({
          path: file.path,
          name: file.path.split("/").pop()!,
          type: file.type,
          size: file.snapshot?.size || 0,
          revision: file.snapshot?.hash || step.id,
          modifiedAt: step.timestamp,
          absolutePath: safePath(original(file)),
          shared: shared(file),
          change: change(file),
        })),
    };
    return result;
  }
  const current = after.get(data.path!);
  const previous = before.get(data.path!);
  const file = current || previous;
  if (!file || file.type !== "file")
    throw new Error("File not found in this workspace state");
  const bytes = await loadSnapshot(data.courseId, file.snapshot);
  let diff: AssignmentFile["diff"];
  if (change(file)) {
    const unavailable = [previous, current].some(
      (version) => version && !version.missing && !version.snapshot?.retained,
    );
    const comparison = unavailable
      ? {
          lines: [],
          message:
            "Saved contents are unavailable for this comparison; only metadata was retained.",
          coarse: true,
        }
      : fileDiff(
          previous && !previous.missing
            ? await loadSnapshot(data.courseId, previous.snapshot)
            : null,
          current && !current.missing ? bytes : null,
        );
    diff = {
      ...comparison,
      lines: comparison.lines.slice(data.diffOffset, data.diffOffset + 200),
      nextOffset:
        data.diffOffset + 200 < comparison.lines.length
          ? data.diffOffset + 200
          : null,
      message: comparison.message.replace(
        /Download the before and after versions to compare\./,
        "An inline text diff is unavailable.",
      ),
    };
  }
  let content: string | undefined;
  if (bytes) {
    try {
      const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      if (!text.includes("\0")) content = text;
    } catch {
      /* Binary snapshots use the existing image/download view. */
    }
  }
  const result: AssignmentFile = {
    diff,
    path: file.path,
    workspacePath: original(file),
    absolutePath: safePath(original(file)),
    revision: file.snapshot?.hash || step.id,
    size: file.snapshot?.size || 0,
    mediaType: assignmentMediaType(file.path),
    content,
    shared: shared(file),
    snapshotNotice: file.missing
      ? "The original was missing when this part was recorded."
      : !file.snapshot?.retained
        ? "Only metadata was retained for this file; saved bytes are unavailable."
        : undefined,
  };
  return result;
}
