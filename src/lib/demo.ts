import { addCourse, readState } from "./store";
import { importLecture } from "./lectures";
import { completeJob, createJob } from "./jobs";

export async function seedDemo() {
  if ((await readState()).courses.length)
    throw new Error("Demo requires an empty workspace");
  const algorithms = await addCourse(
    {
      code: "CS 240",
      name: "Data Structures & Algorithms",
      term: "Example semester",
      color: "green",
    },
    true,
  );
  const math = await addCourse(
    {
      code: "MATH 221",
      name: "Linear Algebra",
      term: "Example semester",
      color: "purple",
    },
    true,
  );
  const systems = await addCourse(
    {
      code: "CS 310",
      name: "Computer Systems",
      term: "Example semester",
      color: "orange",
    },
    true,
  );
  const date = new Date().toISOString().slice(0, 10);
  const lecture = await importLecture({
    courseId: algorithms.id,
    title: "Thinking recursively",
    date,
    transcript: `WEBVTT

00:00:00.000 --> 00:02:00.000
This is a short fictional example lecture. A recursive algorithm solves a problem using smaller instances of the same problem. A base case stops the recursion. Each recursive step must make progress toward that base case.

00:02:00.000 --> 00:04:30.000
Consider merge sort. Split an array into two halves, recursively sort each half, then merge the sorted halves. An array of length zero or one is already sorted, so this is the base case. The merge operation takes linear time in the total number of elements.

00:04:30.000 --> 00:07:00.000
The recurrence is T(n) = 2T(n/2) + O(n). There are logarithmically many levels in the recursion tree, and each level does O(n) work. The total running time is O(n log n). The standard array implementation also uses O(n) auxiliary space.

00:07:00.000 --> 00:09:00.000
Compare this with binary search. Binary search recursively examines only one half, and does constant work at each level. Its recurrence is T(n) = T(n/2) + O(1), giving O(log n) time. Binary search requires the search interval to be sorted.

00:09:00.000 --> 00:10:30.000
A useful correctness argument is induction on input size. First prove the base case. Next assume the recursive calls correctly handle smaller inputs, then show the combination step yields the correct result for the whole input. Termination additionally requires that the input size decreases.

00:10:30.000 --> 00:12:00.000
For this fictional course, the practice sheet asks you to draw a recursion tree and explain the work per level. This is sample course logistics, not a real deadline. Check both the base case and the shrinking input before analyzing a recurrence.
`,
    captureCoverage:
      "Fictional transcript for exploring the app. No real lecture video or screenshots are attached.",
  });
  const job = await createJob({
    kind: "lecture",
    courseId: algorithms.id,
    lectureId: lecture.id,
    prompt: "Explain this fictional example lecture.",
  });
  const cite = (i: number) => `${lecture.id}:t${i}`;
  await completeJob(job.id, {
    summary:
      "Recursion becomes easier to reason about when you separate three questions: does it stop, is it correct, and how much work does it do? This example follows merge sort and binary search to answer each one.",
    sections: [
      {
        title: "The shape of a recursive solution",
        markdown:
          "A recursive algorithm reduces a problem to smaller instances of itself. Two ingredients make that safe:\n\n1. **A base case** that can be answered directly.\n2. **Progress** toward that base case on every recursive call.\n\nThink of recursion as a contract: the smaller call gives you a correct answer, and your job is to combine it into a correct answer to the current problem.",
        citations: [cite(0), cite(4)],
      },
      {
        title: "Merge sort: split, solve, combine",
        markdown:
          "Merge sort splits the array into two halves, sorts both recursively, then merges the results. The base case is an array with at most one element.\n\nThe recurrence **T(n) = 2T(n/2) + O(n)** records two recursive calls and a linear merge. Each level of the recursion tree performs O(n) total work. With O(log n) levels, the running time is **O(n log n)**. The standard array implementation needs **O(n) auxiliary space**.",
        citations: [cite(1), cite(2)],
      },
      {
        title: "Why binary search is different",
        markdown:
          "Binary search explores just **one half** of a sorted interval, with constant work per level. Its recurrence is T(n) = T(n/2) + O(1), giving O(log n) time.\n\n| Algorithm | Recursive calls | Work per level | Total time |\n| --- | --- | --- | --- |\n| Merge sort | Both halves | O(n) | O(n log n) |\n| Binary search | One half | O(1) | O(log n) |\n\nHalving the input alone does not imply logarithmic runtime. Count the branches and the work done across a whole level.",
        citations: [cite(2), cite(3)],
      },
      {
        title: "A proof you can reuse",
        markdown:
          "To prove a recursive algorithm correct, use induction on input size: establish the base case, assume smaller calls are correct, and prove the combination step. Prove termination separately by showing the input gets smaller.",
        citations: [cite(4)],
      },
    ],
    logistics: [
      {
        text: "Example practice: draw a recursion tree and explain the work per level. This is demonstration content, not a real assignment.",
        citations: [cite(5)],
      },
    ],
    concepts: [
      {
        term: "Base case",
        definition:
          "An instance that can be solved directly without making another recursive call.",
        citations: [cite(0)],
      },
      {
        term: "Recurrence relation",
        definition:
          "An expression for an algorithm’s cost in terms of the cost of smaller instances and the work done at the current step.",
        citations: [cite(2), cite(3)],
      },
      {
        term: "Inductive hypothesis",
        definition:
          "The assumption that recursive calls correctly solve smaller inputs, used to prove the current step correct.",
        citations: [cite(4)],
      },
    ],
    questions: [
      {
        question:
          "Both merge sort and binary search halve the input. Why do their runtimes differ?",
        answer:
          "Merge sort explores both halves and performs linear merging work per level. Binary search follows one half and does constant work per level.",
        citations: [cite(2), cite(3)],
      },
      {
        question:
          "What two things do you need to show a recursive algorithm terminates?",
        answer:
          "There is a reachable base case, and every recursive step makes progress toward it (for example, by reducing the input size).",
        citations: [cite(0), cite(4)],
      },
    ],
    gaps: [],
  });
  return algorithms;
}
