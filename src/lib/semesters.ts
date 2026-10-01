import type { Course, Semester, State } from "./schema";

export const defaultSemester: Semester = { season: "Fall", year: 2026 };
export const semesterName = (semester: Semester) =>
  `${semester.season} ${semester.year}`;

function parseSemester(term: string): Semester | undefined {
  const match = /^(Spring|Summer|Fall)\s+(\d{4})$/i.exec(term.trim());
  if (!match || Number(match[2]) < 1900) return;
  return {
    season: (match[1][0].toUpperCase() +
      match[1].slice(1).toLowerCase()) as Semester["season"],
    year: Number(match[2]),
  };
}

export const courseSemester = (course: Course) =>
  semesterName(parseSemester(course.term) || defaultSemester);

export function listSemesters(
  state: Pick<State, "semesters" | "courses">,
): Semester[] {
  const semesters = [
    defaultSemester,
    ...(state.semesters || []),
    ...state.courses.map(
      (course) => parseSemester(course.term) || defaultSemester,
    ),
  ];
  return [
    ...new Map(semesters.map((item) => [semesterName(item), item])).values(),
  ].sort(
    (a, b) =>
      b.year - a.year ||
      ["Spring", "Summer", "Fall"].indexOf(b.season) -
        ["Spring", "Summer", "Fall"].indexOf(a.season),
  );
}
