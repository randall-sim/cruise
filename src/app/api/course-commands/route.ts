import { NextRequest, NextResponse } from "@/lib/web-request";
import { z } from "zod";
import { errorResponse, jsonBody, localRequest } from "@/lib/http";
import {
  listCourseCommands,
  readCourseCommand,
  startCourseCommand,
  stopCourseCommand,
} from "@/lib/course-terminal";
export const runtime = "nodejs";
export async function GET(request: NextRequest) {
  try {
    localRequest(request);
    const q = request.nextUrl.searchParams;
    return NextResponse.json(
      q.has("runId")
        ? await readCourseCommand({
            courseId: q.get("courseId"),
            runId: q.get("runId"),
            offset: q.has("offset") ? Number(q.get("offset")) : 0,
          })
        : await listCourseCommands(z.string().parse(q.get("courseId"))),
      { headers: { "Cache-Control": "no-store" } },
    );
  } catch (e) {
    return errorResponse(e);
  }
}
export async function POST(request: NextRequest) {
  try {
    localRequest(request, true);
    const { action, data } = await jsonBody(request, 20000);
    if (action !== "start" && action !== "stop")
      throw new Error("Unknown command operation");
    return NextResponse.json(
      action === "start"
        ? await startCourseCommand(data)
        : await stopCourseCommand(data),
    );
  } catch (e) {
    return errorResponse(e);
  }
}
