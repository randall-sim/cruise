/** User-started Chrome worker. Never launches a browser, types credentials, clicks arbitrary controls, or submits forms. */
import { chromium, type Page, type Locator } from "@playwright/test";
import { parseArgs } from "node:util";
import { promises as fs } from "node:fs";
import sharp from "sharp";
import nextEnv from "@next/env";
import {
  readState,
  requireCourse,
  mutate,
  writePrivate,
} from "../src/lib/store";
import { importLecture, addCapture } from "../src/lib/lectures";
nextEnv.loadEnvConfig(process.cwd());
const { values: args } = parseArgs({
  options: {
    url: { type: "string" },
    course: { type: "string" },
    title: { type: "string" },
    date: { type: "string" },
    transcript: { type: "string" },
    streams: { type: "string" },
    interval: { type: "string", default: "3" },
    kind: { type: "string", default: "slide" },
    list: { type: "boolean", default: false },
  },
});
if (!args.url)
  throw new Error(
    "Pass --url with the exact URL of your open lecture tab. Use --list first to inspect streams.",
  );
const endpoint = new URL(process.env.CHROME_CDP_URL || "http://127.0.0.1:9222");
if (!["localhost", "127.0.0.1", "[::1]"].includes(endpoint.hostname))
  throw new Error("Chrome CDP must use a loopback endpoint");
const interval = Number(args.interval);
if (!Number.isFinite(interval) || interval < 1 || interval > 30)
  throw new Error("--interval must be between 1 and 30 seconds");
if (!["slide", "whiteboard", "demo"].includes(args.kind!))
  throw new Error("--kind must be slide, whiteboard or demo");
