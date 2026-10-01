import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { promises as fs } from "node:fs";
import path from "node:path";
import net from "node:net";
import { once } from "node:events";

test(
  "Rust CLI serves the course API, validates origins, and protects file reads and changes",
  { timeout: 30000 },
  async () => {
    const root = await fs.mkdtemp(path.resolve("workspace-test-api-"));
    const reservation = net.createServer().listen(0, "127.0.0.1");
    await once(reservation, "listening");
    const port = (reservation.address() as net.AddressInfo).port;
    await new Promise<void>((resolve) => reservation.close(() => resolve()));
    const child = spawn(
      path.resolve(
        "target/debug/course-captain" +
          (process.platform === "win32" ? ".exe" : ""),
      ),
      [
        "run",
        "--port",
        String(port),
        "--workspace",
        root,
        "--origin",
        "https://frontend.example",
      ],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
    let stderr = "";
    child.stderr.on("data", (data) => {
      stderr += data;
    });
    try {
      const token = await new Promise<string>((resolve, reject) => {
        let stdout = "";
        child.stdout.on("data", (chunk) => {
          stdout += chunk;
          const match = stdout.match(/Connection key: ([a-f0-9]+)/);
          if (match) resolve(match[1]);
        });
        child.on("error", reject);
        child.on("exit", (code) =>
          reject(new Error(`Backend exited (${code}): ${stderr}`)),
        );
      });
      const base = `http://127.0.0.1:${port}`;
      const headers = {
        origin: "https://frontend.example",
        authorization: `Bearer ${token}`,
        "content-type": "application/json",
      };
      assert.equal((await fetch(base + "/api/workspace")).status, 401);
      assert.equal(
        (
          await fetch(base + "/api/workspace", {
            headers: { ...headers, origin: "https://evil.example" },
          })
        ).status,
        403,
      );
      const preflight = await fetch(base + "/api/action", {
        method: "OPTIONS",
        headers: {
          origin: headers.origin,
          "access-control-request-method": "POST",
          "access-control-request-headers": "authorization,content-type",
        },
      });
      assert.equal(preflight.status, 204);
      assert.equal(
        preflight.headers.get("access-control-allow-origin"),
        headers.origin,
      );
      const created = await fetch(base + "/api/action", {
        method: "POST",
        headers,
        body: JSON.stringify({
          action: "course.create",
          data: { code: "LOCAL", name: "Local API check", term: "Fall 2026" },
        }),
      });
      assert.equal(created.status, 200, await created.clone().text());
      const { result: course } = await created.json();
      const state = await (
        await fetch(base + "/api/workspace", { headers })
      ).json();
      assert.equal(state.courses[0].id, course.id);
      const file = await fetch(base + "/api/course-files", {
        method: "POST",
        headers,
        body: JSON.stringify({
          action: "create",
          data: {
            courseId: course.id,
            path: "hello.txt",
            content: "Local bytes only",
            explanation: "Verify the local API preserves tracked file writes.",
          },
        }),
      });
      assert.equal(file.status, 200, await file.clone().text());
      const query = new URLSearchParams({
        courseId: course.id,
        path: "hello.txt",
        mode: "asset",
        download: "1",
      });
      const download = await fetch(base + "/api/course-files?" + query, {
        headers,
      });
      assert.equal(await download.text(), "Local bytes only");
      assert.match(
        download.headers.get("content-disposition") || "",
        /attachment/,
      );
      const context = await fetch(base + "/api/context?courseId=" + course.id, {
        headers,
      });
      assert.equal(context.status, 200, await context.clone().text());
      assert.equal((await context.json()).course.id, course.id);
      const traversal = await fetch(
        base +
          "/api/course-files?" +
          new URLSearchParams({
            courseId: course.id,
            path: "../../state.json",
            mode: "asset",
          }),
        { headers },
      );
      assert.equal(traversal.status, 400);
    } finally {
      if (child.exitCode === null && child.pid) {
        child.kill("SIGTERM");
        await once(child, "exit");
      }
      await fs.rm(root, { recursive: true, force: true });
    }
  },
);
