// A detached worker gives every MCP/UI caller the same durable run handle.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { checkpointCourseFiles } from "../src/lib/file-history.ts";
import { showCreatedAssignmentPath } from "../src/lib/assignment-storage.ts";
import { mutate } from "../src/lib/store.ts";
import { excludedCoursePath } from "../src/lib/file-policy.ts";
const requestPath = process.argv[2];
const request = JSON.parse(fs.readFileSync(requestPath, "utf8"));
const directory = path.dirname(requestPath);
let run = JSON.parse(fs.readFileSync(path.join(directory, "run.json"), "utf8"));
function persist() {
  const target = path.join(directory, "run.json");
  fs.writeFileSync(target + ".tmp", JSON.stringify(run));
  fs.renameSync(target + ".tmp", target);
}
run = {
  ...run,
  status: "running",
  startedAt: new Date().toISOString(),
  pid: process.pid,
};
persist();
try {
  run.historyBefore = await checkpointCourseFiles(run.courseId, {
    actor: "external",
    explanation:
      "State observed before a tracked command. Earlier changes and their context are unknown.",
    runId: run.id,
    assignmentId: run.assignmentId,
    tool: "command_before",
  });
  persist();
} catch (error) {
  run = {
    ...run,
    status: "failed",
    finishedAt: new Date().toISOString(),
    error: `Could not record pre-command history: ${error.message}`,
  };
  persist();
  process.exit(1);
}
const windows = process.platform === "win32";
const executable =
  request.invocation?.executable || (windows ? "powershell.exe" : "/bin/bash");
const args =
  request.invocation?.args ||
  (windows
    ? ["-NoProfile", "-NonInteractive", "-Command", request.command]
    : ["--noprofile", "--norc", "-c", request.command]);
const child = spawn(executable, args, {
  cwd: request.cwd,
  env: request.env,
  detached: !windows,
  stdio: ["ignore", "pipe", "pipe"],
  windowsHide: true,
});
let bytes = 0,
  finished = false,
  reason,
  forceTimer;
const log = path.join(directory, "output.log");
function append(chunk) {
  const buffer = Buffer.from(chunk);
  const available = Math.max(0, 2_000_000 - bytes);
  if (buffer.length > available) run.truncated = true;
  if (available) {
    const kept = buffer.subarray(0, available);
    fs.appendFileSync(log, kept);
    bytes += kept.length;
  }
}
child.stdout.on("data", append);
child.stderr.on("data", append);
function killTree(force = false) {
  if (!child.pid) return;
  try {
    if (windows) {
      spawn(
        "taskkill",
        ["/PID", String(child.pid), "/T", ...(force ? ["/F"] : [])],
        { stdio: "ignore", windowsHide: true },
      );
    } else process.kill(-child.pid, force ? "SIGKILL" : "SIGTERM");
  } catch (error) {
    if (error.code !== "ESRCH") append(`\nProcess cleanup: ${error.message}\n`);
  }
}
function stop(status) {
  if (finished || reason) return;
  reason = status;
  killTree();
  forceTimer = setTimeout(() => killTree(true), 1500);
}
const deadline = setTimeout(
  () => stop("timed_out"),
  request.timeoutSeconds * 1000,
);
const heartbeat = setInterval(() => {
  run.heartbeat = new Date().toISOString();
  persist();
  if (fs.existsSync(path.join(directory, "cancel"))) stop("cancelled");
}, 300);
process.on("SIGTERM", () => stop("cancelled"));
process.on("SIGINT", () => stop("cancelled"));
async function finish(code, signal, error) {
  if (finished) return;
  finished = true;
  clearInterval(heartbeat);
  clearTimeout(deadline);
  clearTimeout(forceTimer);
  killTree(true);
  try {
    if (
      run.assignmentId &&
      request.assignmentRoot &&
      fs.existsSync(request.assignmentRoot)
    ) {
      await mutate(async () => {
        for (const entry of fs.readdirSync(request.assignmentRoot, {
          withFileTypes: true,
        })) {
          if (
            !request.assignmentRootsBefore.includes(entry.name) &&
            !entry.isSymbolicLink() &&
            !excludedCoursePath(entry.name) &&
            (entry.isFile() || entry.isDirectory())
          ) {
            await showCreatedAssignmentPath(
              run.courseId,
              run.assignmentId,
              entry.name,
            );
          }
        }
      });
    }
    run.historyAfter = await checkpointCourseFiles(run.courseId, {
      actor: "terminal",
      explanation: run.purpose,
      command: run.command,
      runId: run.id,
      assignmentId: run.assignmentId,
      tool: "command_after",
      context: `Observed net changes across this command (exit ${code ?? "none"}, ${reason || "finished"}). Concurrent external changes may also be present; intermediate writes were not captured.`,
    });
  } catch (historyError) {
    run.error = `Command finished but its history checkpoint failed: ${historyError.message}`;
  }
  run = {
    ...run,
    status: reason || (code === 0 && !error ? "completed" : "failed"),
    exitCode: code,
    signal,
    finishedAt: new Date().toISOString(),
    ...(error ? { error: error.message } : {}),
  };
  let output = "";
  try {
    output = fs.readFileSync(log, "utf8");
  } catch {}
  const summary = `# Command: ${run.purpose}\n\nObserved local command execution, not instructor evidence.\n\n- Started: ${run.startedAt}\n- Working directory: ${run.cwd}\n- Status: ${run.status}\n- Exit code: ${run.exitCode ?? "none"}\n- Signal: ${run.signal || "none"}\n- Assignment: ${run.assignmentId || "Shared course files"}\n\n## Command\n\n${run.command}\n\n## Output (last 12000 characters${run.truncated ? "; log reached 2 MB limit" : ""})\n\n${output.slice(-12000)}\n\n${run.error || ""}\n\nA successful exit only verifies what this command actually checked. Record reasoning and remaining gaps separately.\n`;
  try {
    fs.mkdirSync(path.dirname(request.summaryFile), { recursive: true });
    fs.writeFileSync(request.summaryFile, summary, { mode: 0o600 });
  } catch (e) {
    run.error = `${run.error || ""} Could not save command memory: ${e.message}`;
  }
  persist();
}
child.on("error", (error) => finish(null, null, error));
child.on("close", (code, signal) => finish(code, signal));