const browser = await chromium.connectOverCDP(endpoint.href);
type VideoEntry = {
  locator: Locator;
  page: Page;
  frameUrl: string;
  label: string;
  duration: number;
  time: number;
  paused: boolean;
  muted: boolean;
  trackModes: TextTrackMode[];
};
const videos: VideoEntry[] = [];
let lectureId: string | undefined;
try {
  const page = browser
    .contexts()
    .flatMap((c) => c.pages())
    .find((p) => p.url() === args.url);
  if (!page)
    throw new Error(
      "No open tab matches --url exactly. Sign in and open the lecture in the dedicated Chrome profile first.",
    );
  for (const frame of page.frames()) {
    const locators = await frame.locator("video").all();
    for (const locator of locators) {
      const metadata = await locator.evaluate((v: HTMLVideoElement) => ({
        duration: v.duration,
        time: v.currentTime,
        paused: v.paused,
        muted: v.muted,
        trackModes: Array.from(v.textTracks, (t) => t.mode),
        label: v.getAttribute("aria-label") || v.title || "HTML5 video",
      }));
      videos.push({ locator, page, frameUrl: frame.url(), ...metadata });
    }
  }
  console.log(
    JSON.stringify(
      videos.map((v, i) => ({
        index: i,
        label: v.label,
        frame: v.frameUrl,
        duration: v.duration,
      })),
      null,
      2,
    ),
  );
  if (!videos.length)
    throw new Error(
      "No HTML5 video streams found. This player needs a provider adapter or a downloaded video/transcript workflow.",
    );
  if (!args.list) {
    if (!args.course || !args.title)
      throw new Error("Pass --course <id> and --title <lecture name>");
    requireCourse(await readState(), args.course);
    const indices = args.streams
      ? args.streams.split(",").map(Number)
      : videos.map((_, i) => i);
    if (
      indices.some((i) => !Number.isInteger(i) || !videos[i]) ||
      new Set(indices).size !== indices.length
    )
      throw new Error("Invalid stream indices");
    const chosen = indices.map((i) => videos[i]);
    if (
      chosen.some(
        (v) =>
          !Number.isFinite(v.duration) || v.duration <= 0 || v.duration > 14400,
      )
    )
      throw new Error(
        "Streams must be seekable recordings shorter than four hours. Load the recording first.",
      );
    let transcript = args.transcript
      ? await fs.readFile(args.transcript, "utf8")
      : "";
    if (!transcript) {
      // Loading a native text track uses the already authenticated page. No cookie/token export.
      for (const v of chosen) {
        await v.locator.evaluate((video: HTMLVideoElement) => {
          for (const track of Array.from(video.textTracks))
            track.mode = "hidden";
        });
        try {
          await v.page.waitForFunction(
            () =>
              Array.from(document.querySelectorAll("video")).some((v) =>
                Array.from(v.textTracks).some((t) => t.cues?.length),
              ),
            {},
            { timeout: 2500 },
          );
        } catch {
          /* iframe/native tracks may be unavailable */
        }
        const cues = await v.locator.evaluate((video: HTMLVideoElement) =>
          Array.from(video.textTracks)
            .filter((t) => t.kind === "captions" || t.kind === "subtitles")
            .flatMap((t) =>
              Array.from(t.cues || [], (c) => ({
                start: c.startTime,
                end: c.endTime,
                text: (c as VTTCue).text || "",
              })),
            ),
        );
        if (cues.length) {
          const time = (s: number) =>
            new Date(s * 1000).toISOString().slice(11, 23);
          transcript =
            "WEBVTT\n\n" +
            cues
              .map((c) => `${time(c.start)} --> ${time(c.end)}\n${c.text}`)
              .join("\n\n");
          break;
        }
      }
    }
    if (!transcript)
      throw new Error(
        "No native caption track is accessible. Download a VTT/SRT using the player, then pass --transcript <path>. No lecture was created.",
      );
    const coverage = `Sampled ${chosen.length} HTML5 stream(s) every ${interval}s. Visual changes are candidate captures, not a guarantee that every slide or final drawing was captured. Hidden/provider-specific streams and brief events between samples require manual review.`;
    const lecture = await importLecture({
      courseId: args.course,
      title: args.title,
      date: args.date || new Date().toISOString().slice(0, 10),
      sourceUrl: args.url,
      transcript,
      captureCoverage: `Capture in progress. ${coverage}`,
    });
    lectureId = lecture.id;
    // Caption tracks often end before the final slide. Capture the full recording.
    lecture.duration = Math.max(
      lecture.duration,
      ...chosen.map((v) => v.duration),
    );
    await mutate((state) => {
      state.lectures.find((l) => l.id === lecture.id)!.duration =
        lecture.duration;
    });
    const manifest: { stream: number; seconds: number; reason: string }[] = [];
    for (const [position, v] of chosen.entries()) {
      await v.locator.evaluate((video: HTMLVideoElement) => {
        video.pause();
        video.muted = true;
      });
      let previous: Buffer | undefined;
      let previousPng: Buffer | undefined;
      let previousTime = 0;
      let lastSaved = -100;
      let lastSavedTime = -1;
      const save = async (png: Buffer, seconds: number, reason: string) => {
        if (seconds === lastSavedTime) return;
        await addCapture({
          lectureId: lecture.id,
          seconds,
          stream: `Stream ${indices[position]} · ${v.label}`,
          kind: args.kind,
          image: `data:image/png;base64,${png.toString("base64")}`,
          caption: `Candidate capture (${reason}); visual review required.`,
        });
        manifest.push({ stream: indices[position], seconds, reason });
        lastSavedTime = seconds;
        lastSaved = seconds;
      };
      const end = Math.min(v.duration - 0.1, lecture.duration);
      for (let time = 0; time <= end; time = Math.min(time + interval, end)) {
        await v.locator.evaluate(
          (video: HTMLVideoElement, t) =>
            new Promise<void>((resolve, reject) => {
              if (
                Math.abs(video.currentTime - t) < 0.05 &&
                video.readyState >= 2
              ) {
                resolve();
                return;
              }
              const timer = setTimeout(() => {
                reject(new Error("Video seek timed out"));
              }, 15000);
              video.addEventListener(
                "seeked",
                () => {
                  clearTimeout(timer);
                  resolve();
                },
                { once: true },
              );
              video.currentTime = t;
            }),
          time,
        );
        await v.locator.evaluate(
          () =>
            new Promise<void>((r) =>
              requestAnimationFrame(() => requestAnimationFrame(() => r())),
            ),
        );
        const png = await v.locator.screenshot({ type: "png", timeout: 10000 });
        const pixels = await sharp(png)
          .resize(96, 54, { fit: "fill" })
          .greyscale()
          .raw()
          .toBuffer();
        const difference = previous
          ? pixels.reduce((sum, p, i) => sum + Math.abs(p - previous![i]), 0) /
            (pixels.length * 255)
          : 1;
        if (difference > 0.045) {
          // Also preserve the last state before a change, useful for completed board diagrams.
          if (previousPng && previousTime > lastSavedTime)
            await save(previousPng, previousTime, "before transition");
          await save(png, time, "visual change");
        } else if (time - lastSaved >= 60 || time === end)
          await save(png, time, "periodic/final checkpoint");
        previous = pixels;
        previousPng = png;
        previousTime = time;
        if (time === end) break;
      }
    }
    await writePrivate(
      `courses/${args.course}/lectures/${lecture.id}/capture-manifest.json`,
      JSON.stringify(
        { coverage, interval, streams: indices, captures: manifest },
        null,
        2,
      ),
    );
    await mutate((state) => {
      state.lectures.find((l) => l.id === lecture.id)!.captureCoverage =
        coverage;
    });
    console.log(
      `Saved ${manifest.length} candidate captures. Review lecture ${lecture.id} in Course Captain, then queue its guide.`,
    );
  }
} catch (error) {
  if (lectureId)
    await mutate((state) => {
      const lecture = state.lectures.find((l) => l.id === lectureId);
      if (lecture)
        lecture.captureCoverage = `Partial capture: ${(error as Error).message}. Review the saved images and retry missing streams.`;
    });
  throw error;
} finally {
  for (const v of videos)
    try {
      await v.locator.evaluate(
        (video: HTMLVideoElement, s) => {
          video.currentTime = s.time;
          video.muted = s.muted;
          Array.from(video.textTracks).forEach((t, i) => {
            t.mode = s.trackModes[i] || "disabled";
          });
          if (!s.paused) void video.play().catch(() => {});
        },
        {
          time: v.time,
          paused: v.paused,
          muted: v.muted,
          trackModes: v.trackModes,
        },
      );
    } catch {
      /* closed/unloaded tab */
    }
  // For a CDP-connected browser this disconnects the client; it does not close Chrome.
  await browser.close();
}
