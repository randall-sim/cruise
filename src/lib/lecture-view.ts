import type { Capture, Guide, Lecture } from "./schema";
import { secondsFromTimestamp } from "./transcript";
import { isGuideConstructionNote } from "./markdown-citations";

export type GuideSection = Guide["sections"][number];
export type SectionCapture = Capture & { evidenceId: string };
export type GuidePage = GuideSection & {
  id: string;
  kind?: "checks";
  savedCaptureId?: string;
};

export function isCoveragePage(page: GuideSection): boolean {
  return (
    page.category === "coverage" ||
    /^(?:(?:\d{1,3}:\d{2}(?::\d{2})?\s*[–—-]\s*\d{1,3}:\d{2}(?::\d{2})?)\s*[·•:–—-]?\s*|\d+\s*[.·•:–—-]\s*)?(?:coverage\b|evidence coverage\b|resources and (?:coverage|gaps)\b)/i.test(
      page.title.trim(),
    )
  );
}

export function isAncillaryPage(page: GuideSection): boolean {
  if (page.category) return page.category !== "lecture";
  // Older guides have no category: recognize their explicit ancillary headings.
  return /^(?:(?:\d{1,3}:\d{2}(?::\d{2})?\s*[–—-]\s*\d{1,3}:\d{2}(?::\d{2})?)\s*[·•:–—-]?\s*|\d+\s*[.·•:–—-]\s*)?(?:supplementary\b|course logistics\b|logistics\b|lecture resources\b|resources\b|evidence coverage\b|coverage\b|prior[- ]lecture connections\b|connections to (?:earlier|prior|previous) lectures\b)/i.test(
    page.title.trim(),
  );
}

/** Preserve section anchors while presenting every part of a guide as a page. */
export function lecturePages(
  lecture: Lecture,
  lectures: Lecture[],
): GuidePage[] {
  const guide = lecture.guide;
  const pages: GuidePage[] = guide
    ? guide.sections
        .map((section, i) => ({ ...section, id: `section-${i}` }))
        .filter((page) => !isCoveragePage(page))
    : [
        {
          id: "section-0",
          title: "Lecture transcript",
          markdown: "The lecture guide has not been created yet.",
          citations: [],
          startSeconds: 0,
          endSeconds: Math.max(1, lecture.duration),
        },
      ];
  if (guide?.summary.trim())
    pages.unshift({
      id: "guide-overview",
      title: "Lecture Overview",
      markdown: guide.summary,
      citations: [],
    });
  const cited = new Set(
    pages.flatMap((page) =>
      sectionCaptures(page, lecture, lectures).map((capture) => capture.id),
    ),
  );
  for (const capture of lecture.captures.filter(
    (capture) => !cited.has(capture.id),
  )) {
    pages.push({
      id: `guide-capture-${capture.id}`,
      savedCaptureId: capture.id,
      title: `${capture.kind} · ${capture.caption || "Additional visual evidence"}`,
      markdown:
        capture.caption ||
        "This saved visual has no linked explanation in the current guide.",
      citations: [`${lecture.id}:c:${capture.id}`],
    });
  }
  if (!guide) return pages;
  if (guide.logistics.length)
    pages.push({
      id: "guide-logistics",
      title: "Course logistics",
      markdown: guide.logistics
        .map(
          (item) =>
            `${item.text}\n\n${item.citations.map((id) => `[${id}]`).join(" ")}`,
        )
        .join("\n\n"),
      citations: guide.logistics.flatMap((item) => item.citations),
    });
  const ancillary = pages.filter(isAncillaryPage);
  const teaching = pages.filter((page) => !isAncillaryPage(page));
  pages.splice(0, pages.length, ...teaching);
  if (guide.questions.length)
    pages.push({
      id: "guide-checks",
      kind: "checks",
      title: "Check your understanding",
      markdown: "",
      citations: [],
    });
  pages.push(...ancillary);
  return pages;
}

export function lectureTargetPage(
  pages: GuidePage[],
  lecture: Lecture,
  lectures: Lecture[],
  target: string | null,
): number {
  if (target?.startsWith("capture-"))
    return pages.findIndex((page) =>
      pageCaptures(page, lecture, lectures).some(
        (capture) => capture.id === target.slice(8),
      ),
    );
  if (!target?.startsWith("t-")) return -1;
  const cue = lecture.cues.findIndex(
    (item) => Math.floor(item.start) === Number(target.slice(2)),
  );
  if (cue < 0) return -1;
  const chronological = pages.findIndex((page) => {
    const transcript = sectionTranscript(page, lecture);
    return (
      transcript.range && transcript.cues.some((item) => item.index === cue)
    );
  });
  return chronological >= 0
    ? chronological
    : pages.findIndex((page) =>
        sectionTranscript(page, lecture).cues.some(
          (item) => item.index === cue,
        ),
      );
}

