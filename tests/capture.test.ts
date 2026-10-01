import { test } from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { chromium } from "@playwright/test";
import { addCourse, readState } from "../src/lib/store";
const exec = promisify(execFile);
test(
  "Chrome worker reads captions and saves changing frames from two HTML5 streams",
  { timeout: 90000 },
  async () => {
    const root = await fs.mkdtemp(
      path.join(os.tmpdir(), "course-captain-capture-"),
    );
    process.env.COURSE_CAPTAIN_WORKSPACE = root;
    const course = await addCourse({ code: "CAP", name: "Capture test" });
    const port = await new Promise<number>((resolve) => {
      const s = net.createServer();
      s.listen(0, "127.0.0.1", () => {
        const port = (s.address() as net.AddressInfo).port;
        s.close(() => resolve(port));
      });
    });
    const browser = await chromium.launch({
      headless: true,
      args: [
        `--remote-debugging-port=${port}`,
        "--remote-debugging-address=127.0.0.1",
      ],
    });
    try {
      const video = await fs.readFile(
        path.resolve("tests/fixtures/lecture.webm"),
      );
      const context = await browser.newContext();
      await context.route("https://course.example/**", async (route) => {
        const url = route.request().url();
        if (url.endsWith(".webm"))
          return route.fulfill({
            status: 200,
            contentType: "video/webm",
            body: video,
          });
        if (url.endsWith(".vtt"))
          return route.fulfill({
            status: 200,
            contentType: "text/vtt",
            body: "WEBVTT\n\n00:00:00.000 --> 00:00:04.000\nThis is a test lecture with colored slides.\n",
          });
        const element =
          '<video aria-label="Test slides" preload="auto" controls width="640" height="360" src="/video.webm"><track kind="captions" default src="/captions.vtt" srclang="en" label="English"></video>';
        return route.fulfill({
          status: 200,
          contentType: "text/html",
          body:
            element +
            (url.endsWith("/lecture")
              ? '<iframe src="/frame" width="670" height="390"></iframe>'
              : ""),
        });
      });
      const page = await context.newPage();
      await page.goto("https://course.example/lecture");
      await page.locator("video").evaluate(
        (v: HTMLVideoElement) =>
          new Promise<void>((resolve) => {
            if (v.readyState >= 2) resolve();
            else
              v.addEventListener("loadeddata", () => resolve(), { once: true });
          }),
      );
      const result = await exec(
        process.execPath,
        [
          path.resolve("node_modules/tsx/dist/cli.mjs"),
          path.resolve("scripts/capture.ts"),
          "--url",
          "https://course.example/lecture",
          "--course",
          course.id,
          "--title",
          "Controlled recording",
          "--interval",
          "1",
          "--streams",
          "0,1",
        ],
        {
          env: { ...process.env, CHROME_CDP_URL: `http://127.0.0.1:${port}` },
          timeout: 60000,
        },
      );
      assert.match(result.stdout, /Saved \d+ candidate captures/);
      const lecture = (await readState()).lectures[0];
      assert.ok(lecture.captures.length >= 4);
      assert.equal(new Set(lecture.captures.map((c) => c.stream)).size, 2);
      assert.ok(lecture.cues[0].text.includes("colored slides"));
      assert.ok(
        lecture.duration > 4,
        "must capture the recording tail beyond the captions",
      );
      assert.ok(lecture.captures.some((c) => c.seconds > 4));
      assert.ok(!lecture.captureCoverage.includes("in progress"));
      assert.ok(
        !page.isClosed(),
        "worker must disconnect without closing Chrome",
      );
      assert.ok(
        (await page
          .locator("video")
          .evaluate((v: HTMLVideoElement) => v.currentTime)) < 0.2,
      );
    } finally {
      await browser.close();
      assert.ok(
        root.startsWith(path.join(os.tmpdir(), "course-captain-capture-")),
      );
      await fs.rm(root, { recursive: true, force: true });
    }
  },
);
