import { z } from "zod";
import { promises as fs } from "node:fs";
import { assignmentPath } from "./assignment-schema";
import { assertNoSymlink, readPrivate, safePath, writePrivate } from "./store";
import { excludedCoursePath } from "./file-policy";

const referenceSchema = z.object({
  path: assignmentPath.refine((value) => !value.includes("/")),
  targetPath: assignmentPath,
  type: z.enum(["file", "directory"]),
  createdAt: z.string(),
  excludedPaths: z.array(assignmentPath).default([]),
});
export type AssignmentReference = z.infer<typeof referenceSchema>;
const manifest = (courseId: string, assignmentId: string) =>
  `courses/${courseId}/agent/assignments/${assignmentId}-references.json`;
export async function readReferences(
  courseId: string,
  assignmentId?: string,
): Promise<AssignmentReference[]> {
  if (!assignmentId) return [];
  try {
    return z
      .array(referenceSchema)
      .max(2000)
      .parse(JSON.parse(await readPrivate(manifest(courseId, assignmentId))));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }
}
export async function saveReferences(
  courseId: string,
  assignmentId: string,
  references: Array<
    Omit<AssignmentReference, "excludedPaths"> & { excludedPaths?: string[] }
  >,
) {
  await writePrivate(
    manifest(courseId, assignmentId),
    JSON.stringify(references, null, 2),
  );
}
export async function resolveAssignmentPath(
  courseId: string,
  assignmentId: string | undefined,
  relative: string,
  allowMissing = false,
) {
  assignmentPath.parse(relative);
  const reference = (await readReferences(courseId, assignmentId)).find(
    (ref) => relative === ref.path || relative.startsWith(ref.path + "/"),
  );
  if (reference?.type === "file" && relative !== reference.path)
    throw new Error("Cannot create children inside a file reference");
  if (
    !allowMissing &&
    reference?.excludedPaths.some(
      (p) => relative === p || relative.startsWith(p + "/"),
    )
  )
    throw new Error(
      "Path is not displayed in this assignment; add it again from course Files",
    );
  const sharedPath = reference
    ? reference.targetPath + relative.slice(reference.path.length)
    : !assignmentId
      ? relative
      : `assignments/${assignmentId}/${relative}`;
  if (sharedPath && excludedCoursePath(sharedPath))
    throw new Error("This path is excluded from course file tools");
  const workspacePath = sharedPath
    ? `courses/${courseId}/files/${sharedPath}`
    : `courses/${courseId}/files/assignments/${assignmentId}/${relative}`;
  const absolutePath = safePath(workspacePath);
  await assertNoSymlink(absolutePath);
  if (reference && !allowMissing) {
    const target = safePath(
      `courses/${courseId}/files/${reference.targetPath}`,
    );
    await assertNoSymlink(target);
    const stat = await fs.stat(target);
    if (reference.type === "directory" ? !stat.isDirectory() : !stat.isFile())
      throw new Error(
        "Shared reference target changed type; remove and recreate the reference",
      );
  }
  return {
    absolutePath,
    workspacePath,
    shared: reference
      ? {
          path: sharedPath!,
          referencePath: reference.path,
          root: relative === reference.path,
        }
      : undefined,
  };
}
