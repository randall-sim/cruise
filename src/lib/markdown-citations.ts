export type CitationLink = { label: string; href: string; hidden?: boolean };

export function sourceExcerptMarkdown(text: string): string {
  return text
    .replace(
      /^Read Canvas DocViewer during [^\n]*?resource discovery\.\s*Targeted paraphrase of [^\n]*?:\s*/i,
      "",
    )
    .replace(/^Imported: [^\n]+\n?/gm, "")
    .trim();
}
type MdNode = {
  type: string;
  value?: string;
  url?: string;
  children?: MdNode[];
  data?: { hProperties?: Record<string, unknown> };
};

const nodeText = (node: MdNode): string =>
  node.value || node.children?.map(nodeText).join("") || "";

/** Existing labeled reading asides become cards without changing their saved evidence. */
export function remarkLectureContextCards() {
  return (tree: MdNode) => {
    const visit = (node: MdNode) => {
      if (
        !node.children ||
        ["code", "inlineCode", "blockquote"].includes(node.type)
      )
        return;
      node.children = node.children.map((child) => {
        if (
          child.type !== "paragraph" ||
          !/^(?:reading clarification|connection to (?:the )?(?:previous|prior|earlier) lectures?|prior[- ]lecture connection|course resource|supplementary (?:reading|context)|resource clarification)\s*:/i.test(
            nodeText(child).trim(),
          )
        ) {
          visit(child);
          return child;
        }
        const sources: MdNode[] = [];
        const collect = (part: MdNode) => {
          if (
            part.type === "link" &&
            part.url &&
            /^(?:https:\/\/|\/\?)/.test(part.url)
          ) {
            if (!sources.some((source) => source.url === part.url))
              sources.push(part);
          } else part.children?.forEach(collect);
        };
        collect(child);
        return {
          type: "blockquote",
          data: { hProperties: { className: "lecture-context-card" } },
          children: [
            ...(sources.length
              ? [
                  {
                    type: "paragraph",
                    data: {
                      hProperties: { className: "lecture-context-links" },
                    },
                    children: sources.map((source) => ({
                      type: "link",
                      url: source.url,
                      children: [{ type: "text", value: "Link ↗" }],
                      data: {
                        hProperties: {
                          className: "lecture-context-source",
                          title: nodeText(source),
                          "aria-label": `Open source: ${nodeText(source)}`,
                          target: "_blank",
                          rel: "noopener noreferrer",
                        },
                      },
                    })),
                  },
                ]
              : []),
            child,
          ],
        };
      });
    };
    visit(tree);
  };
}

export function isGuideConstructionNote(text: string): boolean {
  const plain = text.replace(/\*\*/g, "").trim();
  return (
    /^(?:visual provenance|capture provenance|guide construction|capture note|source matching|evidence audit)\s*:/i.test(
      plain,
    ) ||
    (/^visual\s*:/i.test(plain) &&
      /(?:recording (?:screenshot|frame|screen|stream)|saved (?:recording|capture|frame)|matched to|matching (?:instructor|lecture|slide|deck)|original focused|deck page|stream provenance)/i.test(
        plain,
      ))
  );
}

// Transform text nodes only: code and existing links retain their syntax.
// All destinations come from verified workspace evidence.
export function remarkCitations({
  links,
  hideConstructionNotes = false,
}: {
  links: Record<string, CitationLink>;
  hideConstructionNotes?: boolean;
}) {
  return (tree: MdNode) => {
    const visit = (node: MdNode) => {
      if (
        ["link", "image", "code", "inlineCode"].includes(node.type) ||
        !node.children
      )
        return;
      node.children = node.children.flatMap((child) => {
        if (hideConstructionNotes && child.type === "paragraph") {
          const plainText = (part: MdNode): string =>
            part.value || part.children?.map(plainText).join("") || "";
          if (isGuideConstructionNote(plainText(child))) return [];
        }
        if (child.type !== "text" || !child.value) {
          visit(child);
          return [child];
        }
        const result: MdNode[] = [];
        let offset = 0;
        for (const match of child.value.matchAll(/\[([^\]\n]+)\]/g)) {
          const link = Object.hasOwn(links, match[1])
            ? links[match[1]]
            : undefined;
          if (!link) continue;
          if (match.index! > offset)
            result.push({
              type: "text",
              value: child.value.slice(offset, match.index),
            });
          if (!link.hidden)
            result.push({
              type: "link",
              url: link.href,
              children: [{ type: "text", value: link.label }],
            });
          offset = match.index! + match[0].length;
        }
        if (offset < child.value.length)
          result.push({ type: "text", value: child.value.slice(offset) });
        return result;
      });
    };
    visit(tree);
  };
}
