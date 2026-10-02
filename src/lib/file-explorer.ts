import childProcess from "node:child_process";
import { promises as fs } from "node:fs";
import { release } from "node:os";
import path from "node:path";
import { z } from "zod";
import { idSchema } from "./schema";
import { assignmentPath } from "./assignment-schema";
import { listAssignmentDirectory } from "./assignment-files";
import { resolveAssignmentPath } from "./assignment-references";
import { readAssignmentSnapshot } from "./assignment-timeline";
import { assertNoSymlink, safePath } from "./store";
import { excludedCoursePath } from "./file-policy";

export async function resolveExplorerTarget(input: unknown) {
  const data = z
    .object({
      courseId: idSchema,
      assignmentId: idSchema.optional(),
      stepId: idSchema.optional(),
      path: assignmentPath,
    })
    .strict()
    .parse(input);
  let target: string | undefined;
  if (data.stepId) {
    const file = await readAssignmentSnapshot({ ...data, mode: "file" });
    if ("absolutePath" in file) target = file.absolutePath;
  } else {
    const directory = data.path.split("/").slice(0, -1).join("/");
    const tree = await listAssignmentDirectory({
      ...data,
      directory: directory || undefined,
    });
    if (tree.entries.some((entry) => entry.path === data.path))
      target = (
        await resolveAssignmentPath(
          data.courseId,
          data.assignmentId,
          data.path,
          true,
        )
      ).absolutePath;
  }
  if (!target) throw new Error("File not found in this workspace view.");
  const root = safePath(`courses/${data.courseId}/files`);
  const relative = path.relative(root, target);
  if (
    relative.startsWith("..") ||
    path.isAbsolute(relative) ||
    excludedCoursePath(relative.split(path.sep).join("/"))
  )
    throw new Error("Path is outside the available course files.");
  await assertNoSymlink(target);
  let missing = false;
  while (true) {
    try {
      const stat = await fs.stat(target);
      if (!stat.isFile() && !stat.isDirectory())
        throw new Error("Not a regular file or folder.");
      return { path: target, directory: stat.isDirectory(), missing };
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT" || target === root)
        throw error;
      target = path.dirname(target);
      missing = true;
    }
  }
}

function run(command: string, args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    childProcess.execFile(
      command,
      args,
      { timeout: 8000, windowsHide: true },
      (error, stdout) => {
        if (error) reject(error);
        else resolve(stdout);
      },
    );
  });
}

export async function fileExplorerCommand(
  target: { path: string; directory: boolean },
  platform: NodeJS.Platform = process.platform,
  wsl = platform === "linux" &&
    (Boolean(process.env.WSL_DISTRO_NAME) || /microsoft/i.test(release())),
) {
  if (platform === "win32" || wsl) {
    const filename = wsl
      ? (await run("wslpath", ["-w", target.path])).trim()
      : target.path;
    if (!path.win32.isAbsolute(filename) || /[\r\n\0]/.test(filename))
      throw new Error("Cannot convert this file path for Windows Explorer.");
    return {
      command: wsl
        ? "explorer.exe"
        : path.win32.join(
            process.env.SystemRoot || "C:\\Windows",
            "explorer.exe",
          ),
      args: target.directory ? [filename] : ["/select,", filename],
      wait: false,
    };
  }
  if (platform === "darwin")
    return {
      command: "/usr/bin/open",
      args: target.directory ? [target.path] : ["-R", target.path],
      wait: true,
    };
  if (platform === "linux")
    return {
      command: "xdg-open",
      args: [target.directory ? target.path : path.dirname(target.path)],
      wait: true,
    };
  throw new Error(
    "Opening a file explorer is supported on Windows, macOS, and Linux.",
  );
}

export async function openFileExplorer(input: unknown) {
  const target = await resolveExplorerTarget(input);
  try {
    const { command, args, wait } = await fileExplorerCommand(target);
    if (wait) await run(command, args);
    else
      await new Promise<void>((resolve, reject) => {
        // Explorer may keep running or return a nonzero status after handing off
        // to an existing window. Confirm launch without waiting for it to close.
        const child = childProcess.spawn(command, args, {
          stdio: "ignore",
          windowsHide: true,
          detached: true,
        });
        child.once("error", reject);
        child.once("spawn", () => {
          child.unref();
          resolve();
        });
      });
  } catch (error) {
    throw new Error(
      `Could not open the file explorer on the daemon's computer. Ensure a desktop file manager is available. ${(error as Error).message}`,
    );
  }
  return { missing: target.missing };
}
