import { z } from "zod";
import { idSchema, webUrl } from "./schema";
import { mutate, newId, requireCourse, writePrivate } from "./store";

export const sourceInput = z.object({
  courseId: idSchema,
  title: z.string().trim().min(1).max(200),
  url: webUrl.default(""),
  text: z
    .string()
    .trim()
    .min(10)
    .max(300000)
    .describe(
      "Source teaching content only. Do not prepend resource-discovery, DocViewer-reading, capture, import, build or agent-process commentary. Retain substantive caveats and distinguish paraphrases from quotations. Put source identity and original location in title/url.",
    ),
});
export async function saveSource(input: unknown) {
  const data = sourceInput.parse(input);
  return mutate(async (state) => {
    requireCourse(state, data.courseId);
    const id = newId();
    const file = `courses/${data.courseId}/memory/${id}.md`;
    // The title/URL in every chunk keeps the imported source traceable after paragraph chunking.
    const header = `# ${data.title}\n\nImported: ${new Date().toISOString()}\nSource: ${data.url || "User-provided course material"}`;
    await writePrivate(file, `${header}\n\n${data.text}\n`);
    return { id, path: file };
  });
}
