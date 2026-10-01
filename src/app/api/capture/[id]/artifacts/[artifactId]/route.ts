import { NextRequest, NextResponse } from "@/lib/web-request";
import { promises as fs } from "node:fs";
import { readState, safePath, assertNoSymlink } from "@/lib/store";
import { localRequest, errorResponse } from "@/lib/http";

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string; artifactId: string }> },
) {
  try {
    localRequest(request, false, { allowDocumentNavigation: true });
    const { id, artifactId } = await params;
    const capture = (await readState()).lectures
      .flatMap((l) => l.captures)
      .find((c) => c.id === id);
    const artifact = capture?.artifacts?.find((a) => a.id === artifactId);
    if (!artifact) return new NextResponse("Not found", { status: 404 });
    const source = request.nextUrl.searchParams.get("view") === "source";
    if (source && !artifact.definitionPath)
      return new NextResponse("No diagram source", { status: 404 });
    const file = safePath(source ? artifact.definitionPath! : artifact.file);
    await assertNoSymlink(file);
    return new NextResponse(await fs.readFile(file), {
      headers: {
        "Content-Type": source
          ? "text/plain; charset=utf-8"
          : artifact.format === "diagram"
            ? "image/svg+xml"
            : "image/png",
        "Cache-Control": "private, no-store",
        "X-Content-Type-Options": "nosniff",
        "Content-Security-Policy": "default-src 'none'; sandbox",
      },
    });
  } catch (error) {
    return errorResponse(error);
  }
}
