import type { AssignmentTree } from "./assignment-schema";

// Browser-session cache. File bodies are fetched separately when selected.
const trees = new Map<string, { tree: AssignmentTree; savedAt: number }>();
const requests = new Map<string, Promise<AssignmentTree>>();
const probes = new Map<
  string,
  Promise<{ hasEntries: boolean; firstPath?: string }>
>();
const generations = new Map<string, number>();
export const TREE_CACHE_TTL = 30_000;
export const treeKey = (
  courseId: string,
  assignmentId?: string,
  directory = "",
) => `${courseId}:${assignmentId || "files"}:${directory}`;
export function cachedTree(
  courseId: string,
  assignmentId?: string,
  directory = "",
) {
  return trees.get(treeKey(courseId, assignmentId, directory));
}
export async function probeTree(courseId: string, assignmentId?: string) {
  const key = treeKey(courseId, assignmentId);
  const pending = probes.get(key);
  if (pending) return pending;
  const endpoint = assignmentId ? "/api/assignments" : "/api/course-files";
  const request = (async () => {
    const response = await fetch(
      `${endpoint}?${new URLSearchParams({ courseId, ...(assignmentId ? { assignmentId } : {}), mode: "probe" })}`,
      { cache: "no-store" },
    );
    const data = await response.json();
    if (!response.ok) throw new Error(data.error || "Could not check files");
    return data as { hasEntries: boolean; firstPath?: string };
  })();
  probes.set(key, request);
  try {
    return await request;
  } finally {
    if (probes.get(key) === request) probes.delete(key);
  }
}
export function invalidateCourseTrees(courseId: string) {
  for (const key of new Set([...trees.keys(), ...requests.keys()])) {
    if (!key.startsWith(courseId + ":")) continue;
    trees.delete(key);
    requests.delete(key);
    generations.set(key, (generations.get(key) || 0) + 1);
  }
}
export async function fetchTree(
  courseId: string,
  assignmentId?: string,
  force = false,
  directory = "",
) {
  const key = treeKey(courseId, assignmentId, directory);
  const cached = trees.get(key);
  if (!force && cached && Date.now() - cached.savedAt < TREE_CACHE_TTL)
    return cached.tree;
  const pending = requests.get(key);
  if (pending) return pending;
  const generation = generations.get(key) || 0;
  const endpoint = assignmentId ? "/api/assignments" : "/api/course-files";
  const request = (async () => {
    const response = await fetch(
      `${endpoint}?${new URLSearchParams({ courseId, ...(assignmentId ? { assignmentId } : {}), mode: "directory", directory })}`,
      { cache: "no-store" },
    );
    const tree = await response.json();
    if (!response.ok) throw new Error(tree.error || "Could not load files");
    if ((generations.get(key) || 0) === generation) {
      trees.set(key, { tree, savedAt: Date.now() });
      // Bound memory when browsing many courses/assignments.
      if (trees.size > 50) trees.delete(trees.keys().next().value!);
    }
    return tree as AssignmentTree;
  })();
  requests.set(key, request);
  try {
    return await request;
  } finally {
    if (requests.get(key) === request) requests.delete(key);
  }
}
