import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { NextRequest } from "next/server";
import { addCourse, readPrivate, writePrivate, mutate } from "../src/lib/store";
import { importLecture } from "../src/lib/lectures";
import { saveSource } from "../src/lib/sources";
import { searchMemory } from "../src/lib/memory";
import { readCourseContext } from "../src/lib/course-context";
import { GET } from "../src/app/api/context/route";
import { localRequest } from "../src/lib/http";

let root: string;
const previous = process.env.COURSE_CAPTAIN_WORKSPACE;
beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "captain-context-"));
  process.env.COURSE_CAPTAIN_WORKSPACE = root;
});
afterEach(async () => {
  if (previous) process.env.COURSE_CAPTAIN_WORKSPACE = previous;
  else delete process.env.COURSE_CAPTAIN_WORKSPACE;
  assert.ok(root.startsWith(path.join(os.tmpdir(), "captain-context-")));
  await fs.rm(root, { recursive: true, force: true });
});
async function fixture() {
  const course = await addCourse({ code: "BIO", name: "Biology" });
  const other = await addCourse({ code: "CHEM", name: "Chemistry" });
  const lecture = await importLecture({
    courseId: course.id,
    title: "Evolution",
    date: "2026-09-27",
    transcript:
      "WEBVTT\n\n00:00:01.000 --> 00:00:15.000\nSelection changes populations across generations.",
  });
  const note = await saveSource({
    courseId: course.id,
    title: "Selection notes",
    text: "Selection acts on heritable variation within populations.",
  });
  await saveSource({
    courseId: other.id,
    title: "Selection in chemistry",
    text: "Other course content must not appear in Biology evidence.",
  });
  return { course, other, lecture, note };
}

test("browser context shares desktop RAG and reads full Markdown without mutating the workspace", async () => {
  const { course, note } = await fixture();
  const before = await readPrivate("state.json");
  const expected = await searchMemory(course.id, "selection");
  const result = await readCourseContext({
    courseId: course.id,
    query: "selection",
  });
  assert.equal(result.kind, "search");
  if (result.kind !== "search") return;
  assert.deepEqual(
    result.sources.slice(0, 8).map((s) => s.id),
    expected.map((s) => s.id),
  );
  assert.ok(result.sources.every((s) => s.courseId === course.id));
  const source = result.sources.find((s) => s.path === note.path)!;
  const read = await readCourseContext({
    courseId: course.id,
    sourceId: source.id,
    offset: 0,
  });
  assert.equal(read.kind, "source");
  if (read.kind === "source")
    assert.equal(read.documentText, await readPrivate(note.path));
  assert.equal(await readPrivate("state.json"), before);
});

test("context rejects cross-course IDs, arbitrary paths and ambiguous requests", async () => {
  const { course, other, lecture } = await fixture();
  await assert.rejects(
    readCourseContext({ courseId: other.id, lectureId: lecture.id }),
    /not found/,
  );
  await assert.rejects(
    readCourseContext({ courseId: other.id, sourceId: `${lecture.id}:t0` }),
    /not found/,
  );
  for (const sourceId of [
    "../../.env.local",
    "state.json",
    "memory:../../.env.local:0",
  ])
    await assert.rejects(
      readCourseContext({ courseId: course.id, sourceId }),
      /not found/,
    );
  await assert.rejects(readCourseContext({ courseId: "../" }));
  await assert.rejects(
    readCourseContext({ courseId: course.id, path: ".env.local" }),
  );
  await assert.rejects(
    readCourseContext({ query: "selection" }),
    /Select a course/,
  );
  await assert.rejects(
    readCourseContext({
      courseId: course.id,
      lectureId: lecture.id,
      query: "selection",
    }),
    /Choose/,
  );
  await assert.rejects(readCourseContext({ courseId: course.id, offset: -1 }));
  const badLink = `courses/${course.id}/memory/escape.md`;
  await fs.symlink(path.join(root, "state.json"), path.join(root, badLink));
  await assert.rejects(
    readCourseContext({ courseId: course.id, sourceId: `memory:${badLink}:0` }),
    /not found/,
  );
});