/** Fallback pages resolve saved visuals directly; authored citations stay strictly validated. */
export function pageCaptures(
  page: GuidePage,
  lecture: Lecture,
  lectures: Lecture[],
): SectionCapture[] {
  if (!page.savedCaptureId) return sectionCaptures(page, lecture, lectures);
  const capture = lecture.captures.find(
    (capture) => capture.id === page.savedCaptureId,
  );
  return capture
    ? [{ ...capture, evidenceId: `${lecture.id}:c:${capture.id}` }]
    : [];
}

export function sectionCaptures(
  section: GuideSection,
  lecture: Lecture,
  lectures: Lecture[],
): SectionCapture[] {
  return [...new Set(section.citations)].flatMap((id) => {
    const source = lecture.evidence?.find(
      (s) =>
        s.id === id && s.kind === "capture" && s.courseId === lecture.courseId,
    );
    const owner =
      source &&
      lectures.find(
        (l) => l.id === source.lectureId && l.courseId === lecture.courseId,
      );
    const capture = owner?.captures.find(
      (c) => c.file === source?.path && `${owner.id}:c:${c.id}` === id,
    );
    return capture ? [{ ...capture, evidenceId: id }] : [];
  });
}

export function sectionTranscript(section: GuideSection, lecture: Lecture) {
  let range =
    section.startSeconds !== undefined && section.endSeconds !== undefined
      ? { start: section.startSeconds, end: section.endSeconds }
      : undefined;
  // Legacy guides encode their chronological windows in their headings.
  const heading = section.title.match(
    /(?:^|\s)(\d{1,3}:\d{2}(?::\d{2})?)\s*[–—-]\s*(\d{1,3}:\d{2}(?::\d{2})?)(?=\s|$)/,
  );
  if (!range && heading)
    range = {
      start: secondsFromTimestamp(heading[1]),
      end: secondsFromTimestamp(heading[2]),
    };
  if (range && (range.end <= range.start || range.start < 0)) range = undefined;
  const cited = new Set(section.citations);
  const cues = lecture.cues
    .map((cue, index) => ({ ...cue, index }))
    .filter((cue) =>
      range
        ? (cue.start >= range.start &&
            (cue.start < range.end ||
              (cue.start === range.end &&
                cited.has(`${lecture.id}:t${cue.index}`)) ||
              (range.end >= lecture.duration && cue.start === range.end))) ||
          (cue.start < range.start && cue.end > range.start)
        : cited.has(`${lecture.id}:t${cue.index}`),
    );
  return { cues, range };
}

export function recordingAt(sourceUrl: string, seconds?: number) {
  try {
    const url = new URL(sourceUrl);
    if (url.protocol !== "https:" || url.username || url.password) return null;
    const panopto = /\/Panopto\/Pages\/(Viewer|Embed)\.aspx$/i.test(
      url.pathname,
    );
    if (
      panopto &&
      seconds !== undefined &&
      Number.isFinite(seconds) &&
      seconds >= 0
    ) {
      url.searchParams.set("start", String(seconds));
      return { href: url.toString(), seeks: true };
    }
    return { href: url.toString(), seeks: false };
  } catch {
    return null;
  }
}

/** Legacy guides get an extractive outline until authored Fast notes are saved. */
export function fastPageMarkdown(section: GuideSection): string {
  if (section.fastMarkdown) return section.fastMarkdown;
  const points = section.markdown
    .split(/\n\s*\n|\n(?=[-*] )/)
    .map((text) => text.trim())
    .filter((text) => !isGuideConstructionNote(text.replace(/^[-*]\s+/, "")))
    .filter((text) => text && !/^(#{1,6} |!\[|\[.*\]$|```)/.test(text))
    .map((text) => text.replace(/^[-*]\s+/, "").replace(/\n/g, " "))
    .map((text) => text.split(/(?<=[.!?])\s+(?=[A-Z])/)[0])
    .filter(Boolean);
  return (
    [...new Set(points)]
      .slice(0, 6)
      .map((text) => `- ${text}`)
      .join("\n") ||
    "- Review the linked slide and Detailed explanation for this page."
  );
}
