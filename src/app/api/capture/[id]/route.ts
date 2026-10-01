import { NextRequest, NextResponse } from "@/lib/web-request";
import { promises as fs } from "node:fs";
import { assertNoSymlink, readState, safePath } from "@/lib/store";
import { localRequest, errorResponse } from "@/lib/http";
export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> },
) {
  try {
    localRequest(request);
    const { id } = await params;
    const capture = (await readState()).lectures
      .flatMap((l) => l.captures)
      .find((c) => c.id === id);
    if (!capture) return new NextResponse("Not found", { status: 404 });
    const file = safePath(capture.file);
    await assertNoSymlink(file);
    return new NextResponse(await fs.readFile(file), {
      headers: {
        "Content-Type": "image/png",
        "Cache-Control": "private, no-store",
      },
    });
  } catch (error) {
    return errorResponse(error);
  }
}
