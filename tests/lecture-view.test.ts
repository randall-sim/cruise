import { test } from "node:test";
import assert from "node:assert/strict";
import {
  lecturePages,
  isCoveragePage,
  fastPageMarkdown,
  pageCaptures,
  lectureTargetPage,
  recordingAt,
  sectionTranscript,
  sectionCaptures,
} from "../src/lib/lecture-view";
import {
  remarkCitations,
  remarkLectureContextCards,
  sourceExcerptMarkdown,
} from "../src/lib/markdown-citations";
import type { Lecture } from "../src/lib/schema";
import {
  guideSchema,
  lectureGuideOutputSchema,
  multipleChoiceQuestionSchema,
} from "../src/lib/schema";

const lecture: Lecture = {
  id: "lecture",
  courseId: "course",
  title: "Lecture",
  date: "2026-09-27",
  sourceUrl: "",
  createdAt: "",
  captures: [],
  duration: 120,
  status: "imported",
  notesPath: "",
  transcriptPath: "",
  captureCoverage: "",
  reviewed: false,
  cues: [
    { start: 0, end: 12, text: "Opening" },
    { start: 12, end: 20, text: "Uncited explanation" },
    { start: 20, end: 20, text: "Point anchor" },
    { start: 30, end: 40, text: "Next topic" },
    { start: 120, end: 120, text: "Final anchor" },
  ],
};
test("source excerpts omit the discovery preamble while retaining the reading's teaching", () => {
  const prefix =
    "Read Canvas DocViewer during lecture7 resource discovery. Targeted paraphrase of Developmental Explanations, especially printed pp365–366 and its summary before Ecological Explanations: ";
  assert.equal(
    sourceExcerptMarkdown(
      prefix +
        "Comparative distributions permit inference; this is not direct evidence.",
    ),
    "Comparative distributions permit inference; this is not direct evidence.",
  );
  assert.equal(
    sourceExcerptMarkdown("A caveat: many genes predate Bilateria."),
    "A caveat: many genes predate Bilateria.",
  );
});

test("legacy section ranges show uncited raw cues and point anchors without spilling into the next section", () => {
  const section = {
    title: "0:00–0:30 • Opening",
    markdown: "",
    citations: ["lecture:t0"],
  };
  assert.deepEqual(
    sectionTranscript(section, lecture).cues.map((c) => c.text),
    ["Opening", "Uncited explanation", "Point anchor"],
  );
  assert.equal(
    sectionTranscript(
      {
        ...section,
        title: "Overview",
        citations: ["lecture:t0", "lecture:t4"],
      },
      lecture,
    ).cues.length,
    2,
  );
  assert.deepEqual(
    sectionTranscript(
      { ...section, startSeconds: 30, endSeconds: 120 },
      lecture,
    ).cues.map((c) => c.index),
    [3, 4],
  );
  assert.equal(
    sectionTranscript(
      { ...section, title: "Overview", citations: ["other:t0"] },
      lecture,
    ).cues.length,
    0,
  );
});
test("explicitly cited boundary anchors remain in the page transcript without including uncited next topics", () => {
  const page = {
    title: "Point-anchored explanation",
    markdown: "Teaching with a final timestamped citation.",
    startSeconds: 0,
    endSeconds: 20,
    citations: ["lecture:t2"],
  };
  assert.deepEqual(
    sectionTranscript(page, lecture).cues.map((cue) => cue.index),
    [0, 1, 2],
  );
  assert.deepEqual(
    sectionTranscript({ ...page, citations: [] }, lecture).cues.map(
      (cue) => cue.index,
    ),
    [0, 1],
  );
});

