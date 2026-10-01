import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import path from "node:path";
import os from "node:os";
import sharp from "sharp";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import {
  addCourse,
  mutate,
  readState,
  readPrivate,
  safePath,
  writePrivate,
} from "../src/lib/store";
import { parseTranscript } from "../src/lib/transcript";
import { addCapture, importLecture } from "../src/lib/lectures";
import { completeJob, createJob } from "../src/lib/jobs";
import { searchMemory } from "../src/lib/memory";
import { saveSource } from "../src/lib/sources";
import { deleteGuide } from "../src/lib/guides";
import { prepareLectureAgent, lectureStatus } from "../src/lib/lecture-agent";
import { getTaskWorkflow } from "../src/lib/task-workflow";
import { localRequest } from "../src/lib/http";
import { NextRequest } from "next/server";

let root: string;
const originalFetch = globalThis.fetch;
const originalRoot = process.env.COURSE_CAPTAIN_WORKSPACE;
const originalNotion = process.env.NOTION_TOKEN;
const originalCanvas = process.env.CANVAS_TOKEN;
beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "course-captain-test-"));
  process.env.COURSE_CAPTAIN_WORKSPACE = root;
  delete process.env.NOTION_TOKEN;
  delete process.env.CANVAS_TOKEN;
});
afterEach(async () => {
  globalThis.fetch = originalFetch;
  if (originalRoot) process.env.COURSE_CAPTAIN_WORKSPACE = originalRoot;
  else delete process.env.COURSE_CAPTAIN_WORKSPACE;
  if (originalNotion) process.env.NOTION_TOKEN = originalNotion;
  else delete process.env.NOTION_TOKEN;
  if (originalCanvas) process.env.CANVAS_TOKEN = originalCanvas;
  else delete process.env.CANVAS_TOKEN;
  assert.ok(root.startsWith(path.join(os.tmpdir(), "course-captain-test-")));
  await fs.rm(root, { recursive: true, force: true });
});
const transcript =
  "WEBVTT\n\n00:00:01.000 --> 00:00:12.000\nBinary search halves a sorted interval.\n\n00:00:12.000 --> 00:00:22.000\nThe runtime is logarithmic because one half remains at each step.";
async function fixture() {
  const course = await addCourse({ code: "CS 1", name: "Algorithms" });
  const lecture = await importLecture({
    courseId: course.id,
    title: "Binary search",
    date: "2026-09-20",
    transcript,
  });
  return { course, lecture };
}

