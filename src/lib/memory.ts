import { promises as fs } from "node:fs";
import path from "node:path";
import {
  assertNoSymlink,
  readPrivate,
  readState,
  requireCourse,
  safePath,
} from "./store";
import type { Citation, Lecture, State } from "./schema";
import { indexCourseFiles } from "./course-file-index";

export function lectureEvidence(lecture: Lecture): Citation[] {
  // Stable IDs refer to raw cues, not generated summaries.
  return lecture.cues.map((cue, i) => ({
    id: `${lecture.id}:t${i}`,
    courseId: lecture.courseId,
    lectureId: lecture.id,
    title: lecture.title,
    seconds: cue.start,
    text: cue.text,
    path: lecture.transcriptPath,
    url: lecture.sourceUrl,
    kind: "transcript" as const,
  }));
}
export const captureEvidence = (lecture: Lecture): Citation[] =>
  lecture.captures.map((c) => ({
    id: `${lecture.id}:c:${c.id}`,
    courseId: lecture.courseId,
    lectureId: lecture.id,
    title: `${lecture.title} · ${c.stream} · ${c.kind}`,
    seconds: c.seconds,
    text:
      c.caption ||
      `Uncaptioned ${c.kind}. Inspect the image before describing it.`,
    path: c.file,
    kind: "capture",
  }));
const words = (text: string): string[] =>
  text.toLowerCase().match(/[\p{L}\p{N}]{2,}/gu) || [];
const stopWords = new Set(
  "the a an is are was were what how why can could do does of to in it this that and or for with from lecture explain me my about".split(
    " ",
  ),
);

async function markdownEvidence(courseId: string): Promise<Citation[]> {
  const base = `courses/${courseId}/memory`;
  const result: Citation[] = [];
  async function visit(relative: string, depth: number) {
    if (depth > 4) return;
    const target = safePath(relative);
    await assertNoSymlink(target);
    for (const entry of await fs.readdir(target, { withFileTypes: true })) {
      if (entry.isSymbolicLink() || entry.name.startsWith(".")) continue;
      const file = `${relative}/${entry.name}`;
      if (entry.isDirectory()) await visit(file, depth + 1);
      else if (entry.name.endsWith(".md")) {
        const stat = await fs.stat(safePath(file));
        if (stat.size > 2_000_000) continue;
        const content = await readPrivate(file);
        const title = content.match(/^# (.+)/)?.[1] || path.basename(file);
        const url = content.match(/^Source: (https:\/\/\S+)/m)?.[1];
        content
          .split(/\n(?=#{1,3} )|\n\s*\n/)
          .filter((t) => t.trim().length > 30)
          .flatMap((text) => {
            const chunks: string[] = [];
            for (let start = 0; start < text.length; start += 5500)
              chunks.push(text.slice(start, start + 6000));
            return chunks;
          })
          .forEach((text, i) => {
            result.push({
              id: `memory:${file}:${i}`,
              courseId,
              title,
              text:
                file.startsWith(`courses/${courseId}/memory/assignments/`) ||
                file.startsWith(`courses/${courseId}/memory/files/`)
                  ? `Generated assignment learning/work history; not primary instructor evidence.\n${text}`
                  : text,
              path: file,
              url,
              kind:
                file.startsWith(`courses/${courseId}/memory/assignments/`) ||
                file.startsWith(`courses/${courseId}/memory/files/`)
                  ? "assignment"
                  : "note",
            });
          });
      }
    }
  }
  try {
    await visit(base, 0);
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code !== "ENOENT") throw e;
  }
  return result;
}
export async function allEvidence(
  courseId: string,
  state?: State,
): Promise<Citation[]> {
  state ||= await readState();
  requireCourse(state, courseId);
  return [
    ...state.lectures
      .filter((l) => l.courseId === courseId)
      .flatMap((l) => [...lectureEvidence(l), ...captureEvidence(l)]),
    ...state.tasks
      .filter((t) => t.courseId === courseId && !t.archived)
      .map((t) => ({
        id: `task:${t.id}`,
        courseId,
        title: t.title,
        text: `Historical task snapshot — not current requirements or status. Recheck official Canvas/course websites; use workspace checkpoints for local progress.\n${t.title}\n${t.description}\nPreviously recorded due date: ${t.due || "not specified"}`,
        path: "state.json",
        url: t.url,
        kind: "assignment" as const,
      })),
    ...(await markdownEvidence(courseId)),
    ...(await indexCourseFiles(courseId)).evidence,
  ];
}
export function rankEvidence(
  query: string,
  sources: Citation[],
  limit = 8,
): Citation[] {
  const terms = [...new Set(words(query).filter((w) => !stopWords.has(w)))];
  if (!terms.length || !sources.length) return [];
  const docs = sources.map((s) => words(`${s.title} ${s.text}`));
  const average = docs.reduce((sum, doc) => sum + doc.length, 0) / docs.length;
  const frequency = new Map(
    terms.map((term) => [
      term,
      docs.filter((doc) => doc.includes(term)).length,
    ]),
  );
  return sources
    .map((source, i) => {
      const doc = docs[i];
      const score = terms.reduce((total, term) => {
        const tf = doc.filter((w) => w === term).length;
        const idf = Math.log(
          1 +
            (docs.length - (frequency.get(term) || 0) + 0.5) /
              ((frequency.get(term) || 0) + 0.5),
        );
        return (
          total +
          idf *
            ((tf * 2.2) / (tf + 1.2 * (0.25 + (0.75 * doc.length) / average)))
        );
      }, 0);
      return { source, score };
    })
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, limit)
    .map((x) => x.source);
}
export async function searchMemory(courseId: string, query: string, limit = 8) {
  return rankEvidence(query, await allEvidence(courseId), limit);
}
