import { z } from "zod";
import { idSchema } from "./schema";

const coordinate = z.number().finite().min(0).max(2400);
const tone = z.enum([
  "ink",
  "muted",
  "green",
  "blue",
  "orange",
  "purple",
  "white",
]);
export const diagramSchema = z
  .object({
    width: z.number().int().min(800).max(2000),
    height: z.number().int().min(300).max(2400),
    elements: z
      .array(
        z.discriminatedUnion("type", [
          z
            .object({
              type: z.literal("text"),
              x: coordinate,
              y: coordinate,
              lines: z.array(z.string().max(180)).min(1).max(30),
              size: z.number().min(20).max(64).default(28),
              tone: tone.default("ink"),
              bold: z.boolean().default(false),
              mono: z.boolean().default(false),
              anchor: z.enum(["start", "middle", "end"]).default("start"),
            })
            .strict(),
          z
            .object({
              type: z.literal("rect"),
              x: coordinate,
              y: coordinate,
              width: coordinate.positive(),
              height: coordinate.positive(),
              tone: tone.default("green"),
            })
            .strict(),
          z
            .object({
              type: z.literal("line"),
              x1: coordinate,
              y1: coordinate,
              x2: coordinate,
              y2: coordinate,
              arrow: z.boolean().default(false),
              dashed: z.boolean().default(false),
              tone: tone.default("muted"),
            })
            .strict(),
          z
            .object({
              type: z.literal("ellipse"),
              cx: coordinate,
              cy: coordinate,
              rx: coordinate.positive(),
              ry: coordinate.positive(),
              tone: tone.default("blue"),
            })
            .strict(),
        ]),
      )
      .min(1)
      .max(300),
  })
  .strict()
  .superRefine((diagram, ctx) => {
    for (const [index, e] of diagram.elements.entries()) {
      const bounds =
        e.type === "rect"
          ? [e.x + e.width, e.y + e.height]
          : e.type === "ellipse"
            ? [e.cx + e.rx, e.cy + e.ry]
            : e.type === "line"
              ? [Math.max(e.x1, e.x2), Math.max(e.y1, e.y2)]
              : [e.x, e.y + (e.lines.length - 1) * e.size * 1.4];
      if (
        bounds[0] > diagram.width ||
        bounds[1] > diagram.height ||
        (e.type === "ellipse" && (e.cx < e.rx || e.cy < e.ry))
      )
        ctx.addIssue({
          code: "custom",
          path: ["elements", index],
          message: "Element extends beyond the diagram canvas",
        });
    }
  });
export type Diagram = z.infer<typeof diagramSchema>;

export const artifactInput = z
  .object({
    lectureId: idSchema,
    captureId: idSchema,
    title: z.string().trim().min(1).max(180),
    description: z.string().trim().min(10).max(4000),
    sourceIds: z.array(z.string().min(1).max(1000)).min(1).max(60),
    uncertainties: z.array(z.string().min(1).max(1000)).min(1).max(20),
    diagram: diagramSchema.optional(),
    image: z.string().max(20_000_000).optional(),
  })
  .strict();

export const readabilityInput = z
  .object({
    lectureId: idSchema,
    reviews: z
      .array(
        z
          .object({
            captureId: idSchema,
            status: z.enum(["readable", "unclear", "not_applicable"]),
            reason: z.string().trim().min(5).max(1500),
          })
          .strict(),
      )
      .min(1)
      .max(1000),
  })
  .strict();
