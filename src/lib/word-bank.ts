import type { Concept, Guide, Citation } from "./schema";

export const wordBankPolicy = `Before drafting, read get_course's concepts (the course Word bank). Validate each needed definition against the current lecture's primary evidence. For an existing term whose definition fits, reuse it unchanged: omit it from output concepts and do not generate or repeat a prose definition. Write [term](#word-bank-TERM), with TERM the URL-encoded exact Word bank term, wherever a vocabulary explanation is needed; the reader displays the saved definition on hover, focus or tap. For a new term, add one cited concepts entry and use the same link syntax. If an existing definition is incomplete or misleading in this context, output only the supported addition in definition and a nonempty extensionReason explaining why the current lecture requires it. Saving appends the addition, reason and current lecture title/date/ID to the original definition, preserving its text and citations. Never silently replace a definition or create another entry for the same term. Preserve substantive teaching, worked steps and definitions actually discussed by the lecturer; move only added vocabulary asides into popovers. Never add glossary lists/pages to guide sections.`;

export function termKey(term: string) {
  return term.trim().toLocaleLowerCase("en-US");
}

export function wordBankDefinition(
  href: string | undefined,
  concepts: Concept[],
) {
  if (!href?.startsWith("#word-bank-")) return;
  let term: string;
  try {
    term = decodeURIComponent(href.slice(11));
  } catch {
    return;
  }
  // Historical duplicates are left intact; show their distinct saved definitions.
  const definitions = concepts
    .filter((c) => termKey(c.term) === termKey(term))
    .map((c) => c.definition);
  return [...new Set(definitions)].join("\n\n") || undefined;
}

export function mergeWordBank(
  concepts: Concept[],
  entries: Guide["concepts"],
  lecture: { id: string; courseId: string; title: string; date: string },
  evidence: Citation[],
  newId: () => string,
) {
  const result = concepts.map((c) => ({ ...c, citations: [...c.citations] }));
  for (const entry of entries) {
    const existing = result.find(
      (c) =>
        c.courseId === lecture.courseId &&
        termKey(c.term) === termKey(entry.term),
    );
    const citations = entry.citations.map((id) =>
      evidence.find((s) => s.id === id)!,
    );
    if (existing) {
      if (!entry.extensionReason) continue;
      const addition = `\n\nAddition from ${lecture.title} (${lecture.date}; lecture ${lecture.id}). Why: ${entry.extensionReason}\n\n${entry.definition}`;
      if (!existing.definition.includes(addition)) {
        existing.definition += addition;
        existing.mastered = false;
      }
      for (const source of citations)
        if (!existing.citations.some((c) => c.id === source.id))
          existing.citations.push(source);
    } else {
      result.push({
        id: newId(),
        courseId: lecture.courseId,
        lectureId: lecture.id,
        term: entry.term,
        definition: entry.definition,
        citations,
        mastered: false,
      });
    }
  }
  return result;
}