test("recording links preserve session parameters and safely choose player-specific timestamps", () => {
  const result = recordingAt(
    "https://school.hosted.panopto.com/Panopto/Pages/Viewer.aspx?id=recording&start=99&instance=abc",
    20.5,
  )!;
  const url = new URL(result.href);
  assert.equal(result.seeks, true);
  assert.equal(url.searchParams.get("start"), "20.5");
  assert.equal(url.searchParams.get("id"), "recording");
  assert.equal(url.searchParams.get("instance"), "abc");
  assert.equal(url.searchParams.getAll("start").length, 1);
  assert.equal(
    recordingAt("https://school.example/recording/1", 20)!.href,
    "https://school.example/recording/1",
  );
  assert.equal(
    recordingAt("https://school.example/recording/1", 20)!.seeks,
    false,
  );
  for (const value of [
    "",
    "javascript:alert(1)",
    "https://user:secret@example.edu/video",
  ])
    assert.equal(recordingAt(value, 0), null);
});
test("capture references resolve only to verified images in the course and deduplicate", () => {
  const capture = {
    id: "img1",
    seconds: 5,
    kind: "slide" as const,
    stream: "Slides",
    file: "courses/course/lectures/lecture/captures/img1.png",
    caption: "Slide",
  };
  const current = {
    ...lecture,
    captures: [capture],
    evidence: [
      {
        id: "lecture:c:img1",
        lectureId: "lecture",
        courseId: "course",
        title: "Slide",
        text: "",
        path: capture.file,
        kind: "capture" as const,
      },
    ],
  };
  const section = {
    title: "",
    markdown: "",
    citations: ["lecture:c:img1", "lecture:c:img1", "unknown"],
  };
  assert.equal(sectionCaptures(section, current, [current]).length, 1);
  assert.equal(
    sectionCaptures(section, current, [{ ...current, courseId: "other" }])
      .length,
    0,
  );
  assert.equal(
    sectionCaptures(
      section,
      {
        ...current,
        evidence: current.evidence.map((s) => ({
          ...s,
          path: "https://untrusted.example/image.png",
        })),
      },
      [current],
    ).length,
    0,
  );
});
test("readable image references leave code and unknown markers unchanged", () => {
  const tree = {
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [
          { type: "text", value: "See [capture-id] and [constructor]." },
        ],
      },
      { type: "code", value: "const image = '[capture-id]'" },
      {
        type: "paragraph",
        children: [{ type: "inlineCode", value: "[capture-id]" }],
      },
    ],
  };
  remarkCitations({
    links: {
      "capture-id": { label: "Linked image 1", href: "#section-0-images-img1" },
    },
  })(tree);
  const serialized = JSON.stringify(tree);
  assert.match(serialized, /Linked image 1/);
  assert.match(serialized, /\[constructor\]/);
  assert.equal(tree.children[1].value, "const image = '[capture-id]'");
  assert.equal(tree.children[2].children![0].value, "[capture-id]");
});

test("hidden transcript citations disappear while visual links, prose times and code remain", () => {
  const tree = {
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [
          { type: "text", value: "Teaching at 02:38. [cue] See [image]." },
        ],
      },
      { type: "code", value: "[cue]" },
    ],
  };
  remarkCitations({
    links: {
      cue: { label: "02:38", href: "#cue", hidden: true },
      image: { label: "Linked image 1", href: "#image" },
    },
  })(tree);
  assert.equal(
    tree.children[0].children!.map((node) => node.value || "").join(""),
    "Teaching at 02:38.  See .",
  );
  assert.match(JSON.stringify(tree), /Linked image 1/);
  assert.equal(tree.children[1].value, "[cue]");
});

test("reading asides become source cards while main teaching and code remain unchanged", () => {
  const paragraph = {
    type: "paragraph",
    children: [
      {
        type: "strong",
        children: [
          { type: "text", value: "Connection to the previous lecture:" },
        ],
      },
      { type: "text", value: " The mechanism continues here. " },
      {
        type: "link",
        url: "/?course=course&lecture=prior",
        children: [{ type: "text", value: "Lecture 6 slides" }],
      },
    ],
  };
  const tree = {
    type: "root",
    children: [
      paragraph,
      {
        type: "paragraph",
        children: [{ type: "text", value: "Main teaching." }],
      },
      { type: "code", value: "Reading clarification: code" },
    ],
  };
  remarkLectureContextCards()(tree);
  const serialized = JSON.stringify(tree);
  assert.equal(tree.children[0].type, "blockquote");
  assert.match(serialized, /lecture-context-card/);
  assert.match(serialized, /Open source: Lecture 6 slides/);
  assert.match(serialized, /"target":"_blank"/);
  assert.match(serialized, /The mechanism continues here/);
  assert.equal(tree.children[1].type, "paragraph");
  assert.match(JSON.stringify(tree.children[2]), /Reading clarification: code/);
});

