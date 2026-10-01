import { NextRequest, NextResponse } from "@/lib/web-request";
import {
  assignmentBytes,
  probeAssignmentFiles,
  assignmentMediaType,
  listAssignmentFiles,
  listAssignmentDirectory,
  assignmentPathRevision,
  readAssignmentFile,
  writeAssignmentFile,
  editAssignmentFile,
  createAssignmentDirectory,
  deleteAssignmentPath,
  moveAssignmentPath,
  reorderAssignmentFiles,
} from "@/lib/assignment-files";
import { errorResponse, jsonBody, localRequest } from "@/lib/http";

export const runtime = "nodejs";
export async function GET(request: NextRequest) {
  try {
    localRequest(request, false, { allowDocumentNavigation: true });
    const query = request.nextUrl.searchParams;
    const input = {
      courseId: query.get("courseId"),

      path: query.get("path"),
    };
    if (query.get("mode") === "directory") {
      const timings: Record<string, number> = {};
      const listing = await listAssignmentDirectory(
        {
          ...input,
          directory: query.get("directory") || undefined,
        },
        timings,
      );
      return NextResponse.json(listing, {
        headers: {
          "Cache-Control": "no-store",
          "Server-Timing": Object.entries(timings)
            .map(([name, duration]) => `${name};dur=${duration.toFixed(2)}`)
            .join(", "),
        },
      });
    }
    if (query.get("mode") === "revision")
      return NextResponse.json(await assignmentPathRevision(input), {
        headers: { "Cache-Control": "no-store" },
      });
    if (query.get("mode") === "probe")
      return NextResponse.json(await probeAssignmentFiles(input), {
        headers: { "Cache-Control": "no-store" },
      });
    if (query.get("mode") === "asset") {
      const file = await assignmentBytes(input);
      const type = assignmentMediaType(file.path);
      const download = query.get("download") === "1";
      return new NextResponse(new Uint8Array(file.bytes), {
        headers: {
          "Content-Type": type.startsWith("image/")
            ? type
            : "text/plain; charset=utf-8",
          "Content-Disposition": `${download ? "attachment" : "inline"}; filename*=UTF-8''${encodeURIComponent(file.path.split("/").pop()!)}`,
          "Content-Security-Policy": "default-src 'none'; sandbox",
          "X-Content-Type-Options": "nosniff",
          "Cache-Control": "no-store",
        },
      });
    }
    return NextResponse.json(
      query.has("path")
        ? await readAssignmentFile(input)
        : await listAssignmentFiles(input),
      { headers: { "Cache-Control": "no-store" } },
    );
  } catch (error) {
    return errorResponse(error);
  }
}
export async function POST(request: NextRequest) {
  try {
    localRequest(request, true);
    const { action, data: raw } = await jsonBody(request, 18_000_000);
    const data = { ...raw, assignmentId: undefined };
    let result: unknown;
    switch (action) {
      case "create":
        result = await writeAssignmentFile(data, true, "user");
        break;
      case "write":
        result = await writeAssignmentFile(data, false, "user");
        break;
      case "edit":
        result = await editAssignmentFile(data, "user");
        break;
      case "mkdir":
        result = await createAssignmentDirectory(data, "user");
        break;
      case "delete":
        result = await deleteAssignmentPath(data, "user");
        break;
      case "move":
        result = await moveAssignmentPath(data, "user");
        break;
      case "reorder":
        result = await reorderAssignmentFiles(data, "user");
        break;
      default:
        throw new Error("Unknown assignment operation");
    }
    return NextResponse.json(result);
  } catch (error) {
    return errorResponse(error);
  }
}
