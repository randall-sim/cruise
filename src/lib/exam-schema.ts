import { z } from "zod";

const ids = z.array(z.string().min(1)).max(500);
export const questionType = z.enum([
  "multiple_choice",
  "multiple_select",
  "true_false",
  "short_answer",
  "essay",
  "calculation",
  "proof",
  "code",
  "diagram",
]);
export const examScopeSchema = z.discriminatedUnion("mode", [
  z.object({
    mode: z.literal("dates"),
    from: z.iso.date(),
    through: z.iso.date(),
  }),
  z.object({ mode: z.literal("lectures"), lectureIds: ids.min(1) }),
  z.object({
    mode: z.literal("after_event"),
    event: z.string().min(1).max(200),
    date: z.iso.date(),
    through: z.iso.date(),
    evidenceIds: ids.min(1),
  }),
]);
export const examInput = z.object({
  courseId: z.string().regex(/^[a-zA-Z0-9_-]{1,100}$/),
  prompt: z.string().trim().min(1).max(12000),
  scope: examScopeSchema,
  questionCount: z.number().int().min(1).max(60).default(12),
  questionTypes: z.array(questionType).max(9).default([]),
  durationMinutes: z.number().int().min(1).max(360).optional(),
  contentSourceIds: ids.default([]),
  formatSourceIds: ids.default([]),
});
export type ExamRequest = z.infer<typeof examInput> & {
  lectureIds: string[];
  scopeLabel: string;
  contentEvidenceIds: string[];
  scopeEvidenceIds: string[];
};
export const examOutputSchema = z.object({
  title: z.string().min(1).max(200),
  instructions: z.string().min(1).max(8000),
  format: z.object({
    rationale: z.string().min(1).max(5000),
    inferred: z.boolean(),
    citations: ids,
  }),
  questions: z
    .array(
      z.object({
        type: questionType,
        topic: z.string().min(1).max(200),
        prompt: z.string().min(1).max(12000),
        points: z.number().int().min(1).max(100),
        choices: z
          .array(
            z.object({
              id: z.string().min(1).max(20),
              text: z.string().min(1).max(3000),
            }),
          )
          .max(8)
          .default([]),
        correctChoiceIds: z.array(z.string()).max(8).default([]),
        answer: z.string().min(1).max(12000),
        explanation: z.string().min(1).max(12000),
        rubric: z
          .array(
            z.object({
              criterion: z.string().min(1).max(3000),
              points: z.number().int().min(1).max(100),
            }),
          )
          .min(1)
          .max(20),
        citations: ids.min(1),
      }),
    )
    .min(1)
    .max(60),
  gaps: z.array(z.string().max(1000)).max(100),
});
export type ExamOutput = z.infer<typeof examOutputSchema>;