test("lecture reading hides construction notes but preserves teaching, links and code", () => {
  const tree = {
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [
          { type: "text", value: "Visual: recording screenshot matched to " },
          {
            type: "link",
            url: "https://example.edu/slides",
            children: [{ type: "text", value: "original lecture deck" }],
          },
          { type: "text", value: ", page 4. [image]" },
        ],
      },
      {
        type: "paragraph",
        children: [
          {
            type: "text",
            value:
              "Visual: primecheck monopolizes output when it does not yield.",
          },
        ],
      },
      {
        type: "paragraph",
        children: [
          {
            type: "text",
            value:
              "A reconstruction cannot resolve the board's unreadable label.",
          },
        ],
      },
      { type: "code", value: "Visual: saved recording frame." },
    ],
  };
  remarkCitations({ links: {}, hideConstructionNotes: true })(tree);
  assert.equal(tree.children.length, 3);
  assert.match(JSON.stringify(tree), /primecheck monopolizes/);
  assert.match(JSON.stringify(tree), /unreadable label/);
  assert.equal(tree.children[2].value, "Visual: saved recording frame.");
  const section = {
    title: "Teaching",
    markdown:
      "Visual: saved recording frame at 16:29. [image]\n\nThe mechanism has two stages.",
    citations: [],
  };
  assert.equal(fastPageMarkdown(section), "- The mechanism has two stages.");
});

test("guide pages retain original anchors and include ancillary material without losing text-only teaching", () => {
  const current: Lecture = {
    ...lecture,
    guide: {
      summary: "Opening orientation",
      sections: [
        {
          title: "First topic",
          markdown: "Complete explanation",
          citations: ["lecture:t0"],
          startSeconds: 0,
          endSeconds: 30,
        },
        {
          title: "Second topic",
          markdown: "Spoken explanation",
          citations: ["lecture:t3"],
          startSeconds: 30,
          endSeconds: 120,
        },
      ],
      logistics: [{ text: "Read chapter two", citations: ["lecture:t0"] }],
      concepts: [
        { term: "Interval", definition: "A range", citations: ["lecture:t0"] },
      ],
      questions: [
        { question: "Why?", answer: "Because.", citations: ["lecture:t0"] },
      ],
      gaps: ["Board unavailable"],
    },
  };
  const pages = lecturePages(current, []);
  assert.deepEqual(
    pages.map((page) => page.id),
    [
      "guide-overview",
      "section-0",
      "section-1",
      "guide-checks",
      "guide-logistics",
    ],
  );
  assert.equal(pages[2].markdown, "Spoken explanation");
  assert.equal(pages[3].kind, "checks");
  assert.equal(current.guide!.concepts[0].term, "Interval");
  assert.equal(lectureTargetPage(pages, current, [], "t-30"), 2);
  assert.equal(lectureTargetPage(pages, current, [], "capture-missing"), -1);
  assert.equal(lectureTargetPage(pages, current, [], "t-999"), -1);
});

test("new captures and captures without a guide remain readable without an evidence snapshot", () => {
  const capture = {
    id: "new",
    seconds: 5,
    kind: "slide" as const,
    stream: "Slides",
    file: "courses/course/lectures/lecture/captures/new.png",
    caption: "New visual",
  };
  const current = { ...lecture, captures: [capture] };
  const pages = lecturePages(current, [current]);
  assert.equal(pages.length, 2);
  assert.equal(pageCaptures(pages[1], current, [current])[0].id, "new");
  assert.equal(lectureTargetPage(pages, current, [current], "capture-new"), 1);
  assert.equal(sectionCaptures(pages[1], current, [current]).length, 0);
  assert.equal(pageCaptures(pages[1], lecture, [current]).length, 0);
});

