import { z } from "zod";
import type { ExamRequest, ExamOutput } from "./exam-schema";
import type { Assignment } from "./assignments";

export const idSchema = z.string().regex(/^[a-zA-Z0-9_-]{1,100}$/);
export const semesterInput = z.object({
  season: z.enum(["Spring", "Summer", "Fall"]),
  year: z.number().int().min(1900).max(9999),
});
export type Semester = z.infer<typeof semesterInput>;
export const webUrl = z.union([
  z.literal(""),
  z.url().refine((v) => /^https:\/\//.test(v), "Use an https:// URL"),
]);
export const courseInput = z.object({
  code: z.string().trim().min(1).max(30),
  name: z.string().trim().min(1).max(150),
  term: z.string().trim().max(60).default(""),
  color: z.enum(["green", "orange", "blue", "purple"]).default("green"),
  canvasUrl: webUrl.default(""),
  websiteUrl: webUrl.default(""),
  notionUrl: webUrl.optional(),
});
export type Course = z.infer<typeof courseInput> & {
  id: string;
  createdAt: string;
  demo?: boolean;
};
export type Cue = { start: number; end: number; text: string };
export type CaptureArtifact = {
  id: string;
  title: string;
  description: string;
  createdAt: string;
  format: "diagram" | "image";
  file: string;
  definitionPath?: string;
  sourceIds: string[];
  uncertainties: string[];
};
export type Capture = {
  id: string;
  seconds: number;
  kind: "slide" | "whiteboard" | "demo";
  stream: string;
  file: string;
  caption: string;
  artifacts?: CaptureArtifact[];
  readability?: {
    status: "readable" | "unclear" | "not_applicable";
    reason: string;
    reviewedAt: string;
  };
};
export type Citation = {
  id: string;
  courseId: string;
  lectureId?: string;
  title: string;
  seconds?: number;
  text: string;
  path: string;
  url?: string;
  kind: "transcript" | "note" | "capture" | "assignment" | "file";
};
export const citationIds = z.array(z.string()).min(1).max(100);
export const multipleChoiceQuestionSchema = z
  .object({
    question: z.string().trim().min(1).max(3000),
    answer: z
      .string()
      .trim()
      .min(1)
      .max(5000)
      .describe("Explain the correct answer and its reasoning."),
    options: z
      .array(
        z.object({
          text: z.string().trim().min(1).max(2000),
          explanation: z
            .string()
            .trim()
            .min(1)
            .max(5000)
            .describe(
              "Explain specifically why this choice is correct or incorrect.",
            ),
        }),
      )
      .min(2)
      .max(6),
    correctOption: z
      .number()
      .int()
      .min(0)
      .max(5)
      .describe(
        "Zero-based index of the single correct choice; must be within options.",
      ),
    citations: citationIds,
  })
  .superRefine((question, ctx) => {
    if (question.correctOption >= question.options.length)
      ctx.addIssue({
        code: "custom",
        path: ["correctOption"],
        message: "Correct choice must exist in options",
      });
    if (
      new Set(question.options.map((option) => option.text.toLowerCase()))
        .size !== question.options.length
    )
      ctx.addIssue({
        code: "custom",
        path: ["options"],
        message: "Choices must be distinct",
      });
  });
export const guideSchema = z.object({
  summary: z
    .string()
    .min(10)
    .max(8000)
    .describe(
      "Lecture Overview: an evidence-grounded primer for the entire lecture. Highlight its central question, main ideas, why they matter and how they connect. Refresh relevant prerequisites and specific ideas from previous lectures, explaining how they help here. Give readers things to keep in mind, useful distinctions, assumptions and common pitfalls as they read. Cite supporting evidence, use Word bank links, label supplementary context and admit missing prior-lecture evidence. Write after reviewing the full lecture; be readable and substantive rather than a generic teaser or table of contents. Complete worked teaching stays in chronological pages.",
    ),
  sections: z
    .array(
      z.object({
        title: z.string().max(200),
        category: z
          .enum([
            "lecture",
            "supplementary",
            "logistics",
            "resources",
            "coverage",
            "connections",
          ])
          .optional()
          .describe(
            "Mark teaching pages lecture; mark ancillary pages with their category so the quiz follows the final teaching page, before ancillary material.",
          ),
        markdown: z
          .string()
          .min(1)
          .max(20000)
          .describe(
            "Complete explanation attached to this page's visual, or a text-only reading page. Use concise prose without omitting reasoning, examples, intermediate steps, qualifications or spoken details. Cite evidence near claims. Continue on another page when needed. Teach lecture content only; omit guide-construction commentary, Visual/provenance footers, screenshot/deck matching and capture/import/audit process notes. Keep production details in metadata. Write prior-lecture connections and external-resource clarifications as separate paragraphs starting **Connection to the previous lecture:** or **Reading clarification:** with a descriptive Markdown link to their original source; the reader renders them as source cards. Preserve teaching caveats and labeled reconstruction uncertainty.",
          ),
        fastMarkdown: z
          .string()
          .trim()
          .min(1)
          .max(8000)
          .optional()
          .describe(
            "Independently author Fast for every slide and teaching page, including continuations, from the complete slide and transcript/resource evidence. Never take the first sentence, clip Detailed, or mechanically extract opening lines. Fast must stand on its own, retaining the central idea and essential reasoning, conditions or pitfalls. Fast mode: 3–6 concise Markdown bullets for this page's most useful exam-review information. Prioritize key ideas, mechanisms, formulas and assumptions, distinctions, pitfalls and instructor assessment hints. Cite the same primary evidence, retain Word bank links, and do not claim exam coverage without evidence. Required for newly authored teaching pages; optional for older saved guides.",
          ),
        citations: citationIds.describe(
          "Evidence supporting this page. Include the relevant capture ID for a visual page; omit capture IDs for a text-only page. Avoid unrelated visuals. A capture can recur on continuation pages.",
        ),
        startSeconds: z.number().min(0).optional(),
        endSeconds: z.number().min(0).optional(),
      }),
    )
    .min(1)
    .max(100)
    .describe(
      "Ordered lecture carousel pages, following the complete lecture from beginning to end. One focused slide, board or demonstration context per page; text-only pages cover material without a relevant visual. Audit all transcript passages and saved teaching images before completing.",
    ),
  logistics: z
    .array(z.object({ text: z.string().max(3000), citations: citationIds }))
    .max(50),
  concepts: z
    .array(
      z.object({
        term: z.string().min(1).max(120),
        definition: z.string().min(1).max(5000),
        extensionReason: z
          .string()
          .trim()
          .min(1)
          .max(2000)
          .optional()
          .describe(
            "Existing term only: why this lecture needs a supported addition. definition contains only the new addition; the old definition is preserved automatically.",
          ),
        citations: citationIds,
      }),
    )
    .max(80)
    .describe(
      "Only new vocabulary or context additions for the course Word bank. Omit existing terms when their definitions fit. Link terms in guide Markdown using [term](#word-bank-URL_ENCODED_TERM) instead of repeating vocabulary asides. For an existing term needing an addition, supply extensionReason and only the addition in definition.",
    ),
  questions: z
    .array(
      z.union([
        multipleChoiceQuestionSchema,
        z
          .object({
            question: z.string().max(3000),
            answer: z.string().max(5000),
            citations: citationIds,
          })
          .strict(),
      ]),
    )
    .max(30),
  gaps: z
    .array(z.string().max(1000))
    .max(50)
    .describe(
      "Internal evidence limitations. Do not turn these into coverage/resources or evidence coverage and gaps pages in sections.",
    ),
});
export type Guide = z.infer<typeof guideSchema>;
// Old saved written-response checks remain readable; newly authored quizzes use choices.
export const lectureGuideOutputSchema = guideSchema.extend({
  questions: z
    .array(multipleChoiceQuestionSchema)
    .min(8)
    .max(12)
    .describe(
      "Author about ten (8–12) evidence-grounded multiple-choice questions covering the lecture's main ideas, reasoning and pitfalls. Supply plausible distractors and an explanation for every choice. Vary the correct choice position. Exclude logistics and supplementary-only material.",
    ),
});
export type Lecture = {
  id: string;
  courseId: string;
  title: string;
  date: string;
  sourceUrl: string;
  createdAt: string;
  cues: Cue[];
  captures: Capture[];
  duration: number;
  status: "imported" | "ready";
  guide?: Guide;
  evidence?: Citation[];
  notesPath: string;
  transcriptPath: string;
  captureCoverage: string;
  reviewed: boolean;
  hiddenFromUi?: boolean;
};
// Legacy task records are preserved for existing private workspaces, not synced.
export type Task = {
  id: string;
  source: "notion" | "canvas" | "local";
  externalId?: string;
  title: string;
  courseId: string | null;
  due: string | null;
  done: boolean;
  url: string;
  description: string;
  matchedTaskId?: string;
  matchReason?: string;
  archived?: boolean;
};
export type Concept = {
  id: string;
  courseId: string;
  lectureId: string;
  term: string;
  definition: string;
  citations: Citation[];
  mastered: boolean;
};
export type JobKind =
  "lecture" | "question" | "assignment" | "assessment" | "exam";
export const answerSchema = z.object({
  markdown: z.string().min(1).max(60000),
  citations: z.array(z.string()).max(100),
  gaps: z.array(z.string().max(1000)).max(30),
});
export type Job = {
  id: string;
  courseId: string;
  kind: JobKind;
  lectureId?: string;
  taskId?: string;
  prompt: string;
  context: Citation[];
  status: "queued" | "completed" | "failed";
  createdAt: string;
  completedAt?: string;
  result?: z.infer<typeof answerSchema>;
  examRequest?: ExamRequest;
  exam?: ExamOutput;
  error?: string;
};
// Legacy connector settings are retained on disk only; no active connector uses them.
export const settingsSchema = z.object({
  notionDataSourceId: z
    .string()
    .regex(/^[a-zA-Z0-9-]*$/)
    .max(100)
    .default(""),
  notionTitleProperty: z.string().max(100).default("Name"),
  notionCourseProperty: z.string().max(100).default("Course"),
  notionDateProperty: z.string().max(100).default("Due"),
  notionStatusProperty: z.string().max(100).default("Status"),
  notionDoneValues: z.string().max(200).default("Done,Complete,Completed"),
  canvasBaseUrl: webUrl.default(""),
  autoSync: z.boolean().default(false),
});
export type Settings = z.infer<typeof settingsSchema>;
export type State = {
  version: 1;
  semesters?: Semester[];
  assignments?: Assignment[];
  hiddenAssignments?: string[];
  courses: Course[];
  lectures: Lecture[];
  tasks: Task[];
  concepts: Concept[];
  jobs: Job[];
  settings: Settings;
  sync: { notion?: string; canvas?: string; errors: string[] };
};
export const lectureInput = z.object({
  courseId: idSchema,
  title: z.string().trim().min(1).max(200),
  date: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
  sourceUrl: webUrl.default(""),
  transcript: z.string().min(1).max(2_000_000),
  captureCoverage: z
    .string()
    .max(3000)
    .default("Transcript only. Visual material has not been captured."),
});
