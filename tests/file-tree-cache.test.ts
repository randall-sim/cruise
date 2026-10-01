import { test } from "node:test";
import assert from "node:assert/strict";
import {
  fetchTree,
  cachedTree,
  invalidateCourseTrees,
} from "../src/lib/file-tree-cache";

test("tree cache deduplicates loads, isolates assignment views, and invalidates every course view", async () => {
  const original = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = async () => {
    calls++;
    return new Response(
      JSON.stringify({ entries: [], revision: String(calls) }),
    );
  };
  try {
    const [one, two] = await Promise.all([
      fetchTree("cache-course"),
      fetchTree("cache-course"),
    ]);
    assert.equal(one, two);
    await fetchTree("cache-course");
    assert.equal(calls, 1);
    await fetchTree("cache-course", "assignment");
    assert.equal(calls, 2);
    invalidateCourseTrees("cache-course");
    assert.equal(cachedTree("cache-course"), undefined);
    assert.equal(cachedTree("cache-course", "assignment"), undefined);
    await fetchTree("cache-course");
    await fetchTree("cache-course", undefined, true);
    assert.equal(calls, 4);
  } finally {
    globalThis.fetch = original;
    invalidateCourseTrees("cache-course");
  }
});

test("an invalidated in-flight response cannot repopulate the tree cache", async () => {
  const original = globalThis.fetch;
  let finish!: (response: Response) => void;
  globalThis.fetch = () =>
    new Promise((resolve) => {
      finish = resolve;
    });
  try {
    const pending = fetchTree("stale-course");
    invalidateCourseTrees("stale-course");
    finish(new Response(JSON.stringify({ entries: [], revision: "old" })));
    await pending;
    assert.equal(cachedTree("stale-course"), undefined);
  } finally {
    globalThis.fetch = original;
    invalidateCourseTrees("stale-course");
  }
});

test("folder cache keys isolate root and nested listings and invalidate them together", async () => {
  const original = globalThis.fetch;
  const urls: string[] = [];
  globalThis.fetch = async (url) => {
    urls.push(String(url));
    return new Response(JSON.stringify({ entries: [], revision: String(url) }));
  };
  try {
    const root = await fetchTree("folders");
    const child = await fetchTree("folders", undefined, false, "project/src");
    assert.notEqual(root.revision, child.revision);
    assert.equal(
      new URL(urls[1], "http://localhost").searchParams.get("directory"),
      "project/src",
    );
    assert.equal(
      new URL(urls[0], "http://localhost").searchParams.get("mode"),
      "directory",
    );
    await fetchTree("folders", undefined, false, "project/src");
    assert.equal(urls.length, 2);
    invalidateCourseTrees("folders");
    assert.equal(cachedTree("folders", undefined, "project/src"), undefined);
  } finally {
    globalThis.fetch = original;
    invalidateCourseTrees("folders");
  }
});
