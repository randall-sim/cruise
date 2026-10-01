import { test } from "node:test";
import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import path from "node:path";
import {
  addCourse,
  addSemester,
  readPrivate,
  readState,
} from "../src/lib/store";
import {
  courseSemester,
  listSemesters,
  semesterName,
} from "../src/lib/semesters";

test("semester workspaces preserve legacy data, persist empty semesters, and reject invalid or duplicate creation", async () => {
  const previousRoot = process.env.COURSE_CAPTAIN_WORKSPACE;
  const root = await fs.mkdtemp(path.resolve("workspace-test-semesters-"));
  process.env.COURSE_CAPTAIN_WORKSPACE = root;
  try {
    const fall = await addCourse({
      code: "FALL",
      name: "Existing course",
      term: "Fall 2026",
    });
    const legacy = await addCourse({ code: "LEGACY", name: "Legacy course" });
    const spring = await addCourse({
      code: "SPRING",
      name: "Spring course",
      term: "spring 2027",
    });
    const before = await readPrivate("state.json");
    const courseBefore = await readPrivate(`courses/${fall.id}/course.json`);
    assert.deepEqual(listSemesters(await readState()).map(semesterName), [
      "Spring 2027",
      "Fall 2026",
    ]);
    assert.equal(courseSemester(legacy), "Fall 2026");
    assert.equal(courseSemester(spring), "Spring 2027");
    assert.equal(await readPrivate("state.json"), before);
    await assert.rejects(
      addSemester({ season: "Fall", year: 2026 }),
      /already exists/,
    );
    await assert.rejects(addSemester({ season: "Winter", year: 2027 }));
    await assert.rejects(addSemester({ season: "Spring", year: 2027.5 }));
    await assert.rejects(addSemester({ season: "Spring", year: 1899 }));
    const results = await Promise.allSettled([
      addSemester({ season: "Summer", year: 2027 }),
      addSemester({ season: "Summer", year: 2027 }),
    ]);
    assert.equal(
      results.filter((result) => result.status === "fulfilled").length,
      1,
    );
    assert.deepEqual(listSemesters(await readState()).map(semesterName), [
      "Summer 2027",
      "Spring 2027",
      "Fall 2026",
    ]);
    assert.deepEqual((await readState()).courses, JSON.parse(before).courses);
    assert.equal(
      await readPrivate(`courses/${fall.id}/course.json`),
      courseBefore,
    );
  } finally {
    if (previousRoot) process.env.COURSE_CAPTAIN_WORKSPACE = previousRoot;
    else delete process.env.COURSE_CAPTAIN_WORKSPACE;
    await fs.rm(root, { recursive: true, force: true });
  }
});
