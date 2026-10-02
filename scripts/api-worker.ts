// Private stdin/stdout transport owned by the Rust process; no listening socket.
import { createInterface } from "node:readline";
import { NextRequest } from "../src/lib/web-request";
import * as workspace from "../src/app/api/workspace/route";
import * as action from "../src/app/api/action/route";
import * as assignments from "../src/app/api/assignments/route";
import * as files from "../src/app/api/course-files/route";
import * as timeline from "../src/app/api/assignment-timeline/route";
import * as history from "../src/app/api/file-history/route";
import * as commands from "../src/app/api/course-commands/route";
import * as context from "../src/app/api/context/route";
import * as explorer from "../src/app/api/file-explorer/route";
import * as capture from "../src/app/api/capture/[id]/route";
import * as artifact from "../src/app/api/capture/[id]/artifacts/[artifactId]/route";

type Handler = (request: NextRequest) => Promise<Response>;
const routes: Record<string, { GET?: Handler; POST?: Handler }> = {
  "/api/workspace": workspace,
  "/api/action": action,
  "/api/assignments": assignments,
  "/api/course-files": files,
  "/api/assignment-timeline": timeline,
  "/api/file-history": history,
  "/api/course-commands": commands,
  "/api/context": context,
  "/api/file-explorer": explorer,
};
for await (const line of createInterface({ input: process.stdin })) {
  let response: Response;
  try {
    const wire = JSON.parse(line);
    const url = new URL(wire.url, "http://127.0.0.1");
    const request = new NextRequest(url, {
      method: wire.method,
      headers: {
        host: "127.0.0.1",
        origin: "http://127.0.0.1",
        "content-type": wire.contentType || "application/json",
      },
      ...(wire.method !== "GET"
        ? { body: Buffer.from(wire.body, "base64") }
        : {}),
    });
    const image = url.pathname.match(
      /^\/api\/capture\/([a-zA-Z0-9_-]+)(?:\/artifacts\/([a-zA-Z0-9_-]+))?$/,
    );
    if (image && wire.method === "GET") {
      response = image[2]
        ? await artifact.GET(request, {
            params: Promise.resolve({ id: image[1], artifactId: image[2] }),
          })
        : await capture.GET(request, {
            params: Promise.resolve({ id: image[1] }),
          });
    } else {
      const handler = routes[url.pathname]?.[wire.method as "GET" | "POST"];
      response = handler
        ? await handler(request)
        : Response.json({ error: "Unknown API route" }, { status: 404 });
    }
  } catch (error) {
    response = Response.json(
      { error: error instanceof Error ? error.message : "Request failed" },
      { status: 500 },
    );
  }
  process.stdout.write(
    JSON.stringify({
      status: response.status,
      headers: Object.fromEntries(response.headers),
      body: Buffer.from(await response.arrayBuffer()).toString("base64"),
    }) + "\n",
  );
}
