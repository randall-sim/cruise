// Dependency-free launcher: Windows Codex can open a checkout stored in WSL.
// Keep stdout exclusively for the MCP protocol.
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const wsl =
  process.platform === "win32"
    ? root.match(/^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.*)$/i)
    : null;
let command = process.execPath;
let args = [
  path.join(root, "node_modules/tsx/dist/cli.mjs"),
  path.join(root, "scripts/mcp.ts"),
];
if (wsl) {
  command = "wsl.exe";
  args = [
    "--distribution",
    wsl[1],
    "--cd",
    "/" + wsl[2].replaceAll("\\", "/"),
    "--exec",
    "node",
    "node_modules/tsx/dist/cli.mjs",
    "scripts/mcp.ts",
  ];
} else if (!existsSync(args[0])) {
  console.error(
    "Course Captain dependencies are missing. Run npm ci in the repository, then reconnect MCP.",
  );
  process.exit(1);
}
const child = spawn(command, args, {
  cwd: root,
  stdio: "inherit",
  windowsHide: true,
});
child.on("error", (error) => {
  console.error(`Course Captain could not start: ${error.message}`);
  process.exitCode = 1;
});
child.on("exit", (code) => {
  process.exitCode = code ?? 1;
});
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    child.kill(signal);
  });
}
