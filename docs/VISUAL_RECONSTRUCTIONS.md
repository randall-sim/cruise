# Readable lecture visuals

Review every saved teaching image at a size a person can actually read. A large
pixel count, a caption, or successful OCR does not prove readability. Look for
blur, distortion, low resolution, glare, distant boards, tiny code and labels.
Use the native original for doubtful cases, not only a reduced contact sheet.
Blank end frames and images with no teaching content are `not_applicable`; do
not invent a replacement just to fill them.

When important teaching content is unreadable:

1. Preserve the original capture, its ID, timestamp and provenance. Prefer an
   already available readable matching slide/deck/notes figure, or a better
   recording view if the authorized workflow can obtain one.
2. Read nearby original transcript cues and matching course Markdown notes or
   slides. Identify what can be supported and what remains unrecoverable. Do not
   guess exact handwriting, numerical values, formulas, graph coordinates, or
   details simply because they would make a plausible diagram.
3. Create a **reconstruction** of the supported concept or diagram. It may be a
   generated image or a code artifact, using `save_capture_artifact`. Label it as
   an interpretation from evidence, never a recovered photograph or exact
   transcription. Include concrete `uncertainties` even when the concept itself
   is well supported. Cite original transcript or course-note source IDs.
4. Prefer structured vector diagrams for text, code, process flows, graphs and
   relationships. The app renders the inert JSON diagram specification to SVG;
   no supplied HTML or JavaScript executes. For raster illustrations use the
   available image-generation tools, inspect the result, and supply a PNG/JPEG/
   WebP data URI. Do not generate a decorative picture when labels or a precise
   diagram are what the reader needs.
5. Inspect the rendered result. Ensure text is readable, labels do not overlap
   or clip, arrows express supported relationships, and important qualifications
   remain visible. Save the reconstruction with the original capture's ID.
6. Record `review_capture_readability` with each inspected capture's status and
   reason. If available sources cannot support a reconstruction, keep an explicit
   gap instead of fabricating one. Do not claim all captures were reviewed when
   only a subset was inspected.

Each reconstruction appears directly after its original in every applicable
carousel. Original capture citation links still open the original. The reader
can switch to the reconstruction, inspect its evidence and uncertainties, open
it full size, and view a diagram's JSON source. Multiple versions can coexist.
Generated visuals do not enter RAG as primary capture evidence, and do not turn
partial visual coverage into complete coverage. Deleting a guide does not erase
these original-linked assets.

## Tool contract

- `review_capture_readability`: `lectureId`, and `reviews` containing `captureId`,
  `status` (`readable`, `unclear`, `not_applicable`), and `reason`.
- `save_capture_artifact`: `lectureId`, `captureId`, `title`, `description`,
  `sourceIds`, `uncertainties`, and exactly one `diagram` or `image`.
- `get_capture_artifacts`: read a capture's review and reconstruction metadata.

Source IDs come from `search_course`, `get_lecture` cue indices, or existing
job evidence. At least one original transcript or Markdown note must support the
reconstruction. Generated guides or other reconstructions are not substitutes.
The tool validates IDs against the capture's course, but the agent must verify
that each source actually supports its claims.

`diagram` has numeric `width` (800–2000), `height` (300–2400), and `elements`.
Elements are allowlisted `text`, `rect`, `line`, or `ellipse`. See
`src/lib/visual-artifact-schema.ts` for the exact schema. Coordinates are canvas
units; use generous margins and text of at least 20 units. `text` accepts `lines`,
`x`, `y`, `size`, `bold`, `mono`, `anchor`; lines advance by `size * 1.4`.
Colors use named tones: ink, muted, green, blue, orange, purple, white.
Diagram JSON and SVG, raster images, and provenance snapshots are stored under
the lecture's `reconstructions/<original-capture-id>/` directory.
