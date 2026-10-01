import { z } from "zod";
import { idSchema, type State } from "./schema";
import { mutate, newId, readState, requireCourse, writePrivate } from "./store";

export type Assignment = {
  id: string;
  courseId: string;
  title: string;
  description: string;
  createdAt: string;
  legacyJobId?: string;
};
export const createAssignmentInput = z.object({
  courseId: idSchema,
  title: z.string().trim().min(1).max(200),
  description: z.string().max(12000).default(""),
});
export const assignmentIdentity = z.object({
  courseId: idSchema,
  assignmentId: idSchema,
});
export function courseAssignments(
  state: State,
  courseId: string,
): Assignment[] {
  requireCourse(state, courseId);
  return [
    ...(state.assignments || []).filter((a) => a.courseId === courseId),
    ...state.jobs
      .filter(
        (j) =>
          j.courseId === courseId &&
          j.kind === "assignment" &&
          !(state.assignments || []).some((a) => a.id === j.id),
      )
      .map((j) => ({
        id: j.id,
        courseId,
        title: j.prompt.slice(0, 200),
        description: j.prompt,
        createdAt: j.createdAt,
        legacyJobId: j.id,
      })),
  ]
    .filter((a) => !state.hiddenAssignments?.includes(a.id))
    .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}
export function requireAssignment(
  state: State,
  courseId: string,
  assignmentId: string,
) {
  assignmentIdentity.parse({ courseId, assignmentId });
  const assignment = courseAssignments(state, courseId).find(
    (a) => a.id === assignmentId,
  );
  if (!assignment) throw new Error("Assignment not found in this course");
  return assignment;
}
export async function listAssignments(input: unknown) {
  const { courseId } = z.object({ courseId: idSchema }).parse(input);
  return courseAssignments(await readState(), courseId);
}
export async function createAssignment(input: unknown) {
  const data = createAssignmentInput.parse(input);
  return mutate(async (state) => {
    requireCourse(state, data.courseId);
    const assignment: Assignment = {
      ...data,
      id: newId(),
      createdAt: new Date().toISOString(),
    };
    state.assignments ||= [];
    state.assignments.push(assignment);
    await writePrivate(
      `courses/${data.courseId}/agent/assignments/${assignment.id}.json`,
      JSON.stringify(assignment, null, 2),
    );
    return {
      ...assignment,
      root: `courses/${data.courseId}/files/assignments/${assignment.id}`,
      instructions:
        "The student slider must have one stop per actual assignment part (one part for an undivided task), never per tool call or implementation phase. Save operational checkpoints in markdown and a substantial, separate teachingMarkdown explaining concepts, worked examples, solution mechanisms and why the choices work. Do not show status/handoff notes as lessons. " +
        "Work through ordered parts sequentially as a lecturer demonstrating to a student. Save an initial plan and teaching checkpoints with record_assignment_learning (part order/title, phase, nextAction); explain each change, the evidence-backed decision and actual checks. Complete the current part before advancing. get_assignment_timeline reads saved explanations and exact workspace states. Follow docs/ASSIGNMENTS.md. " +
        "All assignment files live in course Files and are reusable by every assignment. An assignment only selects which files/folders to display. Assignment tools automatically save new files under files/assignments/<assignmentId>/ and include them in its view; existing folder references write to the original. Removing an assignment item only changes its view; actual deletion uses course file tools. Reuse originals with reference_course_path instead of copying them. For assignment work, cross-reference the course knowledge base before drafting an approach or code, throughout substantive work, and before completion. Use search_course and read relevant lecture transcript/slide evidence AND past assignment Markdown learning, feedback, mistakes, checks and reusable course files. Follow taught definitions, methods and conventions; connect major decisions to verified source IDs, lecture timestamps and prior assignment/file paths. Cross-check past generated answers against instructor evidence and current requirements before reuse. Save these connections, conflicts and searched-but-missing evidence in progress checkpoints; never invent support. Read the latest assignment checkpoint before resuming and reconcile it with current files and command runs. Use record_assignment_learning before starting, after each meaningful step, decision, edit batch, check or blocker, and before pausing. Include course/assignment IDs, completed/pending work, rationale, evidence, changed paths, command run IDs/results and exact next actions. Verify the saved path and indexed flag; follow docs/ASSIGNMENTS.md. Do not wait until completion. Include course evidence and explicit gaps. Never submit coursework.",
    };
  });
}
export async function deleteAssignment(input: unknown) {
  const data = assignmentIdentity.parse(input);
  return mutate(async (state) => {
    const assignment = requireAssignment(
      state,
      data.courseId,
      data.assignmentId,
    );
    const recoveryPath = `.trash/assignments/${newId()}/assignment.json`;
    await writePrivate(recoveryPath, JSON.stringify(assignment, null, 2));
    state.hiddenAssignments ||= [];
    state.hiddenAssignments.push(assignment.id);
    // Preserve its files and learning history. Hidden assignments cannot be accessed
    // by file tools; recovery is possible without deleting course evidence.
    return { id: assignment.id, deleted: true, recoveryPath };
  });
}
