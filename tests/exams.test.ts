import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { addCourse, readPrivate, readState } from "../src/lib/store";
import { importLecture } from "../src/lib/lectures";
import { saveSource } from "../src/lib/sources";
import { allEvidence } from "../src/lib/memory";
import { completeJob } from "../src/lib/jobs";
import {
  prepareExam,
  previewExamScope,
  queueExam,
  withinExamEvidence,
} from "../src/lib/exams";
import type { ExamOutput } from "../src/lib/exam-schema";

let root: string;
const originalRoot = process.env.COURSE_CAPTAIN_WORKSPACE;
beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "course-captain-exam-"));
  process.env.COURSE_CAPTAIN_WORKSPACE = root;
});
afterEach(async () => {
  if (originalRoot) process.env.COURSE_CAPTAIN_WORKSPACE = originalRoot;
  else delete process.env.COURSE_CAPTAIN_WORKSPACE;
  assert.ok(root.startsWith(path.join(os.tmpdir(), "course-captain-exam-")));
  await fs.rm(root, { recursive: true, force: true });
});

async function fixture() {
  const course = await addCourse({
    code: "EX 101",
    name: "Exam course",
    term: "Fall 2026",
  });
  const lectures = [];
  for (const [index, date] of [
    "2026-09-30",
    "2026-10-01",
    "2026-10-15",
    "2026-10-16",
    "2026-11-30",
    "2026-12-01",
  ].entries()) {
    lectures.push(
      await importLecture({
        courseId: course.id,
        title: `Lecture ${index + 1}`,
        date,
        transcript: `WEBVTT\n\n00:00:00.000 --> 00:00:10.000\nBinary search halves a sorted interval. Lecture topic ${index + 1}.`,
      }),
    );
  }
  const source = await saveSource({
    courseId: course.id,
    title: "Official exam format and midterm date",
    url: "https://example.edu/exams",
    text: "The last midterm was October 15, 2026. The practice paper uses multiple choice and short answers. Binary search requires a sorted interval.",
  });
  const evidence = await allEvidence(course.id);
  const format = evidence.find(
    (s) => s.path === source.path && s.text.includes("last midterm"),
  )!;
  return { course, lectures, format };
}

function output(citation: string): ExamOutput {
  return {
    title: "Original search practice exam",
    instructions: "Attempt both questions before revealing answers.",
    format: {
      inferred: true,
      rationale: "An inferred two-question practice mix.",
      citations: [],
    },
    questions: [
      {
        type: "multiple_choice",
        topic: "Search condition",
        prompt:
          "What condition permits repeatedly halving the search interval?",
        points: 2,
        choices: [
          { id: "A", text: "The interval is sorted" },
          { id: "B", text: "The values are random" },
        ],
        correctChoiceIds: ["A"],
        answer: "The interval must be sorted.",
        explanation:
          "Ordering allows us to discard the half that cannot contain the target.",
        rubric: [{ criterion: "Select the sorted interval", points: 2 }],
        citations: [citation],
      },
      {
        type: "short_answer",
        topic: "Halving",
        prompt: "Explain why ordering makes halving possible.",
        points: 3,
        choices: [],
        correctChoiceIds: [],
        answer: "Compare the target with the middle value.",
        explanation:
          "Ordering identifies which half can still contain the target.",
        rubric: [
          { criterion: "Use the middle comparison", points: 1 },
          { criterion: "Justify discarding the other half", points: 2 },
        ],
        citations: [citation],
      },
    ],
    gaps: [],
  };
}

test("exam date ranges are inclusive and explicit lecture IDs are course-scoped", async () => {
  const { course, lectures } = await fixture();
  const input = {
    courseId: course.id,
    prompt: "October through November",
    scope: { mode: "dates", from: "2026-10-01", through: "2026-11-30" },
  };
  const before = await readPrivate("state.json");
  const preview = await previewExamScope(input);
  assert.deepEqual(
    preview.lectures.map((l) => l.id),
    lectures.slice(1, 5).map((l) => l.id),
  );
  assert.equal(await readPrivate("state.json"), before);
  const explicit = await previewExamScope({
    ...input,
    scope: { mode: "lectures", lectureIds: [lectures[4].id, lectures[1].id] },
  });
  assert.deepEqual(
    explicit.lectures.map((l) => l.id),
    [lectures[1].id, lectures[4].id],
  );
  await assert.rejects(
    () =>
      previewExamScope({
        ...input,
        scope: { mode: "dates", from: "2026-11-30", through: "2026-10-01" },
      }),
    /reversed/,
  );
  await assert.rejects(() =>
    previewExamScope({
      ...input,
      scope: { mode: "dates", from: "2026-02-30", through: "2026-10-01" },
    }),
  );
  await assert.rejects(
    () =>
      previewExamScope({
        ...input,
        scope: { mode: "dates", from: "2025-01-01", through: "2025-12-31" },
      }),
    /No evidence/,
  );
  const other = await addCourse({ code: "OTHER", name: "Other course" });
  await assert.rejects(
    () =>
      previewExamScope({
        ...input,
        courseId: other.id,
        scope: { mode: "lectures", lectureIds: [lectures[1].id] },
      }),
    /not in this course/,
  );
  await assert.rejects(
    () =>
      previewExamScope({
        ...input,
        contentSourceIds: [`${lectures[0].id}:t0`],
      }),
    /outside the exam scope/,
  );
});

