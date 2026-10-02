import { NextRequest, NextResponse } from "@/lib/web-request";
import { errorResponse, jsonBody, localRequest } from "@/lib/http";
import { openFileExplorer } from "@/lib/file-explorer";

export async function POST(request: NextRequest) {
  try {
    localRequest(request, true);
    return NextResponse.json(
      await openFileExplorer(await jsonBody(request, 4096)),
    );
  } catch (error) {
    return errorResponse(error);
  }
}
