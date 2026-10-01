import { NextRequest, NextResponse } from "@/lib/web-request";
import {
  assignmentStepBytes,
  getAssignmentTimeline,
  getAssignmentParts,
  getAssignmentPartChanges,
  readAssignmentSnapshot,
} from "@/lib/assignment-timeline";
import { assignmentMediaType } from "@/lib/assignment-files";
import { localRequest, errorResponse } from "@/lib/http";

export const runtime = "nodejs";
export async function GET(request: NextRequest) {
  try {
    localRequest(request, false, { allowDocumentNavigation: true });
    const query = request.nextUrl.searchParams;
    const input = {
      courseId: query.get("courseId"),
      assignmentId: query.get("assignmentId"),
      stepId: query.get("stepId") || undefined,
      path: query.get("path") || undefined,
      offset: Number(query.get("offset") || 0),
      diffOffset: Number(query.get("diffOffset") || 0),
      directory: query.get("directory") || undefined,
      mode: query.get("mode"),
    };
    if (query.get("asset") === "1" || input.mode === "asset") {
      const file = await assignmentStepBytes(input);
      const type = assignmentMediaType(file.path);
      return new NextResponse(new Uint8Array(file.bytes), {
        headers: {
          "Content-Type": type.startsWith("image/")
            ? type
            : "text/plain; charset=utf-8",
          "Content-Disposition": `${query.get("download") === "1" ? "attachment" : "inline"}; filename*=UTF-8''${encodeURIComponent(file.path.split("/").pop()!)}`,
          "Content-Security-Policy": "default-src 'none'; sandbox",
          "X-Content-Type-Options": "nosniff",
          "Cache-Control": "no-store",
        },
      });
    }
    return NextResponse.json(
      input.mode === "parts"
        ? await getAssignmentParts(input)
        : input.mode === "changes"
          ? await getAssignmentPartChanges(input)
          : input.mode === "directory" || input.mode === "file"
            ? await readAssignmentSnapshot(input)
            : await getAssignmentTimeline(input),
      {
        headers: { "Cache-Control": "no-store" },
      },
    );
  } catch (error) {
    return errorResponse(error);
  }
}
