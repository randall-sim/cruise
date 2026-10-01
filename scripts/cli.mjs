#!/usr/bin/env node
// Keep the command stable across native checkouts and Windows-hosted WSL checkouts.
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
const home = fileURLToPath(new URL("../", import.meta.url));
const wsl =
  process.platform === "win32"
    ? home.match(/^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.*)$/i)
    : null;
const binary = path.join(
  home,
  "target/release/course-captain" +
    (process.platform === "win32" && !wsl ? ".exe" : ""),
);
if (!existsSync(binary)) {
  console.error(
    "Build the daemon first: npm run build in the daemon repository.",
  );
  process.exit(1);
}
const args = process.argv.slice(2);
const child = wsl
  ? spawn(
      "wsl.exe",
      [
        "--distribution",
        wsl[1],
        "--cd",
        "/" + wsl[2].replaceAll("\\", "/"),
        "--exec",
        "./target/release/course-captain",
        ...args,
      ],
      { stdio: "inherit", windowsHide: true },
    )
  : spawn(binary, args, {
      cwd: home,
      env: { ...process.env, COURSE_CAPTAIN_HOME: home },
      stdio: "inherit",
      windowsHide: true,
    });
child.on("error", (error) => {
  console.error(error.message);
  process.exitCode = 1;
});
child.on("exit", (code) => {
  process.exitCode = code ?? 1;
});
for (const signal of ["SIGINT", "SIGTERM"])
  process.on(signal, () => child.kill(signal));
