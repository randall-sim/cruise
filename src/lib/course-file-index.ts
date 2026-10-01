import { promises as fs } from "node:fs";
import { createHash } from "node:crypto";
import { excludedCoursePath } from "./file-policy";
import { assertNoSymlink, safePath, readState, requireCourse } from "./store";
import type { Citation } from "./schema";

export async function indexCourseFiles(courseId: string) {
  requireCourse(await readState(), courseId);
  const base = `courses/${courseId}/files`;
  const evidence: Citation[] = [];
  const skipped: { path: string; reason: string }[] = [];
  let files = 0,
    bytes = 0,
    visited = 0;
  async function walk(relative: string, depth: number) {
    const target = safePath(relative ? `${base}/${relative}` : base);
    await assertNoSymlink(target);
    let entries;
    try {
      entries = await fs.readdir(target, { withFileTypes: true });
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code === "ENOENT") return;
      throw e;
    }
    for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
      const name = relative ? `${relative}/${entry.name}` : entry.name;
      if (excludedCoursePath(name)) continue;
      if (entry.isSymbolicLink()) {
        skipped.push({ path: name, reason: "Symlink" });
        continue;
      }
      if (++visited > 20000 || depth > 20) {
        skipped.push({ path: name, reason: "Index traversal limit" });
        return;
      }
      if (entry.isDirectory()) {
        await walk(name, depth + 1);
        continue;
      }
      if (!entry.isFile()) continue;
      const full = safePath(`${base}/${name}`);
      await assertNoSymlink(full);
      const stat = await fs.stat(full);
      if (stat.size > 2_000_000 || bytes + stat.size > 30_000_000) {
        skipped.push({
          path: name,
          reason: "Text index size limit (2 MB/file, 30 MB/course)",
        });
        continue;
      }
      const buffer = await fs.readFile(full);
      let content: string;
      try {
        content = new TextDecoder("utf-8", { fatal: true }).decode(buffer);
        if (content.includes("\0")) throw new Error("binary");
      } catch {
        skipped.push({
          path: name,
          reason: "Binary; add a Markdown description or extracted text",
        });
        continue;
      }
      if (!content.trim()) continue;
      bytes += buffer.length;
      files++;
      const version = createHash("sha256")
        .update(buffer)
        .digest("hex")
        .slice(0, 16);
      for (let start = 0; start < content.length; start += 5500) {
        const text = content.slice(start, start + 6000);
        const line = content.slice(0, start).split("\n").length;
        evidence.push({
          id: `file:${base}/${name}:${version}:${start}`,
          courseId,
          kind: "file",
          title: `${name} · line ${line}`,
          text,
          path: `${base}/${name}`,
          url: `/?${new URLSearchParams({ course: courseId, tab: "files", file: name })}`,
        });
      }
    }
  }
  await walk("", 0);
  return {
    evidence,
    files,
    bytes,
    skipped,
    excluded:
      "VCS metadata, dependencies, build output, caches and credential files",
    provenance:
      "Local working files, including imported and generated code; not automatically instructor authority. Source IDs include a content revision.",
  };
}
