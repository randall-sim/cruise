import { NextRequest, NextResponse } from "@/lib/web-request";
import { ZodError } from "zod";

export function localRequest(
  request: NextRequest,
  mutation = false,
  options: { allowDocumentNavigation?: boolean } = {},
) {
  const host = request.headers.get("host")?.split(":")[0];
  if (!host || !["localhost", "127.0.0.1"].includes(host))
    throw new Error("This service accepts loopback requests only");
  const origin = request.headers.get("origin");
  if (mutation && origin !== `http://${request.headers.get("host")}`)
    throw new Error("A same-origin browser request is required");
  const site = request.headers.get("sec-fetch-site");
  // A browser extension can open a local page as a cross-site top-level
  // navigation. Allow that only for reader pages, never APIs or mutations.
  const documentNavigation =
    !mutation &&
    options.allowDocumentNavigation &&
    request.headers.get("sec-fetch-mode") === "navigate" &&
    request.headers.get("sec-fetch-dest") === "document";
  if (site === "cross-site" && !documentNavigation)
    throw new Error("Cross-site requests are not allowed");
}
export async function jsonBody(request: NextRequest, maxBytes = 3_000_000) {
  if (!request.headers.get("content-type")?.startsWith("application/json"))
    throw new Error("Expected application/json");
  const reader = request.body?.getReader();
  if (!reader) throw new Error("Missing request body");
  const chunks: Uint8Array[] = [];
  let length = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    length += value.length;
    if (length > maxBytes) {
      await reader.cancel();
      throw new Error("Request body is too large");
    }
    chunks.push(value);
  }
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}
export function errorResponse(error: unknown) {
  return NextResponse.json(
    {
      error:
        error instanceof ZodError
          ? error.issues
              .map((i) => `${i.path.join(".")}: ${i.message}`)
              .join("; ")
          : error instanceof Error
            ? error.message
            : "Request failed",
    },
    { status: 400 },
  );
}
