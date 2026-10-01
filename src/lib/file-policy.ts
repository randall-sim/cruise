// Keep dependency trees, VCS internals, generated output and credentials out of
// the browser reader and course retrieval. Commands can still use them on disk.
const excluded = new Set([
  ".git",
  ".hg",
  ".svn",
  "node_modules",
  ".venv",
  "venv",
  "__pycache__",
  ".next",
  "dist",
  "build",
  "target",
  "coverage",
  ".cache",
  ".runtime",
  ".secrets",
  ".ssh",
  ".npmrc",
  ".pypirc",
  ".netrc",
  ".git-credentials",
  "id_rsa",
  "id_ed25519",
]);
export function excludedCoursePath(relative: string) {
  return relative
    .split("/")
    .some(
      (part) =>
        excluded.has(part) ||
        /^\.env(?:\.|$)/.test(part) ||
        /^(?:auth|storage-state)\.json$|\.(?:pem|key|p12|pfx)$/i.test(part),
    );
}