test("quiz follows all teaching and precedes categorized and legacy ancillary pages", () => {
  const current: Lecture = {
    ...lecture,
    guide: {
      summary: "Lecture primer",
      sections: [
        {
          title: "Opening logistics",
          category: "logistics",
          markdown: "Read",
          citations: ["lecture:t0"],
        },
        {
          title: "Teaching",
          category: "lecture",
          markdown: "Learn",
          citations: ["lecture:t0"],
        },
        {
          title: "Supplementary reading",
          markdown: "Extra",
          citations: ["lecture:t0"],
        },
        {
          title: "35. Coverage audit: remaining gaps",
          markdown: "Gaps",
          citations: ["lecture:t0"],
        },
        {
          title: "Connections to earlier lectures",
          markdown: "Connections",
          citations: ["lecture:t0"],
        },
      ],
      logistics: [{ text: "Due", citations: ["lecture:t0"] }],
      concepts: [],
      questions: [
        { question: "Why?", answer: "Because", citations: ["lecture:t0"] },
      ],
      gaps: ["Missing board"],
    },
  };
  assert.deepEqual(
    lecturePages(current, []).map((page) => page.id),
    [
      "guide-overview",
      "section-1",
      "guide-checks",
      "section-0",
      "section-2",
      "section-4",
      "guide-logistics",
    ],
  );
});

test("coverage appendices are hidden in old and categorized guides without hiding teaching or resource links", () => {
  const page = {
    title: "Teaching",
    markdown: "Lesson",
    citations: ["lecture:t0"],
  };
  for (const title of [
    "Coverage and resources",
    "Evidence coverage and gaps",
    "62 · Coverage audit and lecture resources",
    "00:00–00:30 · Coverage and resources",
  ])
    assert.equal(isCoveragePage({ ...page, title }), true);
  assert.equal(isCoveragePage({ ...page, category: "coverage" }), true);
  assert.equal(isCoveragePage({ ...page, title: "Lecture resources" }), false);
  assert.equal(
    isCoveragePage({
      ...page,
      title: "Testing code coverage",
      category: "lecture",
    }),
    false,
  );
});

test("new quiz schema requires about ten questions and explanations, preserving legacy saved guides", () => {
  const question = {
    question: "Which follows?",
    answer: "Second",
    citations: ["lecture:t0"],
    correctOption: 1,
    options: [
      { text: "First", explanation: "First precedes second." },
      { text: "Second", explanation: "Second follows first." },
    ],
  };
  assert.equal(multipleChoiceQuestionSchema.safeParse(question).success, true);
  assert.equal(
    multipleChoiceQuestionSchema.safeParse({ ...question, correctOption: 2 })
      .success,
    false,
  );
  assert.equal(
    multipleChoiceQuestionSchema.safeParse({
      ...question,
      options: [{ text: "First", explanation: "" }, question.options[1]],
    }).success,
    false,
  );
  assert.equal(
    multipleChoiceQuestionSchema.safeParse({
      ...question,
      options: [question.options[0], question.options[0]],
    }).success,
    false,
  );
  const guide = {
    summary: "Lecture primer",
    sections: [
      { title: "Teaching", markdown: "Learn", citations: ["lecture:t0"] },
    ],
    logistics: [],
    concepts: [],
    gaps: [],
    questions: Array.from({ length: 10 }, () => question),
  };
  assert.equal(lectureGuideOutputSchema.safeParse(guide).success, true);
  assert.equal(
    lectureGuideOutputSchema.safeParse({ ...guide, questions: [question] })
      .success,
    false,
  );
  const legacy = {
    question: "Why?",
    answer: "Because",
    citations: ["lecture:t0"],
  };
  assert.equal(
    guideSchema.safeParse({ ...guide, questions: [legacy] }).success,
    true,
  );
  assert.equal(
    lectureGuideOutputSchema.safeParse({ ...guide, questions: [legacy] })
      .success,
    false,
  );
  assert.equal(
    guideSchema.safeParse({
      ...guide,
      questions: [{ ...question, correctOption: 99 }],
    }).success,
    false,
  );
});

test("Fast prefers authored exam-review bullets and safely outlines older pages", () => {
  const section = {
    title: "Review",
    markdown:
      "A mechanism has two stages. Details explain why.\n\n- Check its assumptions. Then calculate.",
    citations: [],
  };
  assert.equal(
    fastPageMarkdown({ ...section, fastMarkdown: "- Compare the two stages." }),
    "- Compare the two stages.",
  );
  assert.equal(
    fastPageMarkdown(section),
    "- A mechanism has two stages.\n- Check its assumptions.",
  );
  assert.ok(fastPageMarkdown({ ...section, markdown: "" }).startsWith("- "));
});
