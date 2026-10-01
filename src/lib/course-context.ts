import { z } from "zod";
import { allEvidence, rankEvidence } from "./memory";
import { readPrivate, readState, requireCourse } from "./store";
import { idSchema, type Citation } from "./schema";
import { timestamp } from "./transcript";
import {
  courseContextInstructions,
  courseContextUrl,
} from "./context-contract";

const pageSize = 12;
const textSize = 12000;
export const contextInput = z
  .object({
    courseId: idSchema.optional(),
    lectureId: idSchema.optional(),
    sourceId: z.string().min(1).max(1000).optional(),
    section: z.enum(["guide", "sources"]).optional(),
    query: z.string().trim().max(1000).default(""),
    offset: z.coerce.number().int().min(0).max(10_000_000).optional(),
  })
  .strict()
  .superRefine((data, ctx) => {
    if (!data.courseId && (data.lectureId || data.sourceId || data.query))
      ctx.addIssue({ code: "custom", message: "Select a course first" });
    if (data.section && !data.lectureId)
      ctx.addIssue({ code: "custom", message: "Sections require a lecture" });
    if (
      [!!data.lectureId, !!data.sourceId, !!data.query].filter(Boolean).length >
      1
    )
      ctx.addIssue({
        code: "custom",
        message: "Choose a lecture, source, or search query",
      });
  });

export type BrowserEvidence = Citation & {
  contextUrl: string;
  imageUrl?: string;
};
const reference = (source: Citation): BrowserEvidence => ({
  ...source,
  contextUrl: courseContextUrl({
    courseId: source.courseId,
    sourceId: source.id,
  }),
  ...(source.kind === "capture"
    ? {
        imageUrl: `/api/capture/${encodeURIComponent(source.id.split(":c:")[1])}`,
      }
    : {}),
});

