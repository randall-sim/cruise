import { NextRequest, NextResponse } from "@/lib/web-request";
import { z } from "zod";
import {
  addCourse,
  addSemester,
  deleteCourse,
  mutate,
  readState,
  requireCourse,
  writePrivate,
} from "@/lib/store";
import { courseInput, idSchema } from "@/lib/schema";
import { importLecture, addCapture } from "@/lib/lectures";
import { createJob } from "@/lib/jobs";
import { searchMemory } from "@/lib/memory";
import { localRequest, jsonBody, errorResponse } from "@/lib/http";
import { seedDemo } from "@/lib/demo";
import { saveSource } from "@/lib/sources";
import { deleteGuide } from "@/lib/guides";
export const runtime = "nodejs";
export async function POST(request: NextRequest) {
  try {
    localRequest(request, true);
    const { action, data } = await jsonBody(request, 18_000_000);
    let result: unknown;
    switch (action) {
      case "semester.create":
        result = await addSemester(data);
        break;
      case "course.create":
        result = await addCourse(data);
        break;
      case "course.delete":
        result = await deleteCourse(idSchema.parse(data.id));
        break;
      case "course.update":
        result = await mutate(async (state) => {
          const course = requireCourse(state, data.id);
          Object.assign(course, courseInput.parse(data));
          await writePrivate(
            `courses/${course.id}/course.json`,
            JSON.stringify(course, null, 2),
          );
          return course;
        });
        break;
      case "lecture.import":
        result = await importLecture(data);
        break;
      case "guide.delete":
        result = await deleteGuide(data);
        break;
      case "lecture.hide": {
        const input = z
          .object({ courseId: idSchema, lectureId: idSchema })
          .parse(data);
        result = await mutate((state) => {
          requireCourse(state, input.courseId);
          const lecture = state.lectures.find(
            (l) => l.id === input.lectureId && l.courseId === input.courseId,
          );
          if (!lecture) throw new Error("Lecture not found in this course");
          lecture.hiddenFromUi = true;
          return { lectureId: lecture.id, hiddenFromUi: true };
        });
        break;
      }
      case "capture.add":
        result = await addCapture(data);
        break;
      case "lecture.review":
        result = await mutate((state) => {
          const l = state.lectures.find((l) => l.id === data.id);
          if (!l) throw new Error("Lecture not found");
          l.reviewed = z.boolean().parse(data.reviewed);
        });
        break;
      case "job.create":
        result = await createJob(data);
        break;
      case "job.retry":
        result = await mutate((state) => {
          const j = state.jobs.find((j) => j.id === data.id);
          if (!j || j.status !== "failed")
            throw new Error("Only failed jobs can be retried");
          j.status = "queued";
          delete j.error;
        });
        break;
      case "search":
        result = await searchMemory(
          z.string().parse(data.courseId),
          z.string().min(1).max(3000).parse(data.query),
        );
        break;
      case "source.import":
        result = await saveSource(data);
        break;
      case "concept.toggle":
        result = await mutate((state) => {
          const c = state.concepts.find((c) => c.id === data.id);
          if (!c) throw new Error("Concept not found");
          c.mastered = !c.mastered;
        });
        break;
      case "demo":
        if ((await readState()).courses.length)
          throw new Error("Demo can only be loaded into an empty workspace");
        result = await seedDemo();
        break;
      default:
        throw new Error("Unknown action");
    }
    return NextResponse.json({ result: result ?? null });
  } catch (error) {
    return errorResponse(error);
  }
}