test("VTT and SRT preserve timing, clean cue tags, and reject invented timing", () => {
  assert.deepEqual(parseTranscript(transcript)[0], {
    start: 1,
    end: 12,
    text: "Binary search halves a sorted interval.",
  });
  assert.deepEqual(
    parseTranscript(
      "1\n00:01:01,200 --> 00:01:03,900\n<b>Sorted</b> array\n",
    )[0],
    { start: 61.2, end: 63.9, text: "Sorted array" },
  );
  assert.throws(
    () => parseTranscript("Some plain notes without timestamps"),
    /timestamped/,
  );
  assert.throws(
    () => parseTranscript("00:00:04.000 --> 00:00:01.000\nBad interval"),
    /ends before/,
  );
});
test("private storage rejects traversal and symlinks", async () => {
  assert.throws(() => safePath("../../outside"), /outside/);
  assert.throws(() => safePath("."), /outside/);
  await fs.symlink(os.tmpdir(), path.join(root, "escape"));
  await assert.rejects(() => writePrivate("escape/leak.md", "no"), /Symlinks/);
});
test("concurrent mutations retain all course records and write private Markdown", async () => {
  const courses = await Promise.all(
    Array.from({ length: 12 }, (_, i) =>
      addCourse({ code: `CS ${i}`, name: `Course ${i}` }),
    ),
  );
  assert.equal((await readState()).courses.length, 12);
  assert.match(
    await readPrivate(`courses/${courses[0].id}/AGENTS.md`),
    /Never submit anything to official course submission destinations/,
  );
  assert.equal(
    (await fs.stat(path.join(root, "state.json"))).mode & 0o777,
    0o600,
  );
});
test("retrieval is scoped to the requested course, with source timestamps", async () => {
  const { course, lecture } = await fixture();
  const other = await addCourse({ code: "BIO", name: "Biology" });
  await saveSource({
    courseId: other.id,
    title: "Secret biology",
    text: "Binary search is mentioned only in this other private course.",
  });
  const sources = await searchMemory(course.id, "binary search sorted");
  assert.ok(sources.length > 0);
  assert.ok(sources.every((s) => s.courseId === course.id));
  assert.ok(sources.some((s) => s.lectureId === lecture.id && s.seconds === 1));
  assert.equal((await searchMemory(course.id, "photosynthesis")).length, 0);
});
test("long Markdown paragraphs are indexed without dropping their ending", async () => {
  const { course } = await fixture();
  await saveSource({
    courseId: course.id,
    title: "Website syllabus",
    url: "https://example.edu/syllabus",
    text:
      "ordinary words ".repeat(600) + " uniquegradingpolicy weekly assessment",
  });
  const results = await searchMemory(course.id, "uniquegradingpolicy");
  assert.equal(results.length, 1);
  assert.equal(results[0].url, "https://example.edu/syllabus");
});
test("lecture completion rejects fabricated citation IDs and writes notes, glossary and quiz", async () => {
  const { course, lecture } = await fixture();
  const job = await createJob({
    courseId: course.id,
    lectureId: lecture.id,
    kind: "lecture",
    prompt: "Explain all of the lecture",
  });
  const output = {
    summary: "Binary search eliminates half of a sorted interval each step.",
    sections: [
      {
        title: "How it works",
        markdown: "Halve the sorted interval.",
        citations: ["invented"],
      },
    ],
    concepts: [
      {
        term: "Binary search",
        definition: "Repeated halving of a sorted interval.",
        citations: [`${lecture.id}:t0`],
      },
    ],
    questions: [
      {
        question: "Why sorted?",
        answer: "Halving depends on order.",
        citations: [`${lecture.id}:t0`],
      },
    ],
    logistics: [],
    gaps: [],
  };
  await assert.rejects(() => completeJob(job.id, output), /outside this job/);
  assert.equal((await readState()).jobs[0].status, "queued");
  output.sections[0].citations = [`${lecture.id}:t0`];
  await completeJob(job.id, output);
  const state = await readState();
  assert.equal(state.lectures[0].status, "ready");
  assert.equal(state.concepts[0].term, "Binary search");
  assert.match(
    await readPrivate(lecture.notesPath),
    /Halve the sorted interval/,
  );
  assert.doesNotMatch(await readPrivate(lecture.notesPath), /## Word bank/);
  assert.match(await readPrivate(lecture.notesPath), /### 1\. Why sorted\?/);
  assert.match(
    await readPrivate(lecture.notesPath),
    /Halving depends on order/,
  );
  assert.ok(
    state.lectures[0].guide?.gaps.some((g) => g.includes("Transcript only")),
  );
  await assert.rejects(() => completeJob(job.id, output), /not queued/);
});
test("deleting a guide preserves raw evidence, other work and a recovery copy; stale jobs cannot resurrect it", async () => {
  const { course, lecture } = await fixture();
  const png = await sharp({
    create: { width: 8, height: 8, channels: 3, background: "green" },
  })
    .png()
    .toBuffer();
  const capture = await addCapture({
    lectureId: lecture.id,
    seconds: 12,
    kind: "slide",
    stream: "Slides",
    image: `data:image/png;base64,${png.toString("base64")}`,
  });
  const output = {
    summary: "A detailed generated explanation of binary search.",
    sections: [
      {
        title: "Halving",
        markdown: "Repeated halving",
        citations: [`${lecture.id}:t0`],
      },
    ],
    concepts: [
      {
        term: "Halving",
        definition: "Keep half.",
        citations: [`${lecture.id}:t0`],
      },
    ],
    questions: [
      {
        question: "Why sorted?",
        answer: "Order permits elimination.",
        citations: [`${lecture.id}:t0`],
      },
    ],
    logistics: [],
    gaps: [],
  };
  const guideJob = await createJob({
    courseId: course.id,
    lectureId: lecture.id,
    kind: "lecture",
    prompt: "Create guide",
  });
  await completeJob(guideJob.id, output);
  const queued = await createJob({
    courseId: course.id,
    lectureId: lecture.id,
    kind: "lecture",
    prompt: "Rebuild guide",
  });
  const question = await createJob({
    courseId: course.id,
    lectureId: lecture.id,
    kind: "question",
    prompt: "Explain binary search",
  });
  await completeJob(question.id, {
    markdown: "Unrelated study answer",
    citations: [`${lecture.id}:t0`],
    gaps: [],
  });
  const other = await fixture();
  const otherJob = await createJob({
    courseId: other.course.id,
    lectureId: other.lecture.id,
    kind: "lecture",
    prompt: "Keep this job",
  });
  const before = await readState();
  const originalTranscript = await readPrivate(lecture.transcriptPath);
  const originalNotes = await readPrivate(lecture.notesPath);
  await assert.rejects(
    () => deleteGuide({ courseId: other.course.id, lectureId: lecture.id }),
    /not found in this course/,
  );
  const result = await deleteGuide({
    courseId: course.id,
    lectureId: lecture.id,
  });
  assert.equal(result.deleted, true);
  const after = await readState();
  const current = after.lectures.find((l) => l.id === lecture.id)!;
  assert.equal(current.guide, undefined);
  assert.equal(current.evidence, undefined);
  assert.equal(current.status, "imported");
  assert.equal(current.reviewed, false);
  assert.deepEqual(current.cues, lecture.cues);
  assert.deepEqual(current.captures, before.lectures[0].captures);
  assert.equal(await readPrivate(lecture.transcriptPath), originalTranscript);
  assert.deepEqual(await fs.readFile(safePath(capture.file)), png);
  await assert.rejects(() => readPrivate(lecture.notesPath), {
    code: "ENOENT",
  });
  assert.ok(!after.concepts.some((c) => c.lectureId === lecture.id));
  assert.ok(
    !after.jobs.some((j) => j.id === queued.id || j.id === guideJob.id),
  );
  assert.ok(
    after.jobs.some(
      (j) =>
        j.id === question.id && j.result?.markdown === "Unrelated study answer",
    ),
  );
  assert.ok(after.jobs.some((j) => j.id === otherJob.id));
  const recovery = JSON.parse(await readPrivate(result.recoveryPath!));
  assert.equal(recovery.markdown, originalNotes);
  assert.equal(recovery.guide.questions.length, 1);
  assert.equal(recovery.concepts.length, 1);
  await assert.rejects(() => completeJob(queued.id, output), /not queued/);
  assert.equal(
    (await deleteGuide({ courseId: course.id, lectureId: lecture.id })).deleted,
    false,
  );
  const rebuilt = await createJob({
    courseId: course.id,
    lectureId: lecture.id,
    kind: "lecture",
    prompt: "Rebuild guide",
  });
  await completeJob(rebuilt.id, output);
  assert.equal(
    (await readState()).lectures.find((l) => l.id === lecture.id)!.status,
    "ready",
  );
});

test("lecture status counts visual continuation pages without duplicating linked captures", async () => {
  const { course, lecture } = await fixture();
  const png = await sharp({
    create: { width: 8, height: 8, channels: 3, background: "green" },
  })
    .png()
    .toBuffer();
  const capture = await addCapture({
    lectureId: lecture.id,
    seconds: 12,
    kind: "slide",
    stream: "Slides",
    image: `data:image/png;base64,${png.toString("base64")}`,
  });
  const job = await createJob({
    courseId: course.id,
    lectureId: lecture.id,
    kind: "lecture",
    prompt: "Build guide",
  });
  const visual = `${lecture.id}:c:${capture.id}`;
  await completeJob(job.id, {
    summary: "Binary search locates values in sorted data.",
    sections: [
      {
        title: "Search",
        markdown: "Choose the midpoint.",
        citations: [visual, `${lecture.id}:t0`],
      },
      {
        title: "Search continued",
        markdown: "Discard the impossible half.",
        citations: [visual, `${lecture.id}:t0`],
      },
      {
        title: "Closing remarks",
        markdown: "Repeated halving gives logarithmic cost.",
        citations: [`${lecture.id}:t1`],
      },
    ],
    concepts: [],
    questions: [],
    logistics: [],
    gaps: [],
  });
  const status = await lectureStatus(course.id, lecture.id);
  assert.equal(status.sectionCount, 3);
  assert.equal(status.visualPageCount, 2);
  assert.equal(status.textPageCount, 1);
  assert.equal(status.linkedCaptureCount, 1);
  assert.equal("guide" in status, false);
});

test("lecture delegation is bounded, history-free, course-scoped and reuses recording identity", async () => {
  const { course, lecture } = await fixture();
  await mutate((state) => {
    state.lectures[0].sourceUrl = "https://example.edu/recording/1";
  });
  const packet = await prepareLectureAgent({
    courseId: course.id,
    sourceUrl: "https://example.edu/recording/1",
  });
  assert.equal(packet.status, "prepared_not_spawned");
  assert.equal(packet.spawnArguments.model, "gpt-6-astra");
  assert.equal(packet.spawnArguments.reasoning_effort, "medium");
  assert.equal(packet.spawnArguments.fork_turns, "none");
  assert.match(packet.spawnArguments.message, new RegExp(lecture.id));
  assert.ok(packet.spawnArguments.message.length < 3000);
  assert.ok(
    !packet.spawnArguments.message.includes(
      "Binary search halves a sorted interval",
    ),
  );
  assert.equal((await readState()).jobs.length, 0);
  const status = await lectureStatus(course.id, lecture.id);
  assert.equal(status.hasGuide, false);
  assert.equal(status.sectionCount, 0);
  assert.equal(status.visualPageCount, 0);
  assert.equal(status.textPageCount, 0);
  assert.equal(status.linkedCaptureCount, 0);
  assert.equal("cues" in status, false);
  assert.equal("guide" in status, false);
  const other = await addCourse({ code: "OTHER", name: "Other course" });
  await assert.rejects(
    () => prepareLectureAgent({ courseId: other.id, lectureId: lecture.id }),
    /not found in this course/,
  );
  await assert.rejects(
    () => lectureStatus(other.id, lecture.id),
    /not found in this course/,
  );
  await assert.rejects(
    () =>
      prepareLectureAgent({ courseId: course.id, title: "Missing recording" }),
    /Provide lectureId/,
  );
});
test("saved lecture cheat guide embeds cited images and reports readiness without evidence payloads", async () => {
  const { course, lecture } = await fixture();
  const png = await sharp({
    create: { width: 8, height: 8, channels: 3, background: "green" },
  })
    .png()
    .toBuffer();
  const capture = await addCapture({
    lectureId: lecture.id,
    seconds: 12,
    kind: "whiteboard",
    stream: "Board",
    image: `data:image/png;base64,${png.toString("base64")}`,
  });
  const job = await createJob({
    courseId: course.id,
    lectureId: lecture.id,
    kind: "lecture",
    prompt: "Full lecture-skip cheat guide",
  });
  await completeJob(job.id, {
    summary: "Binary search halves a sorted interval each step.",
    sections: [
      {
        title: "12s — Worked example",
        markdown:
          "Fixture image used to test embedding, not a real lecture diagram.",
        citations: [`${lecture.id}:t0`, `${lecture.id}:c:${capture.id}`],
      },
    ],
    concepts: [],
    questions: [],
    logistics: [],
    gaps: ["Test fixture only"],
  });
  const markdown = await readPrivate(lecture.notesPath);
  assert.ok(markdown.includes(`(<captures/${capture.id}.png>)`));
  const status = await lectureStatus(course.id, lecture.id);
  assert.equal(status.status, "ready");
  assert.equal(status.hasGuide, true);
  assert.equal(status.jobs[0].status, "completed");
  assert.equal("markdown" in status, false);
  assert.equal("evidence" in status, false);
});
test("assignment drafts save inert Markdown with no submission or executable path", async () => {
  const { course } = await fixture();
  const job = await createJob({
    courseId: course.id,
    kind: "assignment",
    prompt: "Explain binary search with a private code draft",
  });
  const output = {
    markdown:
      "Plan: identify the sorted interval.\n\n```js\nconst midpoint = Math.floor((lo + hi) / 2);\n```",
    citations: [job.context[0].id],
    gaps: [],
  };
  await completeJob(job.id, output);
  const dir = path.join(
    root,
    `courses/${course.id}/files/assignments/${job.id}`,
  );
  const files = await fs.readdir(dir);
  assert.deepEqual(files, ["draft.md"]);
  assert.match(
    await fs.readFile(path.join(dir, files[0]), "utf8"),
    /const midpoint/,
  );
});
test("empty answers must declare missing evidence and jobs cannot cross course IDs", async () => {
  const { lecture } = await fixture();
  const other = await addCourse({ code: "MATH", name: "Math" });
  await assert.rejects(
    () =>
      createJob({
        courseId: other.id,
        lectureId: lecture.id,
        kind: "lecture",
        prompt: "Cross course",
      }),
    /not found in this course/,
  );
  const job = await createJob({
    courseId: other.id,
    kind: "question",
    prompt: "What does topology mean?",
  });
  await assert.rejects(
    () =>
      completeJob(job.id, {
        markdown: "An unsupported claim",
        citations: [],
        gaps: [],
      }),
    /explicit evidence gap/,
  );
  await completeJob(job.id, {
    markdown: "This course has no topology source material yet.",
    citations: [],
    gaps: ["No supporting course evidence was retrieved."],
  });
});
test("screenshots are normalized to PNG, scoped, and bounded by lecture timing", async () => {
  const { lecture } = await fixture();
  const png = await sharp({
    create: { width: 8, height: 8, channels: 3, background: "green" },
  })
    .png()
    .toBuffer();
  const image = `data:image/png;base64,${png.toString("base64")}`;
  const capture = await addCapture({
    lectureId: lecture.id,
    seconds: 12,
    kind: "slide",
    stream: "Slides",
    image,
  });
  assert.ok(capture.file.endsWith(".png"));
  assert.equal((await readState()).lectures[0].captures.length, 1);
  await assert.rejects(
    () =>
      addCapture({
        lectureId: lecture.id,
        seconds: 9999,
        kind: "slide",
        stream: "Slides",
        image,
      }),
    /outside the lecture/,
  );
  await assert.rejects(
    () =>
      addCapture({
        lectureId: lecture.id,
        seconds: 2,
        kind: "slide",
        stream: "Slides",
        image: "data:image/svg+xml;base64,AAAA",
      }),
    /Only PNG/,
  );
});
test("local API rejects foreign origins and hostnames", () => {
  const allowed = new NextRequest("http://localhost:3000/api/action", {
    headers: { host: "localhost:3000", origin: "http://localhost:3000" },
  });
  assert.doesNotThrow(() => localRequest(allowed, true));
  assert.throws(
    () =>
      localRequest(
        new NextRequest("http://localhost:3000/api/action", {
          headers: { host: "localhost:3000", origin: "https://evil.example" },
        }),
        true,
      ),
    /same-origin/,
  );
  assert.throws(
    () =>
      localRequest(
        new NextRequest("http://evil.example/api/workspace", {
          headers: { host: "evil.example" },
        }),
      ),
    /loopback/,
  );
});

test("browser task briefs preserve legacy records without fetching or writing status", async () => {
  const { course } = await fixture();
  await mutate((state) => {
    state.tasks.push({
      id: "legacy-task",
      courseId: course.id,
      source: "notion",
      title: "Historical problem set",
      description: "A saved old requirement",
      due: "2025-01-01",
      done: false,
      url: "https://www.notion.so/legacy",
    });
    state.settings.autoSync = true;
  });
  const original = await readPrivate("state.json");
  globalThis.fetch = async () => {
    throw new Error("No direct account requests allowed");
  };
  const brief = await getTaskWorkflow({
    courseId: course.id,
    intent: "update_status",
    request: "Mark the problem set done in Notion",
  });
  assert.equal(brief.status, "browser_action_required");
  assert.equal("notion" in brief.sources, false);
  assert.match(brief.instructions, /Notion is no longer used/);
  assert.match(brief.instructions, /complete an assignment means do/);
  assert.equal(await readPrivate("state.json"), original);
  const evidence = await searchMemory(course.id, "Historical problem set");
  assert.match(
    evidence.find((s) => s.id === "task:legacy-task")!.text,
    /Historical task snapshot/,
  );
  assert.equal((await readState()).tasks[0].done, false);
});

test("MCP bridge completes a real queued job and exposes local course commands without submission tools", async () => {
  const { course } = await fixture();
  const job = await createJob({
    courseId: course.id,
    kind: "question",
    prompt: "How does binary search work?",
  });
  const client = new Client({ name: "course-captain-test", version: "1" });
  const transport = new StdioClientTransport({
    command: process.execPath,
    args: [path.resolve("scripts/start-mcp.mjs")],
    cwd: os.tmpdir(),
    env: { PATH: process.env.PATH!, COURSE_CAPTAIN_WORKSPACE: root },
  });
  try {
    await client.connect(transport);
    const tools = await client.listTools();
    assert.ok(tools.tools.some((t) => t.name === "complete_job"));
    const call = async (name: string, args: Record<string, unknown> = {}) => {
      const result = await client.callTool({ name, arguments: args });
      assert.equal(result.isError, undefined, JSON.stringify(result.content));
      return JSON.parse((result.content as { text: string }[])[0].text);
    };
    const overview = await call("get_workspace_overview");
    assert.equal(overview.courses[0].id, course.id);
    assert.equal(overview.credentials, undefined);
    assert.equal(overview.settings, undefined);
    assert.equal(overview.NOTION_TOKEN, undefined);
    for (const name of [
      "configure_connections",
      "sync_sources",
      "list_tasks",
      "create_local_task",
      "set_local_task_done",
      "map_task_to_course",
    ]) {
      assert.ok(!tools.tools.some((tool) => tool.name === name));
    }
    await call("update_course", {
      courseId: course.id,
      changes: {
        canvasUrl: "https://canvas.example/courses/123",
        notionUrl: "https://www.notion.so/my-course",
      },
    });
    await call("update_course", {
      courseId: course.id,
      changes: { term: "Fall" },
    });
    assert.equal(
      (await readState()).courses[0].notionUrl,
      "https://www.notion.so/my-course",
    );
    const workflow = await call("get_task_workflow", {
      courseId: course.id,
      intent: "check",
      request: "What is due?",
    });
    assert.equal(workflow.status, "browser_action_required");
    assert.equal(workflow.sources.canvas, "https://canvas.example/courses/123");
    assert.equal("notion" in workflow.sources, false);
    assert.match(
      workflow.instructions,
      /Legacy update_status intent does not authorize external writes/,
    );
    assert.equal(workflow.intent, "check");
    const update = await call("get_task_workflow", {
      courseId: course.id,
      intent: "update_status",
      request: "Mark lab 1 complete in Notion",
    });
    assert.equal(update.intent, "update_status");
    const missing = await client.callTool({
      name: "get_task_workflow",
      arguments: { courseId: "missing", request: "Check assignments" },
    });
    assert.equal(missing.isError, true);
    const catalog = await call("get_course", { courseId: course.id });
    const assignment = await call("create_assignment", {
      courseId: course.id,
      title: "Binary search exercise",
    });
    const assignmentScope = {
      courseId: course.id,
      assignmentId: assignment.id,
    };
    const draft = await call("create_assignment_file", {
      ...assignmentScope,
      path: "answer.py",
      content: "def search():\n    return None\n",
      explanation:
        "Created a source-code draft for reasoning about binary search.",
    });
    const file = await call("read_assignment_file", {
      ...assignmentScope,
      path: "answer.py",
    });
    assert.equal(file.revision, draft.revision);
    await call("edit_assignment_file", {
      ...assignmentScope,
      path: "answer.py",
      expectedRevision: file.revision,
      edits: [{ oldText: "return None", newText: "return -1" }],
      explanation:
        "Use an explicit sentinel for the not-found case; this draft has not run.",
    });
    await call("record_assignment_learning", {
      ...assignmentScope,
      title: "Search sentinel",
      markdown:
        "The draft uses -1 as a not-found sentinel. This is generated reasoning and has not been executed.",
      paths: ["answer.py"],
      sourceIds: [],
      gaps: ["Implementation is only a draft; no tests run."],
    });
    assert.ok(
      (
        await call("search_course", {
          courseId: course.id,
          query: "search sentinel",
        })
      ).some((s: { kind: string }) => s.kind === "assignment"),
    );
    const packet = await call("prepare_lecture_agent", {
      courseId: course.id,
      lectureId: catalog.lectures[0].id,
    });
    assert.equal(packet.spawnArguments.fork_turns, "none");
    await call("set_lecture_coverage", {
      courseId: course.id,
      lectureId: catalog.lectures[0].id,
      coverage: "Transcript fixture; video unavailable.",
    });
    const compact = await call("get_lecture_status", {
      courseId: course.id,
      lectureId: catalog.lectures[0].id,
    });
    assert.equal(compact.coverage, "Transcript fixture; video unavailable.");
    assert.equal(compact.cues, undefined);
    const lecturePage = await call("get_lecture", {
      courseId: course.id,
      lectureId: catalog.lectures[0].id,
      limit: 1,
    });
    assert.equal(lecturePage.cues.length, 1);
    assert.equal(lecturePage.nextOffset, 1);
    const other = await addCourse({ code: "OTHER", name: "Other course" });
    const crossCourse = await client.callTool({
      name: "get_lecture",
      arguments: { courseId: other.id, lectureId: catalog.lectures[0].id },
    });
    assert.equal(crossCourse.isError, true);
    await call("fail_job", { jobId: job.id, reason: "Temporary test failure" });
    await call("retry_job", { jobId: job.id });
    assert.ok(
      !tools.tools.some((t) =>
        /submit|export|execute|shell|upload_assignment/.test(t.name),
      ),
    );
    assert.ok(tools.tools.some((t) => t.name === "run_course_command"));
    const result = await client.callTool({
      name: "get_job",
      arguments: { jobId: job.id, limit: 1 },
    });
    assert.equal(result.isError, undefined);
    const data = JSON.parse((result.content as { text: string }[])[0].text);
    assert.equal(data.evidence.length, 1);
    assert.ok(data.outputSchema);
    const completed = await client.callTool({
      name: "complete_job",
      arguments: {
        jobId: job.id,
        output: {
          markdown: "Binary search halves a sorted interval.",
          citations: [data.evidence[0].id],
          gaps: [],
        },
      },
    });
    assert.equal(completed.isError, undefined);
    assert.equal((await readState()).jobs[0].status, "completed");
    const lectureJob = await call("queue_study_job", {
      courseId: course.id,
      lectureId: catalog.lectures[0].id,
      kind: "lecture",
      prompt: "Test deletable guide",
    });
    await call("complete_job", {
      jobId: lectureJob.id,
      output: {
        summary: "A guide available for deletion.",
        sections: [
          {
            title: "Introduction",
            markdown: "Binary search",
            citations: [data.evidence[0].id],
          },
        ],
        concepts: [],
        questions: [],
        logistics: [],
        gaps: [],
      },
    });
    const deleted = await call("delete_lecture_guide", {
      courseId: course.id,
      lectureId: catalog.lectures[0].id,
    });
    assert.equal(deleted.deleted, true);
    assert.equal(
      (
        await call("get_lecture_status", {
          courseId: course.id,
          lectureId: catalog.lectures[0].id,
        })
      ).hasGuide,
      false,
    );
    const discovery = await call("prepare_exam", {
      courseId: course.id,
      prompt: "Make a practice exam for lecture 1",
    });
    assert.equal(discovery.status, "discovery_required");
    const examInput = {
      courseId: course.id,
      prompt: "Make a practice exam for lecture 1",
      questionCount: 1,
      scope: { mode: "lectures", lectureIds: [catalog.lectures[0].id] },
    };
    const preview = await call("preview_exam_scope", examInput);
    assert.equal(preview.lectures[0].id, catalog.lectures[0].id);
    const examJob = await call("queue_exam", examInput);
    assert.equal(examJob.examRequest.contentEvidenceIds, undefined);
    const examPage = await call("get_job", { jobId: examJob.id, limit: 1 });
    assert.ok(examPage.outputSchema.properties.questions);
    assert.equal(examPage.contentEvidenceIds.length, 1);
    const outsider = await call("import_lecture", {
      courseId: course.id,
      title: "Unselected lecture",
      date: "2027-01-01",
      transcript:
        "WEBVTT\n\n00:00:00.000 --> 00:00:10.000\nUnselected quasar cosmology topic.",
    });
    const extension = await call("extend_job_evidence", {
      jobId: examJob.id,
      query: "quasar cosmology",
    });
    assert.equal(extension.added.length, 0);
    await call("complete_job", {
      jobId: examJob.id,
      output: {
        title: "Binary search practice",
        instructions: "Explain your reasoning.",
        format: {
          rationale: "Proposed short answer practice.",
          inferred: true,
          citations: [],
        },
        questions: [
          {
            type: "short_answer",
            topic: "Search",
            prompt: "Why must the interval be sorted?",
            points: 2,
            answer: "Order determines the retained half.",
            explanation: "Comparison excludes one ordered half.",
            rubric: [{ criterion: "Explain ordering", points: 2 }],
            citations: examPage.contentEvidenceIds,
          },
        ],
        gaps: [],
      },
    });
    const examResult = await call("get_job", { jobId: examJob.id });
    assert.equal(examResult.job.exam.questions.length, 1);
    assert.ok(
      !examResult.evidence.some(
        (s: { lectureId?: string }) => s.lectureId === outsider.id,
      ),
    );
  } finally {
    await client.close();
  }
});
