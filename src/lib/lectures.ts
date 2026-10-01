import { z } from "zod";
import sharp from "sharp";
import { lectureInput, type Lecture, type Capture } from "./schema";
import { mutate, newId, requireCourse, writePrivate } from "./store";
import { parseTranscript, timestamp } from "./transcript";

export async function importLecture(input: unknown): Promise<Lecture> {
  const data = lectureInput.parse(input);
  const cues = parseTranscript(data.transcript);
  return mutate(async (state) => {
    requireCourse(state, data.courseId);
    const id = newId();
    const base = `courses/${data.courseId}/lectures/${id}`;
    const lecture: Lecture = {
      id,
      courseId: data.courseId,
      title: data.title,
      date: data.date,
      sourceUrl: data.sourceUrl,
      createdAt: new Date().toISOString(),
      cues,
      captures: [],
      duration: Math.max(...cues.map((c) => c.end)),
      status: "imported",
      notesPath: `${base}/notes.md`,
      transcriptPath: `${base}/transcript.md`,
      captureCoverage: data.captureCoverage,
      reviewed: false,
    };
    await writePrivate(`${base}/source.vtt`, data.transcript);
    await writePrivate(
      lecture.transcriptPath,
      `# ${data.title}\n\nSource: ${data.sourceUrl || "Uploaded transcript"}\n\n` +
        cues
          .map((c, i) => `## ${timestamp(c.start)} [${id}:t${i}]\n\n${c.text}`)
          .join("\n\n"),
    );
    await writePrivate(
      lecture.notesPath,
      `# ${data.title}\n\nEvidence imported; the complete lecture guide is not complete. Continue visual capture and inspection, then create and complete a lecture job.\n\nCapture coverage: ${data.captureCoverage}\n`,
    );
    state.lectures.push(lecture);
    return lecture;
  });
}
export const captureInput = z.object({
  lectureId: z.string(),
  seconds: z.number().min(0),
  kind: z.enum(["slide", "whiteboard", "demo"]),
  stream: z.string().min(1).max(120),
  caption: z.string().max(3000).default(""),
  image: z.string().max(16_000_000),
});
export async function addCapture(input: unknown): Promise<Capture> {
  const data = captureInput.parse(input);
  if (!/^data:image\/(png|jpeg|webp);base64,/.test(data.image))
    throw new Error("Only PNG, JPEG, and WebP captures are accepted");
  const raw = Buffer.from(data.image.split(",")[1], "base64");
  const png = await sharp(raw, { limitInputPixels: 20_000_000 })
    .resize({ width: 1920, withoutEnlargement: true })
    .png()
    .toBuffer();
  return mutate(async (state) => {
    const lecture = state.lectures.find((l) => l.id === data.lectureId);
    if (!lecture) throw new Error("Lecture not found");
    if (data.seconds > lecture.duration + 60)
      throw new Error("Capture timestamp is outside the lecture");
    if (lecture.captures.length >= 1000)
      throw new Error("Maximum of 1,000 captures per lecture");
    const id = newId();
    const capture: Capture = {
      id,
      seconds: data.seconds,
      kind: data.kind,
      stream: data.stream,
      caption: data.caption,
      file: `courses/${lecture.courseId}/lectures/${lecture.id}/captures/${id}.png`,
    };
    await writePrivate(capture.file, png);
    lecture.captures.push(capture);
    lecture.captures.sort((a, b) => a.seconds - b.seconds);
    return capture;
  });
}