test("long files and lecture sources are paginated; hidden lectures remain searchable evidence", async () => {
  const { course, lecture, note } = await fixture();
  const fullText =
    "# Long memory\n\n" +
    "Selection and inheritance in populations. ".repeat(1100);
  await writePrivate(note.path, fullText);
  const evidence = await searchMemory(course.id, "inheritance");
  let offset: number | null = 0;
  let reconstructed = "";
  while (offset !== null) {
    const result = await readCourseContext({
      courseId: course.id,
      sourceId: evidence[0].id,
      offset,
    });
    assert.equal(result.kind, "source");
    if (result.kind !== "source") break;
    assert.ok(result.documentText.length <= 12000);
    reconstructed += result.documentText;
    offset = result.nextOffset;
  }
  assert.equal(reconstructed, fullText);
  await mutate((state) => {
    const saved = state.lectures.find((l) => l.id === lecture.id)!;
    saved.hiddenFromUi = true;
    saved.captures = Array.from({ length: 25 }, (_, i) => ({
      id: `capture${i}`,
      seconds: i,
      kind: "slide",
      stream: "Slides",
      file: "unused.png",
      caption: `Slide ${i}`,
    }));
  });
  const index = await readCourseContext({ courseId: course.id });
  if (index.kind === "course")
    assert.ok(index.items.every((i) => i.title !== lecture.title));
  const read = await readCourseContext({
    courseId: course.id,
    sourceId: `${lecture.id}:t0`,
  });
  if (read.kind === "source")
    assert.match(read.documentText, /00:01.*Selection/);
  const sources = await readCourseContext({
    courseId: course.id,
    lectureId: lecture.id,
    section: "sources",
  });
  assert.equal(sources.kind, "lecture");
  if (sources.kind === "lecture") {
    assert.equal(sources.generatedGuide, null);
    assert.equal(sources.sources.length, 12);
    assert.equal(sources.totalSources, 26);
    assert.equal(sources.nextOffset, 12);
    assert.equal(sources.sources[1].imageUrl, "/api/capture/capture0");
  }
});

test("HTTP evidence route allows loopback reads and denies foreign origins", async () => {
  const { course } = await fixture();
  const url = `http://localhost:3000/api/context?courseId=${course.id}`;
  const read = await GET(
    new NextRequest(url, { headers: { host: "localhost:3000" } }),
  );
  assert.equal(read.status, 200);
  assert.equal(read.headers.get("cache-control"), "no-store");
  const foreignHeaders: Record<string, string>[] = [
    { host: "evil.example" },
    { host: "localhost:3000", origin: "https://evil.example" },
    { host: "localhost:3000", "sec-fetch-site": "cross-site" },
  ];
  for (const headers of foreignHeaders) {
    const response = await GET(new NextRequest(url, { headers }));
    assert.equal(response.status, 400);
    assert.equal(response.headers.get("access-control-allow-origin"), null);
  }
  const navigation = new NextRequest("http://localhost:3000/context", {
    headers: {
      host: "localhost:3000",
      "sec-fetch-site": "cross-site",
      "sec-fetch-mode": "navigate",
      "sec-fetch-dest": "document",
    },
  });
  assert.doesNotThrow(() =>
    localRequest(navigation, false, { allowDocumentNavigation: true }),
  );
  assert.throws(() => localRequest(navigation), /Cross-site/);
  assert.throws(
    () => localRequest(navigation, true, { allowDocumentNavigation: true }),
    /same-origin/,
  );
  const frame = new NextRequest("http://localhost:3000/context", {
    headers: {
      host: "localhost:3000",
      "sec-fetch-site": "cross-site",
      "sec-fetch-mode": "navigate",
      "sec-fetch-dest": "iframe",
    },
  });
  assert.throws(
    () => localRequest(frame, false, { allowDocumentNavigation: true }),
    /Cross-site/,
  );
});