test("after-event scopes require real source IDs and exclude the cutoff day", async () => {
  const { course, lectures, format } = await fixture();
  const input = {
    courseId: course.id,
    prompt: "After the last midterm",
    scope: {
      mode: "after_event",
      event: "Midterm 1",
      date: "2026-10-15",
      through: "2026-11-30",
      evidenceIds: [format.id],
    },
  };
  const preview = await previewExamScope(input);
  assert.deepEqual(
    preview.lectures.map((l) => l.id),
    [lectures[3].id, lectures[4].id],
  );
  assert.equal(preview.boundaryEvidence[0].id, format.id);
  await assert.rejects(() =>
    previewExamScope({ ...input, scope: { ...input.scope, evidenceIds: [] } }),
  );
  await assert.rejects(
    () =>
      previewExamScope({
        ...input,
        scope: { ...input.scope, evidenceIds: ["invented"] },
      }),
    /Event boundary/,
  );
  const discovery = await prepareExam(course.id, input.prompt);
  assert.equal(discovery.status, "discovery_required");
  assert.ok(discovery.evidence.some((s) => s.id === format.id));
  assert.equal((await readState()).jobs.length, 0);
});

test("exam content, format and cutoff evidence have separate roles that retrieval cannot broaden", async () => {
  const { course, lectures, format } = await fixture();
  const job = await queueExam({
    courseId: course.id,
    prompt: "Practice lecture 4",
    questionCount: 2,
    scope: { mode: "lectures", lectureIds: [lectures[3].id] },
    formatSourceIds: [format.id],
  });
  const exam = output(`${lectures[3].id}:t0`);
  exam.questions[0].citations = [format.id];
  await assert.rejects(() => completeJob(job.id, exam), /scope/);
  exam.questions[0].citations = [`${lectures[0].id}:t0`];
  await assert.rejects(() => completeJob(job.id, exam), /scope/);
  const pool = withinExamEvidence(job, await allEvidence(course.id));
  assert.ok(!pool.some((s) => s.lectureId === lectures[0].id));
  assert.ok(pool.some((s) => s.id === format.id));
  const notesOnly = await queueExam({
    courseId: course.id,
    prompt: "Practice this saved note",
    questionCount: 2,
    scope: { mode: "dates", from: "2025-01-01", through: "2025-12-31" },
    contentSourceIds: [format.id],
  });
  assert.deepEqual(notesOnly.examRequest?.lectureIds, []);
  assert.ok(notesOnly.examRequest?.contentEvidenceIds.includes(format.id));
});

test("exam completion validates question counts, type mix, choices, rubrics and format attribution", async () => {
  const { course, lectures } = await fixture();
  const job = await queueExam({
    courseId: course.id,
    prompt: "Mixed exam",
    questionCount: 2,
    questionTypes: ["multiple_choice", "short_answer"],
    scope: { mode: "lectures", lectureIds: [lectures[1].id] },
  });
  const valid = output(`${lectures[1].id}:t0`);
  const badCount = structuredClone(valid);
  badCount.questions.pop();
  await assert.rejects(() => completeJob(job.id, badCount), /count/);
  const badType = structuredClone(valid);
  badType.questions[1].type = "essay";
  await assert.rejects(() => completeJob(job.id, badType), /Missing requested/);
  const badChoice = structuredClone(valid);
  badChoice.questions[0].correctChoiceIds = ["C"];
  await assert.rejects(() => completeJob(job.id, badChoice), /choice IDs/);
  const badRubric = structuredClone(valid);
  badRubric.questions[0].rubric[0].points = 1;
  await assert.rejects(() => completeJob(job.id, badRubric), /Rubric/);
  const badFormat = structuredClone(valid);
  badFormat.format.inferred = false;
  await assert.rejects(() => completeJob(job.id, badFormat), /inferred/);
  await completeJob(job.id, valid);
  const saved = (await readState()).jobs.find((j) => j.id === job.id)!;
  assert.equal(saved.status, "completed");
  assert.equal(saved.exam?.questions.length, 2);
  const paper = await readPrivate(
    `courses/${course.id}/exams/${job.id}/exam.md`,
  );
  const key = await readPrivate(
    `courses/${course.id}/exams/${job.id}/answer-key.md`,
  );
  assert.ok(!paper.includes(valid.questions[0].explanation));
  assert.ok(key.includes(valid.questions[0].explanation));
  assert.ok(key.includes(`${lectures[1].id}:t0`));
  assert.deepEqual(
    await fs.readdir(path.join(root, `courses/${course.id}/exams/${job.id}`)),
    ["answer-key.md", "exam.md"],
  );
});

test("all supported question types can be saved and omitted lecture coverage is flagged", async () => {
  const { course, lectures } = await fixture();
  const kinds = [
    "multiple_choice",
    "multiple_select",
    "true_false",
    "short_answer",
    "essay",
    "calculation",
    "proof",
    "code",
    "diagram",
  ] as const;
  const job = await queueExam({
    courseId: course.id,
    prompt: "Try all supported types",
    questionCount: 9,
    questionTypes: [...kinds],
    scope: { mode: "lectures", lectureIds: [lectures[1].id, lectures[2].id] },
  });
  const exam = output(`${lectures[1].id}:t0`);
  exam.questions = kinds.map((type) => {
    const q = structuredClone(
      exam.questions[type === "multiple_choice" ? 0 : 1],
    );
    q.type = type;
    if (type === "multiple_select") {
      q.choices = [
        { id: "A", text: "One" },
        { id: "B", text: "Two" },
        { id: "C", text: "Neither" },
      ];
      q.correctChoiceIds = ["A", "B"];
    }
    if (type === "true_false") {
      q.choices = [
        { id: "T", text: "True" },
        { id: "F", text: "False" },
      ];
      q.correctChoiceIds = ["T"];
    }
    return q;
  });
  await completeJob(job.id, exam);
  const saved = (await readState()).jobs.find((j) => j.id === job.id)!;
  assert.ok(saved.exam?.gaps.some((gap) => gap.includes(lectures[2].id)));
});
