import { z } from "zod";
import { idSchema } from "./schema";

export const assignmentPath = z
  .string()
  .min(1)
  .max(400)
  .refine((value) => {
    const parts = value.split("/");
    return (
      parts.length <= 20 &&
      parts.every(
        (part) =>
          part &&
          part !== "." &&
          part !== ".." &&
          !/[\\:\x00-\x1f<>"|?*]/.test(part) &&
          !/[. ]$/.test(part) &&
          !/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(part),
      )
    );
  }, "Use a relative assignment path without traversal or reserved names");
export const assignmentScope = z.object({
  courseId: idSchema,
  assignmentId: idSchema,
});
export const assignmentFileInput = assignmentScope.extend({
  path: assignmentPath,
});
export const assignmentWriteInput = assignmentFileInput.extend({
  content: z.string().max(14_000_000),
  encoding: z.enum(["utf8", "base64"]).default("utf8"),
  expectedRevision: z.string().optional(),
  explanation: z.string().trim().min(10).max(6000),
  context: z.string().trim().max(8000).optional(),
});
export const assignmentEditInput = assignmentFileInput.extend({
  context: z.string().trim().max(8000).optional(),
  expectedRevision: z.string().min(1),
  edits: z
    .array(
      z.object({
        oldText: z.string().min(1).max(200000),
        newText: z.string().max(200000),
      }),
    )
    .min(1)
    .max(100),
  explanation: z.string().trim().min(10).max(6000),
});
export const assignmentDeleteInput = assignmentFileInput.extend({
  context: z.string().trim().max(8000).optional(),
  expectedRevision: z.string().min(1),
  explanation: z.string().trim().min(10).max(6000),
});
export const assignmentMoveInput = assignmentDeleteInput.extend({
  destination: assignmentPath,
});
export const assignmentReferenceInput = assignmentFileInput.extend({
  path: assignmentPath.refine(
    (value) => !value.includes("/"),
    "References must be placed at the assignment root",
  ),
  targetPath: assignmentPath,
});
export type SharedReference = {
  path: string;
  referencePath: string;
  root: boolean;
  missing?: boolean;
};
export const assignmentReorderInput = assignmentScope.extend({
  paths: z.array(assignmentPath).max(20000),
  directory: z.union([z.literal(""), assignmentPath]).optional(),
  expectedRevision: z.string().min(1),
});
export const assignmentLearningInput = assignmentScope.extend({
  teachingMarkdown: z
    .string()
    .trim()
    .min(200)
    .max(50000)
    .optional()
    .describe(
      "Student-facing lesson for this assignment part, separate from operational markdown. Explain the problem, needed concepts in plain language, the solution mechanism and why it works, a concrete worked example, code connections, and what actual checks establish. No status, tool calls, run IDs or handoff notes. Write a complete explanation of the part, not a change log. Required when completing a numbered part.",
    ),
  teachingForStepId: idSchema
    .optional()
    .describe(
      "Only to improve an existing part's teaching: attach the lesson to this saved step without resnapshotting today's files or altering chronological progress. Requires teachingMarkdown. Use an existing step belonging to the same assignment and part.",
    ),
  part: z
    .object({
      order: z.number().int().min(1).max(1000),
      title: z.string().trim().min(1).max(200),
    })
    .optional()
    .describe(
      "Required for new assignment walkthroughs: stable part number and title, in requirement order. Complete one part before advancing.",
    ),
  phase: z
    .enum(["plan", "work", "check", "blocked", "completed"])
    .default("work")
    .describe(
      "Plan before editing; explain each meaningful change and actual check. completed means this part is locally complete, never submitted.",
    ),
  nextAction: z
    .string()
    .trim()
    .max(4000)
    .default("")
    .describe("Exact next action for the student or resuming agent."),
  title: z.string().trim().min(1).max(200),
  markdown: z.string().trim().min(20).max(50000),
  paths: z.array(assignmentPath).max(100).default([]),
  sourceIds: z.array(z.string()).max(100).default([]),
  gaps: z.array(z.string().max(2000)).max(30).default([]),
});
export type AssignmentEntry = {
  change?: "added" | "changed" | "removed";
  path: string;
  name: string;
  type: "file" | "directory";
  size: number;
  revision: string;
  modifiedAt: string;
  shared?: SharedReference;
  absolutePath?: string;
};
export type AssignmentTree = {
  courseId: string;
  assignmentId?: string;
  root: string;
  absoluteRoot: string;
  entries: AssignmentEntry[];
  revision: string;
};
export type AssignmentFile = {
  snapshotNotice?: string;
  path: string;
  workspacePath: string;
  absolutePath: string;
  revision: string;
  metadataRevision?: string;
  size: number;
  mediaType: string;
  content?: string;
  shared?: SharedReference;
};
