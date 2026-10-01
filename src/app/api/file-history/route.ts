import { NextRequest, NextResponse } from "@/lib/web-request";
import { getFileHistory, downloadFileRevision } from "@/lib/file-history";
import { errorResponse, localRequest } from "@/lib/http";
export const runtime = "nodejs";
export async function GET(request: NextRequest) {
  try {
    localRequest(request, false, { allowDocumentNavigation: true });
    const q = request.nextUrl.searchParams;
    const input = {
      courseId: q.get("courseId"),
      assignmentId: q.get("assignmentId") || undefined,
      path: q.get("path") || undefined,
      historyId: q.get("historyId") || undefined,
      revisionId: q.get("revisionId") || undefined,
      offset: Number(q.get("offset") || 0),
      diffOffset: Number(q.get("diffOffset") || 0),
    };
    const side = q.get("snapshot");
    if (side === "before" || side === "after") {
      const { bytes, name } = await downloadFileRevision(input, side);
      return new NextResponse(new Uint8Array(bytes), {
        headers: {
          "Content-Type": "application/octet-stream",
          "Content-Disposition": `attachment; filename*=UTF-8''${encodeURIComponent(`${side}-${name}`)}`,
          "Content-Security-Policy": "default-src 'none'; sandbox",
          "X-Content-Type-Options": "nosniff",
          "Cache-Control": "no-store",
        },
      });
    }
    return NextResponse.json(await getFileHistory(input), {
      headers: { "Cache-Control": "no-store" },
    });
  } catch (error) {
    return errorResponse(error);
  }
}
