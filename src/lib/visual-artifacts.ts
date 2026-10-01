import sharp from "sharp";
import {
  artifactInput,
  diagramSchema,
  readabilityInput,
  type Diagram,
} from "./visual-artifact-schema";
import { allEvidence } from "./memory";
import { mutate, newId, writePrivate } from "./store";
import type { CaptureArtifact } from "./schema";

const colors = {
  ink: ["#172f2b", "#f3f6f5"],
  muted: ["#53655d", "#f2f4f1"],
  green: ["#315e42", "#e7f2e9"],
  blue: ["#285c83", "#e9f3fa"],
  orange: ["#874813", "#fff0dd"],
  purple: ["#674784", "#f1eafa"],
  white: ["#53655d", "#ffffff"],
};
const escape = (value: string) =>
  value.replace(
    /[&<>"']/g,
    (c) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&apos;",
      })[c]!,
  );

// Render an allowlisted diagram language, not arbitrary HTML/JS/SVG supplied by an agent.
export function renderDiagram(input: Diagram, title: string) {
  const diagram = diagramSchema.parse(input);
  const shapes = diagram.elements
    .map((e) => {
      const [stroke, fill] = colors[e.tone];
      if (e.type === "rect")
        return `<rect x="${e.x}" y="${e.y}" width="${e.width}" height="${e.height}" rx="14" fill="${fill}" stroke="${stroke}" stroke-width="2"/>`;
      if (e.type === "ellipse")
        return `<ellipse cx="${e.cx}" cy="${e.cy}" rx="${e.rx}" ry="${e.ry}" fill="${fill}" stroke="${stroke}" stroke-width="2"/>`;
      if (e.type === "line")
        return `<line x1="${e.x1}" y1="${e.y1}" x2="${e.x2}" y2="${e.y2}" stroke="${stroke}" stroke-width="3"${e.dashed ? ' stroke-dasharray="9 7"' : ""}${e.arrow ? ` marker-end="url(#arrow-${e.tone})"` : ""}/>`;
      return `<text fill="${stroke}" font-family="${e.mono ? "DejaVu Sans Mono, monospace" : "DejaVu Sans, sans-serif"}" font-size="${e.size}" font-weight="${e.bold ? 700 : 400}" text-anchor="${e.anchor}">${e.lines.map((line, i) => `<tspan x="${e.x}" y="${e.y + i * e.size * 1.4}">${escape(line)}</tspan>`).join("")}</text>`;
    })
    .join("\n");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${diagram.width}" height="${diagram.height}" viewBox="0 0 ${diagram.width} ${diagram.height}" role="img"><title>${escape(title)} — reconstruction from course evidence</title><defs>${Object.entries(
    colors,
  )
    .map(
      ([tone, color]) =>
        `<marker id="arrow-${tone}" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto" markerUnits="strokeWidth"><path d="M0,0 L0,6 L8,3 z" fill="${color[0]}"/></marker>`,
    )
    .join(
      "",
    )}</defs><rect width="100%" height="100%" fill="#ffffff"/>${shapes}</svg>`;
}

export async function saveCaptureArtifact(
  input: unknown,
): Promise<CaptureArtifact> {
  const data = artifactInput.parse(input);
  if (Boolean(data.diagram) === Boolean(data.image))
    throw new Error("Supply exactly one diagram or image");
  let image: Buffer | undefined;
  if (data.image) {
    if (!/^data:image\/(png|jpeg|webp);base64,/.test(data.image))
      throw new Error("Use a PNG, JPEG or WebP reconstruction");
    image = await sharp(Buffer.from(data.image.split(",")[1], "base64"), {
      limitInputPixels: 20_000_000,
    })
      .resize({ width: 2400, withoutEnlargement: true })
      .png()
      .toBuffer();
  }
  return mutate(async (state) => {
    const lecture = state.lectures.find((l) => l.id === data.lectureId);
    const capture = lecture?.captures.find((c) => c.id === data.captureId);
    if (!lecture || !capture)
      throw new Error("Capture not found in this lecture");
    if ((capture.artifacts?.length || 0) >= 10)
      throw new Error("At most ten reconstructions per capture");
    const available = await allEvidence(lecture.courseId, state);
    const sources = [...new Set(data.sourceIds)].map((id) => {
      const source = available.find((s) => s.id === id);
      if (!source || source.kind === "assignment")
        throw new Error(`Invalid reconstruction source: ${id}`);
      return source;
    });
    if (!sources.some((s) => s.kind === "transcript" || s.kind === "note"))
      throw new Error(
        "A reconstruction needs original transcript or course-note support",
      );
    const id = newId();
    const base = `courses/${lecture.courseId}/lectures/${lecture.id}/reconstructions/${capture.id}/${id}`;
    const artifact: CaptureArtifact = {
      id,
      title: data.title,
      description: data.description,
      createdAt: new Date().toISOString(),
      format: data.diagram ? "diagram" : "image",
      file: `${base}.${data.diagram ? "svg" : "png"}`,
      sourceIds: sources.map((s) => s.id),
      uncertainties: data.uncertainties,
      ...(data.diagram ? { definitionPath: `${base}.json` } : {}),
    };
    await writePrivate(
      artifact.file,
      data.diagram ? renderDiagram(data.diagram, data.title) : image!,
    );
    if (data.diagram)
      await writePrivate(
        artifact.definitionPath!,
        JSON.stringify(data.diagram, null, 2),
      );
    await writePrivate(
      `${base}.provenance.json`,
      JSON.stringify(
        {
          ...artifact,
          lectureId: lecture.id,
          captureId: capture.id,
          originalPath: capture.file,
          seconds: capture.seconds,
          sources,
          status:
            "Generated interpretation; not an original lecture capture or recovered transcription",
        },
        null,
        2,
      ),
    );
    capture.artifacts ||= [];
    capture.artifacts.push(artifact);
    return artifact;
  });
}

export async function reviewCaptureReadability(input: unknown) {
  const data = readabilityInput.parse(input);
  return mutate(async (state) => {
    const lecture = state.lectures.find((l) => l.id === data.lectureId);
    if (!lecture) throw new Error("Lecture not found");
    if (
      new Set(data.reviews.map((r) => r.captureId)).size !== data.reviews.length
    )
      throw new Error("Duplicate capture reviews");
    const captures = data.reviews.map((r) => {
      const capture = lecture.captures.find((c) => c.id === r.captureId);
      if (!capture)
        throw new Error("Reviewed capture not found in this lecture");
      return capture;
    });
    const reviewedAt = new Date().toISOString();
    data.reviews.forEach(({ status, reason }, i) => {
      captures[i].readability = { status, reason, reviewedAt };
    });
    await writePrivate(
      `courses/${lecture.courseId}/lectures/${lecture.id}/capture-readability.json`,
      JSON.stringify(
        lecture.captures.map((c) => ({
          captureId: c.id,
          review: c.readability || null,
        })),
        null,
        2,
      ),
    );
    return { lectureId: lecture.id, reviewed: data.reviews.length };
  });
}
