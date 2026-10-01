import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import path from "node:path";
import os from "node:os";
import sharp from "sharp";
import { NextRequest } from "next/server";
import { addCourse, readState, readPrivate, safePath } from "../src/lib/store";
import { importLecture, addCapture } from "../src/lib/lectures";
import { allEvidence } from "../src/lib/memory";
import { captureSlides } from "../src/lib/capture-slides";
import { diagramSchema } from "../src/lib/visual-artifact-schema";
import {
  renderDiagram,
  saveCaptureArtifact,
  reviewCaptureReadability,
} from "../src/lib/visual-artifacts";
import { GET } from "../src/app/api/capture/[id]/artifacts/[artifactId]/route";
import { readCourseContext } from "../src/lib/course-context";

let root: string;
const previous = process.env.COURSE_CAPTAIN_WORKSPACE;
beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "captain-visual-"));
  process.env.COURSE_CAPTAIN_WORKSPACE = root;
});
afterEach(async () => {
  if (previous) process.env.COURSE_CAPTAIN_WORKSPACE = previous;
  else delete process.env.COURSE_CAPTAIN_WORKSPACE;
  assert.ok(root.startsWith(path.join(os.tmpdir(), "captain-visual-")));
  await fs.rm(root, { recursive: true, force: true });
});
const diagram = {
  width: 1000,
  height: 500,
  elements: [
    { type: "rect", x: 40, y: 120, width: 920, height: 300, tone: "green" },
    {
      type: "text",
      x: 80,
      y: 180,
      lines: ["RAM holds the running program."],
      size: 30,
    },
  ],
};
async function fixture() {
  const course = await addCourse({ code: "OS", name: "Operating systems" });
  const lecture = await importLecture({
    courseId: course.id,
    title: "Boot",
    date: "2026-09-27",
    transcript:
      "WEBVTT\n\n00:00:01.000 --> 00:00:20.000\nRAM holds the running program.",
  });
  const png = await sharp({
    create: { width: 50, height: 50, channels: 3, background: "gray" },
  })
    .png()
    .toBuffer();
  const capture = await addCapture({
    lectureId: lecture.id,
    seconds: 5,
    kind: "whiteboard",
    stream: "Board",
    image: `data:image/png;base64,${png.toString("base64")}`,
  });
  const input = {
    lectureId: lecture.id,
    captureId: capture.id,
    title: "Boot concept",
    description: "Reconstructed from the lecture explanation.",
    sourceIds: [`${lecture.id}:t0`],
    uncertainties: ["Exact board labels cannot be read."],
    diagram,
  };
  return { course, lecture, capture, input };
}
test("reconstructions preserve original evidence and appear directly after it", async () => {
  const { course, lecture, capture, input } = await fixture();
  const original = await fs.readFile(safePath(capture.file));
  const evidenceBefore = await allEvidence(course.id);
  const artifact = await saveCaptureArtifact(input);
  await reviewCaptureReadability({
    lectureId: lecture.id,
    reviews: [
      {
        captureId: capture.id,
        status: "unclear",
        reason: "Small blurry board text",
      },
    ],
  });
  assert.deepEqual(await fs.readFile(safePath(capture.file)), original);
  assert.deepEqual(await allEvidence(course.id), evidenceBefore);
  const saved = (await readState()).lectures[0].captures[0];
  assert.equal(saved.readability?.status, "unclear");
  assert.deepEqual(
    captureSlides([saved]).map((s) => s.id),
    [capture.id, `${capture.id}:artifact:${artifact.id}`],
  );
  assert.match(await readPrivate(artifact.file), /<svg/);
  assert.equal(
    JSON.parse(await readPrivate(artifact.definitionPath!)).width,
    1000,
  );
  const context = await readCourseContext({
    courseId: course.id,
    sourceId: `${lecture.id}:c:${capture.id}`,
  });
  if (context.kind === "source") {
    assert.equal(context.reconstructions.length, 1);
    assert.match(context.reconstructions[0].status, /not original/);
  } else assert.fail("Expected a capture source");
});
test("reconstruction tools require real scoped support and inert diagram data", async () => {
  const { capture, input } = await fixture();
  const course2 = await addCourse({ code: "OTHER", name: "Other course" });
  const lecture2 = await importLecture({
    courseId: course2.id,
    title: "Other",
    date: "2026-09-27",
    transcript:
      "WEBVTT\n\n00:00:01.000 --> 00:00:20.000\nUnrelated evidence here.",
  });
  for (const sourceIds of [
    ["invented"],
    [`${lecture2.id}:t0`],
    [`${input.lectureId}:c:${capture.id}`],
  ])
    await assert.rejects(
      saveCaptureArtifact({ ...input, sourceIds }),
      /source|support/,
    );
  await assert.rejects(
    saveCaptureArtifact({ ...input, captureId: "wrong" }),
    /not found/,
  );
  await assert.rejects(saveCaptureArtifact({ ...input, uncertainties: [] }));
  await assert.rejects(
    saveCaptureArtifact({ ...input, diagram: undefined }),
    /exactly one/,
  );
  await assert.rejects(
    saveCaptureArtifact({ ...input, image: "not-an-image" }),
    /exactly one/,
  );
  await assert.rejects(
    saveCaptureArtifact({
      ...input,
      diagram: { ...diagram, script: "alert(1)" },
    }),
  );
  await assert.rejects(
    saveCaptureArtifact({
      ...input,
      diagram: {
        ...diagram,
        elements: [{ type: "rect", x: 990, y: 0, width: 100, height: 100 }],
      },
    }),
    /canvas/,
  );
  const escaped = renderDiagram(
    diagramSchema.parse({
      ...diagram,
      elements: [
        {
          type: "text",
          x: 40,
          y: 100,
          lines: ['<script>alert("test")</script> & <foreignObject>'],
        },
      ],
    }),
    "<script>title</script>",
  );
  assert.ok(!escaped.includes("<script>"));
  assert.match(escaped, /&lt;script&gt;/);
  await assert.rejects(
    reviewCaptureReadability({
      lectureId: input.lectureId,
      reviews: [
        { captureId: "wrong", status: "unclear", reason: "Cannot read it" },
      ],
    }),
    /not found/,
  );
  assert.equal(
    (await readState()).lectures[0].captures[0].artifacts,
    undefined,
  );
});
test("image artifacts normalize to PNG and artifact routes cannot read arbitrary files", async () => {
  const { input } = await fixture();
  const png = await sharp({
    create: { width: 32, height: 32, channels: 3, background: "blue" },
  })
    .jpeg()
    .toBuffer();
  const artifact = await saveCaptureArtifact({
    ...input,
    diagram: undefined,
    image: `data:image/jpeg;base64,${png.toString("base64")}`,
  });
  assert.equal(artifact.format, "image");
  assert.equal((await sharp(safePath(artifact.file)).metadata()).format, "png");
  const req = new NextRequest(
    `http://localhost:3000/api/capture/${input.captureId}/artifacts/${artifact.id}`,
    { headers: { host: "localhost:3000" } },
  );
  const response = await GET(req, {
    params: Promise.resolve({ id: input.captureId, artifactId: artifact.id }),
  });
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "image/png");
  assert.match(response.headers.get("content-security-policy")!, /sandbox/);
  const denied = await GET(req, {
    params: Promise.resolve({
      id: input.captureId,
      artifactId: "../../.env.local",
    }),
  });
  assert.equal(denied.status, 404);
});
