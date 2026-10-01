import type { Cue } from "./schema";

export function secondsFromTimestamp(value: string): number {
  const parts = value.replace(",", ".").split(":").map(Number);
  if (
    parts.some((v) => !Number.isFinite(v) || v < 0) ||
    parts.length < 2 ||
    parts.length > 3
  )
    throw new Error("Invalid timestamp");
  return parts.reduce((total, part) => total * 60 + part, 0);
}
export const timestamp = (seconds: number) => {
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor(total / 60) % 60;
  const s = total % 60;
  return `${h ? `${h}:` : ""}${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
};
export function parseTranscript(raw: string): Cue[] {
  const lines = raw
    .replace(/^\uFEFF/, "")
    .replace(/\r/g, "")
    .split("\n");
  const cues: Cue[] = [];
  for (let i = 0; i < lines.length; i++) {
    const match = lines[i].match(
      /((?:\d{1,3}:)?\d{2}:\d{2}[.,]\d{3})\s*-->\s*((?:\d{1,3}:)?\d{2}:\d{2}[.,]\d{3})/,
    );
    if (!match) continue;
    const start = secondsFromTimestamp(match[1]);
    const end = secondsFromTimestamp(match[2]);
    if (end < start) throw new Error("A transcript cue ends before it starts");
    const text: string[] = [];
    while (++i < lines.length && lines[i].trim()) text.push(lines[i]);
    const cleaned = text
      .join(" ")
      .replace(/<[^>]*>/g, "")
      .replace(/&amp;/g, "&")
      .replace(/&lt;/g, "<")
      .replace(/&gt;/g, ">")
      .trim();
    if (cleaned) cues.push({ start, end, text: cleaned });
  }
  if (!cues.length)
    throw new Error(
      "No timestamped cues found. Import a WebVTT (.vtt) or SubRip (.srt) transcript.",
    );
  if (cues.length > 30000) throw new Error("Transcript exceeds 30,000 cues");
  return cues.sort((a, b) => a.start - b.start);
}
