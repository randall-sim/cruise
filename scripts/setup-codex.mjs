import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

// Run on the same OS as the Codex host. Absolute paths also work when a task
// starts in a course subdirectory. Does not edit global config or project trust.
const root = fileURLToPath(new URL("../", import.meta.url));
const dir = path.join(root, ".codex");
const file = path.join(dir, "config.toml");
const begin = "# BEGIN Course Captain managed MCP";
const end = "# END Course Captain managed MCP";
const block = `${begin}\n# Regenerate after moving the checkout: node scripts/setup-codex.mjs\n[mcp_servers.course-captain]\ncommand = ${JSON.stringify(process.execPath)}\nargs = [${JSON.stringify(path.join(root, "scripts/start-mcp.mjs"))}]\ncwd = ${JSON.stringify(root)}\nstartup_timeout_sec = 60\ntool_timeout_sec = 120\n${end}`;
mkdirSync(dir, { recursive: true });
let previous = "";
try {
  previous = readFileSync(file, "utf8");
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
if (previous.includes(begin) && previous.includes(end)) {
  previous =
    previous.slice(0, previous.indexOf(begin)) +
    block +
    previous.slice(previous.indexOf(end) + end.length);
} else {
  if (/\[mcp_servers\.(?:course-captain|"course-captain")\]/.test(previous)) {
    throw new Error(
      "Existing course-captain MCP configuration is unmanaged; review it before replacing it.",
    );
  }
  previous =
    previous.trimEnd() + (previous.trim() ? "\n\n" : "") + block + "\n";
}
writeFileSync(file, previous);
console.log(
  "Course Captain project MCP configured. Reopen this trusted repository in Codex to load its tools.",
);