export async function readCourseContext(input: unknown) {
  const data = contextInput.parse(input);
  const state = await readState();
  const offset = data.offset ?? 0;
  const base = {
    instructions: courseContextInstructions,
    readOnly: true as const,
  };
  if (!data.courseId) {
    return {
      ...base,
      kind: "workspace" as const,
      courses: state.courses.slice(offset, offset + pageSize).map((c) => ({
        id: c.id,
        code: c.code,
        name: c.name,
        term: c.term,
        contextUrl: courseContextUrl({ courseId: c.id }),
      })),
      nextOffset:
        offset + pageSize < state.courses.length ? offset + pageSize : null,
    };
  }
  const course = requireCourse(state, data.courseId);
  const courseBase = {
    ...base,
    course: {
      id: course.id,
      code: course.code,
      name: course.name,
      term: course.term,
      canvasUrl: course.canvasUrl,
      websiteUrl: course.websiteUrl,
      notionUrl: course.notionUrl,
    },
  };
  const sources = await allEvidence(course.id, state);
  if (data.sourceId) {
    // Resolve only an existing citation in the selected course, never a supplied file path.
    const source = sources.find((s) => s.id === data.sourceId);
    if (!source) throw new Error("Source not found in this course");
    let documentText = source.text;
    if (
      source.kind === "note" ||
      (source.kind === "assignment" &&
        (source.path.startsWith(`courses/${course.id}/memory/assignments/`) ||
          source.path.startsWith(`courses/${course.id}/memory/files/`)))
    ) {
      if (
        !source.path.startsWith(`courses/${course.id}/memory/`) ||
        !source.path.endsWith(".md")
      )
        throw new Error("Source is not course Markdown memory");
      documentText = await readPrivate(source.path);
    } else if (source.kind === "file") {
      if (!source.path.startsWith(`courses/${course.id}/files/`))
        throw new Error("Not a course file");
      documentText = await readPrivate(source.path);
    } else if (source.kind === "transcript") {
      const lecture = state.lectures.find(
        (l) => l.id === source.lectureId && l.courseId === course.id,
      )!;
      documentText = lecture.cues
        .map(
          (cue, i) =>
            `[${lecture.id}:t${i}] ${timestamp(cue.start)}–${timestamp(cue.end)} ${cue.text}`,
        )
        .join("\n\n");
    }
    const start =
      data.offset ??
      Math.max(
        0,
        documentText.indexOf(
          source.kind === "transcript"
            ? `[${source.id}]`
            : source.text.replace(
                /^Generated assignment learning\/work history; not primary instructor evidence\.\n/,
                "",
              ),
        ),
      );
    const capture =
      source.kind === "capture"
        ? state.lectures
            .find((l) => l.id === source.lectureId && l.courseId === course.id)
            ?.captures.find(
              (c) => `${source.lectureId}:c:${c.id}` === source.id,
            )
        : undefined;
    return {
      ...courseBase,
      kind: "source" as const,
      source: reference(source),
      reconstructions: (capture?.artifacts || []).map((a) => ({
        id: a.id,
        title: a.title,
        description: a.description,
        format: a.format,
        imageUrl: `/api/capture/${capture!.id}/artifacts/${a.id}`,
        sources: a.sourceIds.map((id) => ({
          id,
          contextUrl: courseContextUrl({ courseId: course.id, sourceId: id }),
        })),
        uncertainties: a.uncertainties,
        status: "Generated interpretation, not original lecture evidence",
      })),
      documentText: documentText.slice(start, start + textSize),
      offset: start,
      totalCharacters: documentText.length,
      nextOffset:
        start + textSize < documentText.length ? start + textSize : null,
    };
  }
  if (data.lectureId) {
    const lecture = state.lectures.find(
      (l) => l.id === data.lectureId && l.courseId === course.id,
    );
    if (!lecture) throw new Error("Lecture not found in this course");
    // Generated guides are secondary context, kept distinct from original source evidence.
    const guideText = lecture.guide
      ? JSON.stringify(lecture.guide, null, 2)
      : "No generated guide is saved.";
    const guideIds = new Set(
      lecture.guide
        ? [
            ...lecture.guide.sections.flatMap((s) => s.citations),
            ...lecture.guide.concepts.flatMap((s) => s.citations),
            ...lecture.guide.logistics.flatMap((s) => s.citations),
            ...lecture.guide.questions.flatMap((s) => s.citations),
          ]
        : [],
    );
    const originalSources = sources.filter(
      (s) =>
        s.lectureId === lecture.id &&
        (s.kind === "capture" || s.id === `${lecture.id}:t0`),
    );
    const guideSources = sources.filter((s) => guideIds.has(s.id));
    const references = [
      ...new Map(
        [...originalSources, ...guideSources].map((s) => [s.id, s]),
      ).values(),
    ];
    const sourceOffset = data.section === "sources" ? offset : 0;
    return {
      ...courseBase,
      kind: "lecture" as const,
      lecture: {
        id: lecture.id,
        title: lecture.title,
        date: lecture.date,
        sourceUrl: lecture.sourceUrl,
        captureCoverage: lecture.captureCoverage,
        hiddenFromUi: !!lecture.hiddenFromUi,
      },
      generatedGuide:
        data.section === "sources"
          ? null
          : guideText.slice(offset, offset + textSize),
      offset,
      nextOffset:
        data.section === "sources"
          ? offset + pageSize < references.length
            ? offset + pageSize
            : null
          : offset + textSize < guideText.length
            ? offset + textSize
            : null,
      sources: references
        .slice(sourceOffset, sourceOffset + pageSize)
        .map((s) => ({ ...reference(s), text: s.text.slice(0, 500) })),
      totalSources: references.length,
      allSourcesUrl: courseContextUrl({
        courseId: course.id,
        lectureId: lecture.id,
        section: "sources",
      }),
    };
  }
  if (data.query) {
    // Identical ranking and evidence pool to searchMemory/search_course on desktop.
    const ranked = rankEvidence(data.query, sources, offset + pageSize + 1);
    return {
      ...courseBase,
      kind: "search" as const,
      query: data.query,
      sources: ranked.slice(offset, offset + pageSize).map((s) => ({
        ...reference(s),
        text: s.text.slice(0, 1500),
        textTruncated: s.text.length > 1500,
      })),
      nextOffset: ranked.length > offset + pageSize ? offset + pageSize : null,
    };
  }
  const files = [
    ...new Map(
      sources
        .filter(
          (s) =>
            s.kind === "note" ||
            (s.kind === "assignment" &&
              (s.path.startsWith(`courses/${course.id}/memory/assignments/`) ||
                s.path.startsWith(`courses/${course.id}/memory/files/`))),
        )
        .map((s) => [s.path, s]),
    ).values(),
  ];
  const items = [
    ...state.lectures
      .filter((l) => l.courseId === course.id && !l.hiddenFromUi)
      .map((l) => ({
        title: l.title,
        detail: `${l.date} · lecture`,
        contextUrl: courseContextUrl({ courseId: course.id, lectureId: l.id }),
      })),
    ...files.map((s) => ({
      title: s.title,
      detail: s.path,
      contextUrl: courseContextUrl({
        courseId: course.id,
        sourceId: s.id,
        offset: 0,
      }),
    })),
  ];
  return {
    ...courseBase,
    kind: "course" as const,
    items: items.slice(offset, offset + pageSize),
    totalItems: items.length,
    nextOffset: offset + pageSize < items.length ? offset + pageSize : null,
  };
}
