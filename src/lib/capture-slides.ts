import type { Capture, CaptureArtifact } from "./schema";

export type CaptureSlide = {
  id: string;
  capture: Capture;
  artifact?: CaptureArtifact;
  url: string;
};
export function captureSlides(captures: Capture[]): CaptureSlide[] {
  return captures.flatMap((capture) => [
    { id: capture.id, capture, url: `/api/capture/${capture.id}` },
    ...(capture.artifacts || []).map((artifact) => ({
      id: `${capture.id}:artifact:${artifact.id}`,
      capture,
      artifact,
      url: `/api/capture/${capture.id}/artifacts/${artifact.id}`,
    })),
  ]);
}
