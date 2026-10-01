import { promises as fs } from "node:fs";
import { z } from "zod";
import { idSchema } from "./schema";
import {
  assertNoSymlink,
  mutate,
  newId,
  readPrivate,
  requireCourse,
  safePath,
  writePrivate,
} from "./store";

export const deleteGuideInput = z.object({
  courseId: idSchema,
  lectureId: idSchema,
});

export async function deleteGuide(input: unknown) {
  const { courseId, lectureId } = deleteGuideInput.parse(input);
  return mutate(async (state) => {
    requireCourse(state, courseId);
    const lecture = state.lectures.find(
      (l) => l.id === lectureId && l.courseId === courseId,
    );
    if (!lecture) throw new Error("Lecture not found in this course");
    if (!lecture.guide) return { lectureId, deleted: false };
    const notesPath = `courses/${courseId}/lectures/${lectureId}/notes.md`;
    if (lecture.notesPath !== notesPath)
      throw new Error("Guide file path does not match this lecture");
    const notesFile = safePath(notesPath);
    await assertNoSymlink(notesFile);
    let markdown: string | null = null;
    try {
      markdown = await readPrivate(notesPath);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    const jobs = state.jobs.filter(
      (j) =>
        j.kind === "lecture" &&
        j.lectureId === lectureId &&
        j.courseId === courseId,
    );
    const concepts = state.concepts.filter(
      (c) => c.lectureId === lectureId && c.courseId === courseId,
    );
    const recoveryPath = `.trash/guides/${courseId}/${lectureId}/${newId()}.json`;
    // Recovery data is outside indexed memory. Preserve it before any removal.
    await writePrivate(
      recoveryPath,
      JSON.stringify(
        {
          deletedAt: new Date().toISOString(),
          courseId,
          lectureId,
          notesPath,
          markdown,
          guide: lecture.guide,
          evidence: lecture.evidence,
          reviewed: lecture.reviewed,
          jobs,
          concepts,
        },
        null,
        2,
      ),
    );
    await fs.unlink(notesFile).catch((error: NodeJS.ErrnoException) => {
      if (error.code !== "ENOENT") throw error;
    });
    delete lecture.guide;
    delete lecture.evidence;
    lecture.status = "imported";
    lecture.reviewed = false;
    state.concepts = state.concepts.filter(
      (c) => !(c.lectureId === lectureId && c.courseId === courseId),
    );
    const removedJobs = new Set(jobs.map((j) => j.id));
    // Removing queued guide jobs prevents an already-dispatched completion from
    // restoring the guide. A new explicit Build guide request gets a fresh job.
    state.jobs = state.jobs.filter((j) => !removedJobs.has(j.id));
    return {
      lectureId,
      deleted: true,
      recoveryPath,
      removedGuideJobs: jobs.length,
    };
  });
}
