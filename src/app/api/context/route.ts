import { NextRequest, NextResponse } from "@/lib/web-request";
import { localRequest, errorResponse } from "@/lib/http";
import { readCourseContext } from "@/lib/course-context";

export async function GET(request: NextRequest) {
  try {
    localRequest(request);
    const origin = request.headers.get("origin");
    if (origin && origin !== request.nextUrl.origin)
      throw new Error("A same-origin request is required");
    const result = await readCourseContext(
      Object.fromEntries(request.nextUrl.searchParams),
    );
    return NextResponse.json(result, {
      headers: { "Cache-Control": "no-store" },
    });
  } catch (error) {
    return errorResponse(error);
  }
}
